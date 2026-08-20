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
use lofty::picture::{Picture, PictureType};
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
use super::{ConvertError, ConvertJob, JobId, JobKind};

/// Runs `job` to completion: decodes, optionally resamples, encodes, and
/// tags the output(s). Checks `job.cancel` between packets. Encoding
/// happens on a scratch file next to the real output; on cancellation, a
/// decode error, or an encode error the scratch file is deleted and the
/// real output path is never touched, so nothing partial is ever left
/// behind at the name the caller asked for.
pub fn run(job: &ConvertJob) -> Result<(), ConvertError> {
    std::fs::create_dir_all(&job.out_dir)?;
    match job.kind {
        JobKind::Convert => {
            let stem = job.source.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
            let out_path = unique_out_path(&job.out_dir, stem, job.format.extension());
            transcode(job, &job.source, &out_path, None, None, 0, 1000)?;
            copy_tags(&job.source, &out_path);
            Ok(())
        }
        JobKind::CueSplit => cue_split(job),
    }
}

/// Splits the audio file referenced by a `.cue` sheet (`job.source`) into
/// one tagged output file per track. Each track's slice of the overall
/// `job.progress` permille range is fixed up front by
/// [`track_progress_range`] (equal-weight per track), so progress climbs
/// monotonically across the whole rip instead of resetting to ~0 at every
/// track boundary — `transcode` alone can't know it's one of several calls
/// sharing a single job's progress bar.
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

    // Read the whole album's tag once so every track can carry over the
    // fields the CUE sheet itself doesn't encode (album, genre, date, cover
    // art), rather than each track ending up untagged beyond its title.
    let src_tag = Probe::open(&audio_path)
        .ok()
        .and_then(|p| p.read().ok())
        .and_then(|f| f.primary_tag().or_else(|| f.first_tag()).cloned());

    let track_count = tracks.len();
    for (i, track) in tracks.iter().enumerate() {
        if job.cancel.load(Ordering::Relaxed) {
            return Err(ConvertError::Cancelled);
        }

        let stem = format!("{:02} - {}", track.number, sanitize_filename(&track.title));
        let out_path = unique_out_path(&job.out_dir, &stem, job.format.extension());
        let start = track.start.as_secs_f64();
        let end = track.end.map(|d| d.as_secs_f64());
        let (progress_base, progress_span) = track_progress_range(i, track_count);
        transcode(job, &audio_path, &out_path, Some(start), end, progress_base, progress_span)?;

        let mut tag = Tag::new(detect_tag_type(&out_path));
        tag.set_title(track.title.clone());
        tag.set_artist(track.performer.clone());
        tag.set_track(track.number);
        tag.set_track_total(track_count as u32);
        if let Some(src_tag) = &src_tag {
            copy_shared_tag_fields(src_tag, &mut tag);
        }
        write_tag(&out_path, tag);
    }
    Ok(())
}

/// Computes the `(progress_base, progress_span)` permille slice that CUE
/// track `index` (0-based) of `total` tracks owns within the job's overall
/// progress bar. Allocates equal weight per track rather than by duration —
/// exact duration-weighting would need a separate full-file probe pass,
/// which isn't worth the complexity here. Integer division keeps the spans
/// exact: they sum to 1000 with no drift, for any `total >= 1`.
fn track_progress_range(index: usize, total: usize) -> (u32, u32) {
    let total = total.max(1) as u32;
    let index = index as u32;
    let base = 1000 * index / total;
    let next_base = 1000 * (index + 1) / total;
    (base, next_base - base)
}

/// Decodes `[start, end)` seconds of `source_path` (the whole file when
/// both are `None`) and encodes it to `out_path` per `job.format` /
/// `job.target_rate`. `progress_base`/`progress_span` place this call's
/// own 0-1000 permille progress within a larger slice of `job.progress` —
/// `[progress_base, progress_base + progress_span]`, i.e. `(0, 1000)` for
/// a plain whole-file conversion, or a per-track slice when `cue_split`
/// calls this once per track and needs the job's progress to climb
/// monotonically across all of them instead of restarting at each track.
///
/// Encodes into a same-directory scratch file and installs it with an
/// atomic rename only once it's fully written — see [`TempFileGuard`].
fn transcode(
    job: &ConvertJob,
    source_path: &Path,
    out_path: &Path,
    start: Option<f64>,
    end: Option<f64>,
    progress_base: u32,
    progress_span: u32,
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

    let tmp_path = temp_out_path(out_path, job.id);
    let mut tmp_guard = TempFileGuard::new(tmp_path.clone());
    let mut sink = encoder::create_sink(
        job.format,
        &tmp_path,
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

        job.progress.store(
            progress_base + source.progress_permille(frames_done) * progress_span / 1000,
            Ordering::Relaxed,
        );
    }

    // `StreamResampler::process` only emits output once a full internal
    // chunk of input has accumulated, so it's always holding back a
    // fractional chunk plus its filter's group delay. Without draining
    // that here, every resampled conversion loses its true tail — for a
    // clip shorter than one resampler chunk, the entire output is silently
    // empty.
    if let Some(rs) = resampler.as_mut() {
        let tail = rs.flush();
        if !tail.is_empty() {
            sink.write(&tail)?;
        }
    }

    sink.finish()?;
    std::fs::rename(&tmp_path, out_path)?;
    tmp_guard.disarm();
    job.progress.store(progress_base + progress_span, Ordering::Relaxed);
    Ok(())
}

/// Same-directory scratch path for `out_path`'s encode, suffixed with
/// `job_id` so two concurrently-running jobs never collide on the same
/// temp file.
fn temp_out_path(out_path: &Path, job_id: JobId) -> PathBuf {
    let file_name = out_path.file_name().and_then(|f| f.to_str()).unwrap_or("output");
    out_path.with_file_name(format!(".{file_name}.{job_id}.part"))
}

/// Deletes its file on drop unless [`disarm`](Self::disarm) was called
/// first. `transcode` disarms it only after `sink.finish()` and the
/// rename into the real output path both succeed, so every early return
/// (cancellation, a decode error, an encode error) deletes the scratch
/// file instead of leaving a partial result on disk.
struct TempFileGuard {
    path: PathBuf,
    keep: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn disarm(&mut self) {
        self.keep = true;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Copies title/artist/track/disk plus the shared album/genre/date/cover
/// art fields (see [`copy_shared_tag_fields`]) from `src` to `dst`,
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
    if let Some(v) = src_tag.track() {
        tag.set_track(v);
    }
    if let Some(v) = src_tag.disk() {
        tag.set_disk(v);
    }
    copy_shared_tag_fields(&src_tag, &mut tag);

    write_tag(dst, tag);
}

/// Copies the release-level fields both `copy_tags` (whole-file convert)
/// and `cue_split` (per-track rip) need from the same source tag — album,
/// genre, release date, and front-cover artwork — but neither title,
/// artist, nor track number, since those differ per output (CUE tracks get
/// their own title/artist/number from the cue sheet, not the source tag).
/// Best-effort: a missing field is skipped, never an error.
fn copy_shared_tag_fields(src_tag: &Tag, dst_tag: &mut Tag) {
    if let Some(v) = src_tag.album() {
        dst_tag.set_album(v.into_owned());
    }
    if let Some(v) = src_tag.genre() {
        dst_tag.set_genre(v.into_owned());
    }
    if let Some(v) = src_tag.date() {
        dst_tag.set_date(v);
    }
    if let Some(picture) = front_cover(src_tag) {
        dst_tag.push_picture(picture.clone());
    }
}

/// Returns `tag`'s front-cover picture, falling back to the first embedded
/// picture if none is explicitly typed as the front cover — most rips only
/// embed a single (untyped-as-front) picture, and that's still the one
/// users expect to see as artwork.
fn front_cover(tag: &Tag) -> Option<&Picture> {
    let pictures = tag.pictures();
    pictures.iter().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pictures.first())
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

/// Strips characters that are awkward, invalid, or path-traversing in
/// filenames (path separators, NUL, other control characters); strips a
/// leading `-` that could be misread as a flag by anything that later
/// shells out on the name; trims surrounding `.`/whitespace so a name
/// that's entirely dots can't resolve to `.`/`..` as a path component; and
/// caps the result's byte length so a pathological tag can't exceed
/// common filesystem name limits.
fn sanitize_filename(name: &str) -> String {
    const MAX_BYTES: usize = 150;

    let mut cleaned = String::new();
    for c in name.trim().chars() {
        if c == '\0' || c.is_control() {
            continue;
        }
        let c = if "/\\:*?\"<>|".contains(c) { '_' } else { c };
        if cleaned.len() + c.len_utf8() > MAX_BYTES {
            break;
        }
        cleaned.push(c);
    }

    while cleaned.starts_with('-') {
        cleaned.remove(0);
    }
    let cleaned = cleaned.trim_matches('.').trim();

    if cleaned.is_empty() { "track".to_owned() } else { cleaned.to_owned() }
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

    /// Writes a mono 44.1kHz sine WAV of `num_frames` samples, tagged with a
    /// title, so an end-to-end job (decode → encode → tag copy) can be
    /// exercised without needing a fixture file on disk.
    fn write_test_wav_frames(path: &Path, num_frames: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..num_frames {
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

    /// One second of `write_test_wav_frames`, used by tests that don't care
    /// about the exact clip length.
    fn write_test_wav(path: &Path) {
        write_test_wav_frames(path, 44_100);
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

    #[test]
    fn run_copies_date_and_cover_art_end_to_end() {
        use lofty::picture::{MimeType, Picture};
        use lofty::tag::items::Timestamp;

        let dir = std::env::temp_dir().join(format!("lyra-pipeline-art-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        write_test_wav(&source);

        // Layer a date and a front-cover picture onto the tag `write_test_wav`
        // already wrote, so this test exercises exactly the fields
        // `copy_shared_tag_fields` is responsible for.
        let mut tagged = Probe::open(&source).and_then(|p| p.read()).unwrap();
        let mut tag = tagged.primary_tag().cloned().unwrap();
        tag.set_date(Timestamp { year: 2024, ..Default::default() });
        let cover_bytes = vec![0xFFu8, 0xD8, 0xFF, 0xD9]; // minimal fake JPEG payload
        tag.push_picture(Picture::unchecked(cover_bytes.clone()).mime_type(MimeType::Jpeg).build());
        tagged.insert_tag(tag);
        tagged.save_to_path(&source, WriteOptions::default()).unwrap();

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

        let out_tagged = Probe::open(out_dir.join("source.flac")).unwrap().read().unwrap();
        let out_tag = out_tagged.primary_tag().expect("output should have a tag");
        assert_eq!(out_tag.date().map(|t| t.year), Some(2024), "release date should carry over");
        let pictures = out_tag.pictures();
        assert_eq!(pictures.len(), 1, "expected exactly one carried-over picture");
        assert_eq!(pictures[0].data(), &cover_bytes[..], "cover art bytes should be preserved");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn track_progress_range_allocates_equal_monotonic_spans() {
        for total in [1usize, 2, 3, 7] {
            let mut prev_end = 0u32;
            for index in 0..total {
                let (base, span) = track_progress_range(index, total);
                assert_eq!(base, prev_end, "track {index}/{total} should start where the previous one ended");
                assert!(span > 0, "track {index}/{total} got a zero-width progress span");
                prev_end = base + span;
            }
            assert_eq!(prev_end, 1000, "spans for {total} tracks should sum to exactly 1000");
        }
    }

    #[test]
    fn sanitize_filename_blocks_traversal_and_dashes_and_caps_length() {
        assert_eq!(sanitize_filename(".."), "track");
        assert_eq!(sanitize_filename("..."), "track");
        assert!(!sanitize_filename("../../.bashrc").contains('/'));
        assert!(!sanitize_filename("-rf --no-preserve-root").starts_with('-'));
        assert_eq!(sanitize_filename("a\0b").find('\0'), None);

        let long = "x".repeat(1000);
        assert!(sanitize_filename(&long).len() <= 150);
    }

    #[test]
    fn cancelled_conversion_leaves_no_output_file() {
        let dir = std::env::temp_dir().join(format!("lyra-pipeline-cancel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        write_test_wav(&source);

        let out_dir = dir.join("out");
        let job = ConvertJob::new(
            1,
            source,
            JobKind::Convert,
            encoder::OutputFormat::Wav16,
            None,
            out_dir.clone(),
        );
        job.request_cancel();

        let result = run(&job);
        assert!(matches!(result, Err(ConvertError::Cancelled)), "expected Cancelled, got {result:?}");

        let entries: Vec<_> = std::fs::read_dir(&out_dir)
            .map(|rd| rd.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        assert!(entries.is_empty(), "expected no leftover files in {out_dir:?}, found {entries:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_with_resample_does_not_drop_a_short_clip() {
        let dir = std::env::temp_dir().join(format!("lyra-pipeline-resample-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        // Shorter than the resampler's internal processing chunk (1024
        // source-rate frames), so without draining its buffered tail via
        // `flush()` the whole clip would come out empty.
        write_test_wav_frames(&source, 441);

        let out_dir = dir.join("out");
        let job = ConvertJob::new(
            1,
            source,
            JobKind::Convert,
            encoder::OutputFormat::Flac,
            Some(48_000),
            out_dir.clone(),
        );
        run(&job).expect("resampled conversion job should succeed");

        let out_path = out_dir.join("source.flac");
        let tagged = Probe::open(&out_path).unwrap().read().unwrap();
        let duration = tagged.properties().duration();
        assert!(
            duration.as_secs_f64() > 0.0,
            "resampled short clip should not be flushed away entirely, got {duration:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
