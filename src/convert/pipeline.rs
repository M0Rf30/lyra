// SPDX-License-Identifier: GPL-3.0

//! Standalone decode → resample → encode pipeline for local file
//! conversion, transcoding, and CUE-sheet splitting.
//!
//! Decoding uses the same probe/track-selection pattern as
//! [`crate::player::engine::decoder`], but against a plain `File` /
//! `MediaSourceStream` — no DSD, no HTTP streaming, since this pipeline
//! never touches playback. Because symphonia's probe is content-based, this
//! transparently "rips" the first audio track out of video containers
//! (mp4/mkv) too, with no container-specific special-casing.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::player::engine::resampler::{ResamplerQuality, StreamResampler};

use super::cue;
use super::encoder;
use super::{ConvertError, ConvertJob, JobKind};

/// Runs `job` to completion: decodes, optionally resamples, encodes, and
/// tags the output(s). Checks `job.cancel` between packets and deletes any
/// partial output on cancellation.
pub fn run(job: &ConvertJob) -> Result<(), ConvertError> {
    std::fs::create_dir_all(&job.out_dir)?;
    match job.kind {
        JobKind::Convert => {
            let stem = job.source.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
            let out_path = unique_out_path(&job.out_dir, stem, job.format.extension());
            transcode(job, &job.source, &out_path, None, None)?;
            copy_tags(&job.source, &out_path);
            Ok(())
        }
        JobKind::CueSplit => cue_split(job),
    }
}

/// Splits the audio file referenced by a `.cue` sheet (`job.source`) into
/// one tagged output file per track.
fn cue_split(job: &ConvertJob) -> Result<(), ConvertError> {
    let cue_text = std::fs::read_to_string(&job.source)?;
    let tracks = cue::parse(&cue_text).map_err(|e| ConvertError::Cue(e.to_string()))?;
    let file_name = cue::parse_file_name(&cue_text)
        .ok_or_else(|| ConvertError::Cue("no FILE line in CUE sheet".to_owned()))?;
    let audio_path = job
        .source
        .parent()
        .map(|dir| dir.join(&file_name))
        .unwrap_or_else(|| PathBuf::from(&file_name));

    for track in &tracks {
        if job.cancel.load(Ordering::Relaxed) {
            return Err(ConvertError::Cancelled);
        }

        let stem = format!("{:02} - {}", track.number, sanitize_filename(&track.title));
        let out_path = unique_out_path(&job.out_dir, &stem, job.format.extension());
        let start = track.start.as_secs_f64();
        let end = track.end.map(|d| d.as_secs_f64());
        transcode(job, &audio_path, &out_path, Some(start), end)?;

        let mut tag = Tag::new(detect_tag_type(&out_path));
        tag.set_title(track.title.clone());
        tag.set_artist(track.performer.clone());
        tag.set_track(track.number);
        write_tag(&out_path, tag);
    }
    Ok(())
}

/// Decodes `[start, end)` seconds of `source_path` (the whole file when
/// both are `None`) and encodes it to `out_path` per `job.format` /
/// `job.target_rate`.
fn transcode(
    job: &ConvertJob,
    source_path: &Path,
    out_path: &Path,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(), ConvertError> {
    let mut source = AudioSource::open(source_path)?;
    if let Some(start) = start.filter(|&s| s > 0.0) {
        source.seek(start)?;
    }

    let src_rate = source.sample_rate;
    let channels = source.channels;
    let dst_rate = job.target_rate.unwrap_or(src_rate);
    let mut resampler = (dst_rate != src_rate)
        .then(|| StreamResampler::new(src_rate, dst_rate, channels as usize, ResamplerQuality::SincMedium))
        .flatten();

    let mut sink = encoder::create_sink(
        job.format,
        out_path,
        channels,
        dst_rate,
        source.bits_per_sample,
    )?;

    // Frame budget for the `[start, end)` window, in source-domain frames.
    let max_frames = end.map(|e| ((e - start.unwrap_or(0.0)).max(0.0) * f64::from(src_rate)).round() as u64);

    const CHUNK_FRAMES: usize = 8192;
    let mut buf = vec![0f32; CHUNK_FRAMES * channels.max(1) as usize];
    let mut frames_done: u64 = 0;

    loop {
        if job.cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(out_path);
            return Err(ConvertError::Cancelled);
        }

        let mut want = buf.len();
        if let Some(max) = max_frames {
            let remaining = max.saturating_sub(frames_done) as usize * channels.max(1) as usize;
            if remaining == 0 {
                break;
            }
            want = want.min(remaining);
        }

        let n = source.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        frames_done += (n / channels.max(1) as usize) as u64;

        let chunk = &buf[..n];
        match resampler.as_mut() {
            Some(rs) => sink.write(&rs.process(chunk))?,
            None => sink.write(chunk)?,
        }

        job.progress.store(source.progress_permille(frames_done), Ordering::Relaxed);
    }

    sink.finish()?;
    job.progress.store(1000, Ordering::Relaxed);
    Ok(())
}

/// Copies title/artist/album/genre/track/disk tags from `src` to `dst`,
/// best-effort — a missing source tag or an unwritable field is skipped,
/// never a hard error, since the encoded output is already valid without it.
fn copy_tags(src: &Path, dst: &Path) {
    let Some(src_tag) = Probe::open(src)
        .ok()
        .and_then(|p| p.read().ok())
        .and_then(|f| f.primary_tag().or_else(|| f.first_tag()).cloned())
    else {
        return;
    };

    let tag_type = detect_tag_type(dst);
    let mut tag = Tag::new(tag_type);

    if let Some(v) = src_tag.title() {
        tag.set_title(v.into_owned());
    }
    if let Some(v) = src_tag.artist() {
        tag.set_artist(v.into_owned());
    }
    if let Some(v) = src_tag.album() {
        tag.set_album(v.into_owned());
    }
    if let Some(v) = src_tag.genre() {
        tag.set_genre(v.into_owned());
    }
    if let Some(v) = src_tag.track() {
        tag.set_track(v);
    }
    if let Some(v) = src_tag.disk() {
        tag.set_disk(v);
    }

    write_tag(dst, tag);
}

/// Probes `path` for the tag type its container actually supports, falling
/// back to Vorbis comments (used by FLAC, our most common output) if the
/// probe fails.
fn detect_tag_type(path: &Path) -> lofty::tag::TagType {
    Probe::open(path)
        .ok()
        .and_then(|p| p.read().ok())
        .map(|f| f.primary_tag_type())
        .unwrap_or(lofty::tag::TagType::VorbisComments)
}

/// Inserts `tag` into `path`'s tagged file and saves, ignoring failures —
/// tagging is best-effort per the caller's contract.
fn write_tag(path: &Path, tag: Tag) {
    if let Ok(mut tagged) = Probe::open(path).and_then(|p| p.read()) {
        tagged.insert_tag(tag);
        let _ = tagged.save_to_path(path, WriteOptions::default());
    }
}

/// Strips characters that are awkward or invalid in filenames.
fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let cleaned: String = trimmed
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    if cleaned.is_empty() { "track".to_owned() } else { cleaned }
}

/// Builds `dir/stem.ext`, appending ` (N)` before the extension if that
/// path already exists, so repeated conversions never clobber each other.
fn unique_out_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return candidate;
    }
    for n in 1..1000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}.{ext}"))
}

/// Wraps a `File` so decode progress can be tracked by bytes consumed, for
/// formats/sources where symphonia can't report `num_frames` up front.
struct CountingFile {
    inner: File,
    byte_len: Option<u64>,
    read_bytes: Arc<AtomicU64>,
}

impl Read for CountingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read_bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for CountingFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for CountingFile {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}

/// A single open, probed audio track ready for streaming PCM reads.
struct AudioSource {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    /// Bits per decoded sample, if the codec reports one (only consulted
    /// for picking a FLAC output bit depth).
    bits_per_sample: Option<u32>,
    /// Total frames if known from container metadata; falls back to
    /// `file_len`/`read_bytes` for progress when unknown.
    total_frames: Option<u64>,
    file_len: Option<u64>,
    read_bytes: Arc<AtomicU64>,
    sample_buf: Vec<f32>,
    sample_pos: usize,
}

impl AudioSource {
    fn open(path: &Path) -> Result<Self, ConvertError> {
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let file = File::open(path)?;
        let file_len = file.metadata().ok().map(|m| m.len());
        let read_bytes = Arc::new(AtomicU64::new(0));
        let counting = CountingFile {
            inner: file,
            byte_len: file_len,
            read_bytes: Arc::clone(&read_bytes),
        };
        let mss = MediaSourceStream::new(Box::new(counting), Default::default());

        let reader = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| ConvertError::Decode(format!("probe failed: {e}")))?;

        let track = reader.default_track(TrackType::Audio).ok_or(ConvertError::NoAudioTrack)?;
        let track_id = track.id;

        let audio = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(audio)) => audio.clone(),
            _ => return Err(ConvertError::NoAudioTrack),
        };
        let sample_rate = audio
            .sample_rate
            .ok_or_else(|| ConvertError::Decode("source has no sample rate".to_owned()))?;
        let channels = audio.channels.as_ref().map_or(2, |c| c.count() as u16);
        let bits_per_sample = audio.bits_per_sample;
        let total_frames = track.num_frames;

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&audio, &AudioDecoderOptions::default())
            .map_err(|e| ConvertError::Decode(format!("no decoder available: {e}")))?;

        Ok(Self {
            reader,
            decoder,
            track_id,
            sample_rate,
            channels,
            bits_per_sample,
            total_frames,
            file_len,
            read_bytes,
            sample_buf: Vec::new(),
            sample_pos: 0,
        })
    }

    /// Seeks near `secs` seconds from the start (used for CUE track
    /// boundaries).
    fn seek(&mut self, secs: f64) -> Result<(), ConvertError> {
        let time = Time::try_from_secs_f64(secs.max(0.0))
            .ok_or_else(|| ConvertError::Decode("invalid seek position".to_owned()))?;
        self.reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time { time, track_id: Some(self.track_id) },
            )
            .map_err(|e| ConvertError::Decode(format!("seek failed: {e}")))?;
        self.decoder.reset();
        self.sample_buf.clear();
        self.sample_pos = 0;
        Ok(())
    }

    /// Reads decoded, interleaved `f32` PCM into `buffer`, returning how
    /// many samples were written (0 only at end-of-stream).
    fn read(&mut self, buffer: &mut [f32]) -> Result<usize, ConvertError> {
        let mut written = 0;

        while written < buffer.len() {
            if self.sample_pos < self.sample_buf.len() {
                let available = self.sample_buf.len() - self.sample_pos;
                let to_copy = (buffer.len() - written).min(available);
                buffer[written..written + to_copy]
                    .copy_from_slice(&self.sample_buf[self.sample_pos..self.sample_pos + to_copy]);
                written += to_copy;
                self.sample_pos += to_copy;
                if written >= buffer.len() {
                    break;
                }
            }

            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(SymphoniaError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(ConvertError::Decode(format!("failed to read packet: {e}"))),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(ConvertError::Decode(format!("failed to decode packet: {e}"))),
            };

            if decoded.frames() == 0 {
                continue;
            }

            decoded.copy_to_vec_interleaved(&mut self.sample_buf);
            self.sample_pos = 0;
        }

        Ok(written)
    }

    /// Progress in permille (0-1000), from decoded source frames when the
    /// total is known, else from bytes consumed out of the source file.
    fn progress_permille(&self, frames_done: u64) -> u32 {
        if let Some(total) = self.total_frames.filter(|&t| t > 0) {
            ((frames_done.min(total) * 1000) / total) as u32
        } else if let Some(total) = self.file_len.filter(|&t| t > 0) {
            let done = self.read_bytes.load(Ordering::Relaxed);
            ((done.min(total) * 1000) / total) as u32
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Writes a 1-second 44.1kHz mono sine WAV, tagged with a title, so the
    /// end-to-end job (decode → encode → tag copy) can be exercised without
    /// needing a fixture file on disk.
    fn write_test_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..44_100u32 {
            let s = (TAU * 440.0 * i as f32 / 44_100.0).sin();
            writer.write_sample((s * f32::from(i16::MAX)) as i16).unwrap();
        }
        writer.finalize().unwrap();

        let mut tagged = Probe::open(path).and_then(|p| p.read()).unwrap();
        let mut tag = Tag::new(tagged.primary_tag_type());
        tag.set_title("Pipeline Test Track".to_owned());
        tagged.insert_tag(tag);
        tagged.save_to_path(path, WriteOptions::default()).unwrap();
    }

    #[test]
    fn run_converts_and_copies_tags_end_to_end() {
        let dir = std::env::temp_dir().join(format!("lyra-pipeline-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        write_test_wav(&source);

        let out_dir = dir.join("out");
        let job = ConvertJob::new(
            1,
            source,
            JobKind::Convert,
            encoder::OutputFormat::Flac,
            None,
            out_dir.clone(),
        );

        run(&job).expect("conversion job should succeed");

        let out_path = out_dir.join("source.flac");
        assert!(out_path.exists(), "expected {out_path:?} to exist");

        let tagged = Probe::open(&out_path).unwrap().read().unwrap();
        let duration = tagged.properties().duration();
        assert!(
            (duration.as_secs_f64() - 1.0).abs() < 0.05,
            "expected ~1s, got {duration:?}"
        );
        let tag = tagged.primary_tag().expect("output should have a tag");
        assert_eq!(tag.title().map(|c| c.into_owned()), Some("Pipeline Test Track".to_owned()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
