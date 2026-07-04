// SPDX-License-Identifier: GPL-3.0

//! Local audio playback backend using rodio.
//!
//! Handles `TrackSource::LocalFile` and `TrackSource::HttpStream`
//! by decoding audio locally and outputting to the system sound device.

use super::PlaybackState;
use super::backend::{PlaybackBackend, PlayerError};
use super::eq_source::{EqController, EqSource, SharedCoeffs, new_shared_coeffs};
use crate::library::TrackSource;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Rodio-based local audio playback backend.
pub struct LocalBackend {
    stream: MixerDeviceSink,
    sink: Arc<Mutex<Player>>,
    state: PlaybackState,
    volume: f32,
    current_duration: Arc<Mutex<Duration>>,
    /// Position at the time playback started or was last seeked.
    base_position: Duration,
    /// When playback started (or resumed). `None` when paused/stopped.
    /// Wrapped in `Arc<Mutex>` so the HTTP download thread can set it
    /// when the decoded audio is appended and playback actually begins.
    play_started_at: Arc<Mutex<Option<Instant>>>,
    /// `true` while a background thread is downloading an HTTP stream.
    /// While loading: `position()` returns ZERO, `is_finished()` returns false.
    loading: Arc<AtomicBool>,
    /// Shared blocking HTTP client for HTTP stream playback.
    http_client: reqwest::blocking::Client,
    /// Shared EQ coefficients (lock-free, read by audio thread).
    eq_coeffs: SharedCoeffs,
    /// Shared EQ bypass flag.
    eq_bypass: Arc<AtomicBool>,
    /// UI-side EQ controller.
    eq_controller: EqController,
    /// Set to `true` by `TrackBoundarySource` when the current track's audio ends.
    track_finished: Arc<AtomicBool>,
    /// Crossfade duration in seconds (0 = disabled, use gapless instead).
    crossfade_secs: f32,
    /// The outgoing sink during a crossfade transition (fading out).
    crossfade_out: Option<Arc<Mutex<Player>>>,
    /// Replay gain to apply to the next track (in dB). Set before `play()`.
    replay_gain_db: Option<f32>,
    /// Shared PCM buffer for the visualizer (copies audio samples for projectM).
    #[cfg(feature = "visualizer")]
    pcm_buffer: Option<Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>>,
}

impl LocalBackend {
    /// Create a new local backend with the default audio output.
    pub fn new() -> Result<Self, PlayerError> {
        let stream = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| PlayerError(format!("Failed to open audio output: {e}")))?;
        let sink = Player::connect_new(stream.mixer());
        let volume = 0.8;
        sink.set_volume(volume);

        let eq_coeffs = new_shared_coeffs();
        let eq_bypass = Arc::new(AtomicBool::new(true)); // EQ disabled by default
        let eq_controller = EqController::new(
            Arc::clone(&eq_coeffs),
            Arc::clone(&eq_bypass),
            44100.0, // default sample rate, updated per track
        );

        Ok(Self {
            stream,
            sink: Arc::new(Mutex::new(sink)),
            state: PlaybackState::Stopped,
            volume,
            current_duration: Arc::new(Mutex::new(Duration::ZERO)),
            base_position: Duration::ZERO,
            play_started_at: Arc::new(Mutex::new(None)),
            loading: Arc::new(AtomicBool::new(false)),
            http_client: reqwest::blocking::Client::new(),
            eq_coeffs,
            eq_bypass,
            eq_controller,
            track_finished: Arc::new(AtomicBool::new(false)),
            crossfade_secs: 0.0,
            crossfade_out: None,
            replay_gain_db: None,
            #[cfg(feature = "visualizer")]
            pcm_buffer: None,
        })
    }

    /// Get a clone of the EQ controller for UI-thread use.
    pub fn eq_controller(&self) -> &EqController {
        &self.eq_controller
    }

    /// Set crossfade duration in seconds (0 = disabled).
    pub fn set_crossfade(&mut self, secs: f32) {
        self.crossfade_secs = secs.max(0.0);
    }

    /// Set the replay gain to apply to the next track.
    ///
    /// Call this before `play()` with the appropriate gain value from the
    /// Track's `rg_track_gain` or `rg_album_gain` based on the current
    /// `ReplayGainMode`.
    pub fn set_replay_gain_db(&mut self, gain_db: Option<f32>) {
        self.replay_gain_db = gain_db;
    }

    /// Cancel any active crossfade immediately (e.g. on manual skip).
    fn cancel_crossfade(&mut self) {
        if let Some(out_sink) = self.crossfade_out.take()
            && let Ok(sink) = out_sink.lock()
        {
            sink.stop();
        }
    }

    /// Start a crossfade transition: the current sink becomes the outgoing
    /// (fading out) sink, and a new sink is created for the incoming track.
    fn start_crossfade<S>(&mut self, source: S) -> Result<(), PlayerError>
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        // Cancel any existing crossfade.
        self.cancel_crossfade();

        // The current sink becomes the outgoing sink (will fade out).
        let outgoing = Arc::clone(&self.sink);

        // Create the incoming sink (starts at zero volume, fades in).
        let new_sink = Player::connect_new(self.stream.mixer());
        new_sink.set_volume(0.0); // Start silent, fade in.

        // Reset track boundary flag for the new track.
        self.track_finished.store(false, Ordering::Release);

        self.append_source_to_sink(&new_sink, source);

        let incoming = Arc::new(Mutex::new(new_sink));
        self.sink = Arc::clone(&incoming);
        self.crossfade_out = Some(outgoing.clone());

        self.state = PlaybackState::Playing;
        *self.current_duration.lock().unwrap() = duration;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = Some(Instant::now());
        self.loading.store(false, Ordering::Release);

        // Spawn a thread to ramp volumes over the crossfade duration.
        let fade_duration = Duration::from_secs_f32(self.crossfade_secs);
        let target_volume = self.volume;

        std::thread::spawn(move || {
            const STEP_MS: u64 = 50;
            let steps = (fade_duration.as_millis() as u64 / STEP_MS).max(1);

            for step in 1..=steps {
                std::thread::sleep(Duration::from_millis(STEP_MS));
                let progress = step as f32 / steps as f32;

                // Fade in the incoming sink.
                if let Ok(sink) = incoming.lock() {
                    sink.set_volume(target_volume * progress);
                }
                // Fade out the outgoing sink.
                if let Ok(sink) = outgoing.lock() {
                    sink.set_volume(target_volume * (1.0 - progress));
                }
            }

            // Ensure final state: outgoing fully stopped, incoming at target volume.
            if let Ok(sink) = outgoing.lock() {
                sink.stop();
            }
            if let Ok(sink) = incoming.lock() {
                sink.set_volume(target_volume);
            }
        });

        Ok(())
    }

    /// Lock the sink mutex.
    fn lock_sink(&self) -> Result<MutexGuard<'_, Player>, PlayerError> {
        self.sink
            .lock()
            .map_err(|e| PlayerError(format!("Audio sink lock poisoned: {e}")))
    }

    /// Internal: play a local file.
    fn play_local_file(&mut self, path: PathBuf) -> Result<(), PlayerError> {
        let file = File::open(&path).map_err(|e| PlayerError(format!("Cannot open file: {e}")))?;
        let reader = BufReader::new(file);
        let source =
            Decoder::new(reader).map_err(|e| PlayerError(format!("Cannot decode file: {e}")))?;

        self.start_source(source)
    }

    /// Internal: play an HTTP stream (e.g. Subsonic `stream` URL).
    ///
    /// Uses [`HttpRangeReader`] to stream audio from the server. The reader
    /// implements `Read + Seek` via HTTP Range requests, so:
    /// - Playback starts immediately (no full download needed).
    /// - Seeking works by re-requesting with `Range: bytes=N-`.
    ///
    /// The initial connection + probe is done in a background thread to
    /// avoid blocking the UI.
    fn play_http_stream(&mut self, url: String) -> Result<(), PlayerError> {
        use super::http_range_reader::HttpRangeReader;

        // Stop current playback
        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);

        // Create a new sink that starts paused — the background thread
        // will append the decoded source and un-pause.
        let new_sink = Player::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);
        new_sink.pause();
        let sink_arc = Arc::new(Mutex::new(new_sink));
        self.sink = Arc::clone(&sink_arc);

        self.state = PlaybackState::Playing;
        *self.current_duration.lock().unwrap() = Duration::ZERO;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = None;
        self.loading.store(true, Ordering::Release);

        let duration_arc = Arc::clone(&self.current_duration);
        let loading = Arc::clone(&self.loading);
        let started_at = Arc::clone(&self.play_started_at);
        let http_client = self.http_client.clone();

        // Capture PCM buffer and pipeline parameters for the background thread
        #[cfg(feature = "visualizer")]
        let pcm_buffer = self.pcm_buffer.clone();
        let eq_coeffs = Arc::clone(&self.eq_coeffs);
        let eq_bypass = Arc::clone(&self.eq_bypass);
        let track_finished = Arc::clone(&self.track_finished);
        let replay_gain_db = self.replay_gain_db;

        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let reader = HttpRangeReader::new(url, Some(http_client))?;
                let byte_len = reader.content_length();

                // Use rodio's builder to set byte_len (enables seeking + duration).
                let source = if byte_len > 0 {
                    Decoder::builder()
                        .with_data(reader)
                        .with_byte_len(byte_len)
                        .build()
                } else {
                    // Unknown length — seeking won't work but playback will.
                    Decoder::builder().with_data(reader).build()
                }
                .map_err(|e| format!("Cannot decode HTTP stream: {e}"))?;

                let duration = source.total_duration().unwrap_or(Duration::ZERO);

                // Build the full source pipeline (ReplayGain → EQ → TrackBoundary → TappedSource)
                // to match local file playback behavior and enable visualizer audio feed.
                let amplified: Box<dyn Source<Item = f32> + Send> =
                    if let Some(gain_db) = replay_gain_db {
                        let linear = 10.0_f32.powf(gain_db / 20.0);
                        Box::new(source.amplify(linear))
                    } else {
                        Box::new(source)
                    };

                let eq_source = EqSource::new(amplified, eq_coeffs, eq_bypass);
                let boundary_source = TrackBoundarySource::new(eq_source, track_finished);

                if let Ok(sink) = sink_arc.lock() {
                    // Apply visualizer tapping if enabled
                    #[cfg(feature = "visualizer")]
                    {
                        if let Some(ref pcm_buf) = pcm_buffer {
                            tracing::debug!(
                                "Creating TappedSource for HTTP stream visualizer audio feed"
                            );
                            let tapped = TappedSource::new(boundary_source, Arc::clone(pcm_buf));
                            sink.append(tapped);
                        } else {
                            tracing::debug!(
                                "No PCM buffer for HTTP stream - visualizer audio tapping disabled"
                            );
                            sink.append(boundary_source);
                        }
                    }
                    #[cfg(not(feature = "visualizer"))]
                    {
                        sink.append(boundary_source);
                    }

                    sink.play();
                }

                *duration_arc.lock().unwrap() = duration;
                *started_at.lock().unwrap() = Some(Instant::now());
                loading.store(false, Ordering::Release);

                Ok(())
            })();

            if let Err(e) = result {
                tracing::error!("HTTP stream playback failed: {e}");
                loading.store(false, Ordering::Release);
                if let Ok(sink) = sink_arc.lock() {
                    sink.stop();
                }
            }
        });

        Ok(())
    }

    /// Set the shared PCM buffer for the visualizer.
    #[cfg(feature = "visualizer")]
    pub fn set_pcm_buffer(
        &mut self,
        buffer: Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>,
    ) {
        self.pcm_buffer = Some(buffer);
    }

    /// Common playback start: stop current sink, create new one, append source.
    ///
    /// Source chain: decoder → EQ → TrackBoundary → TappedSource (visualizer) → Player.
    ///
    /// If crossfade is enabled and audio is currently playing, uses crossfade
    /// transition instead of a hard cut.
    fn start_source<S>(&mut self, source: S) -> Result<(), PlayerError>
    where
        S: Source<Item = f32> + Send + 'static,
    {
        // Use crossfade if enabled and something is currently playing.
        if self.crossfade_secs > 0.0 && self.state == PlaybackState::Playing {
            return self.start_crossfade(source);
        }

        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        // Cancel any in-progress crossfade.
        self.cancel_crossfade();

        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);

        let new_sink = Player::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);

        // Reset track boundary flag for the new track.
        self.track_finished.store(false, Ordering::Release);

        self.append_source_to_sink(&new_sink, source);

        self.sink = Arc::new(Mutex::new(new_sink));
        self.state = PlaybackState::Playing;
        *self.current_duration.lock().unwrap() = duration;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = Some(Instant::now());
        self.loading.store(false, Ordering::Release);

        Ok(())
    }

    /// Build the source chain (ReplayGain → EQ → TrackBoundary → Tapped) and append to sink.
    ///
    /// Extracted so both `start_source()` and `queue_next()` use the same pipeline.
    fn append_source_to_sink<S>(&self, sink: &Player, source: S)
    where
        S: Source<Item = f32> + Send + 'static,
    {
        // Apply replay gain volume adjustment before EQ.
        // Convert dB to linear: gain = 10^(dB/20).
        let amplified: Box<dyn Source<Item = f32> + Send> =
            if let Some(gain_db) = self.replay_gain_db {
                let linear = 10.0_f32.powf(gain_db / 20.0);
                Box::new(source.amplify(linear))
            } else {
                Box::new(source)
            };

        // Wrap in EQ filter (reads shared coefficients lock-free).
        let eq_source = EqSource::new(
            amplified,
            Arc::clone(&self.eq_coeffs),
            Arc::clone(&self.eq_bypass),
        );

        // Wrap in track boundary detector.
        let boundary_source = TrackBoundarySource::new(eq_source, Arc::clone(&self.track_finished));

        // When the visualizer feature is enabled and a PCM buffer is set,
        // wrap the source in TappedSource to feed audio data to projectM.
        #[cfg(feature = "visualizer")]
        {
            if let Some(ref pcm_buf) = self.pcm_buffer {
                tracing::debug!("Creating TappedSource for visualizer audio feed");
                let tapped = TappedSource::new(boundary_source, Arc::clone(pcm_buf));
                sink.append(tapped);
            } else {
                tracing::debug!("No PCM buffer set - visualizer audio tapping disabled");
                sink.append(boundary_source);
            }
        }
        #[cfg(not(feature = "visualizer"))]
        {
            sink.append(boundary_source);
        }
    }
}

impl PlaybackBackend for LocalBackend {
    fn play(&mut self, source: TrackSource) -> Result<(), PlayerError> {
        match source {
            TrackSource::LocalFile(path) => self.play_local_file(path),
            TrackSource::HttpStream(url) => self.play_http_stream(url),
            TrackSource::MpdFile(_) => Err(PlayerError(
                "MPD files should use MpdBackend, not LocalBackend".to_string(),
            )),
        }
    }

    fn pause(&mut self) -> Result<(), PlayerError> {
        if self.state == PlaybackState::Playing {
            // Freeze position: save current elapsed, clear the timer.
            self.base_position = self.position();
            *self.play_started_at.lock().unwrap() = None;

            let sink = self.lock_sink()?;
            sink.pause();
            drop(sink);
            self.state = PlaybackState::Paused;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        if self.state == PlaybackState::Paused {
            let sink = self.lock_sink()?;
            sink.play();
            drop(sink);
            // Resume the timer from saved base_position.
            *self.play_started_at.lock().unwrap() = Some(Instant::now());
            self.state = PlaybackState::Playing;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        // Cancel any in-progress crossfade.
        self.cancel_crossfade();
        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);
        self.state = PlaybackState::Stopped;
        *self.current_duration.lock().unwrap() = Duration::ZERO;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = None;
        self.loading.store(false, Ordering::Release);
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        let sink = self.lock_sink()?;
        sink.try_seek(position)
            .map_err(|e| PlayerError(format!("Seek failed: {e}")))?;
        drop(sink);
        self.base_position = position;
        if self.state == PlaybackState::Playing {
            *self.play_started_at.lock().unwrap() = Some(Instant::now());
        }
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        self.volume = volume.clamp(0.0, 1.0);
        let sink = self.lock_sink()?;
        sink.set_volume(self.volume);
        Ok(())
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn state(&self) -> PlaybackState {
        self.state
    }

    fn position(&self) -> Duration {
        // While the background download is in progress, report zero.
        if self.loading.load(Ordering::Acquire) {
            return Duration::ZERO;
        }
        match self.state {
            PlaybackState::Playing => {
                let elapsed = self
                    .play_started_at
                    .lock()
                    .unwrap()
                    .map(|t| t.elapsed())
                    .unwrap_or(Duration::ZERO);
                self.base_position + elapsed
            }
            PlaybackState::Paused => self.base_position,
            PlaybackState::Stopped => Duration::ZERO,
        }
    }

    fn duration(&self) -> Duration {
        *self.current_duration.lock().unwrap()
    }

    fn is_finished(&self) -> Result<bool, PlayerError> {
        // While the background download is in progress, never "finished".
        if self.loading.load(Ordering::Acquire) {
            return Ok(false);
        }
        // Only consider "finished" if we were actively playing and the
        // sink ran out of sources (i.e. the track ended naturally).
        // Stopped/Paused states are never "finished" — they represent
        // explicit user actions, not natural track completion.
        if self.state != PlaybackState::Playing {
            return Ok(false);
        }
        // Check the track boundary flag (set by TrackBoundarySource when
        // the current track's audio data is exhausted). This fires even when
        // a next track is already queued in the sink, allowing the UI to
        // update metadata without a gap in audio.
        if self.track_finished.load(Ordering::Acquire) {
            return Ok(true);
        }
        // Fallback: also check if the sink itself is completely empty.
        let sink = self.lock_sink()?;
        Ok(sink.empty())
    }

    fn queue_next(&mut self, source: TrackSource) -> Result<(), PlayerError> {
        // Decode the next track.
        let decoded: Box<dyn Source<Item = f32> + Send> = match source {
            TrackSource::LocalFile(ref path) => {
                let file =
                    File::open(path).map_err(|e| PlayerError(format!("Cannot open file: {e}")))?;
                let reader = BufReader::new(file);
                let dec = Decoder::new(reader)
                    .map_err(|e| PlayerError(format!("Cannot decode file: {e}")))?;
                Box::new(dec)
            }
            TrackSource::HttpStream(_) => {
                // HTTP streams don't support gapless pre-queuing (they
                // need background download). Fall back to normal play.
                return Ok(());
            }
            TrackSource::MpdFile(_) => {
                return Err(PlayerError("MPD files should use MpdBackend".to_string()));
            }
        };

        // Reset the boundary flag so the next track's boundary is detected.
        self.track_finished.store(false, Ordering::Release);

        // Append to the existing sink (gapless).
        let sink = self.lock_sink()?;
        self.append_source_to_sink(&sink, decoded);
        Ok(())
    }
}

// --- TrackBoundarySource adapter for gapless playback ---

/// A `Source` wrapper that sets a shared flag when the inner source is exhausted.
///
/// Used with a persistent `Player` to detect when a track ends naturally,
/// allowing the UI to update metadata and pre-queue the next track for
/// gapless playback.
pub struct TrackBoundarySource<S> {
    inner: S,
    finished: Arc<AtomicBool>,
    /// Whether we've already signaled completion (to avoid repeated stores).
    signaled: bool,
}

impl<S> TrackBoundarySource<S> {
    /// Wrap a source with track boundary detection.
    pub fn new(inner: S, finished: Arc<AtomicBool>) -> Self {
        finished.store(false, Ordering::Release);
        Self {
            inner,
            finished,
            signaled: false,
        }
    }
}

impl<S> Iterator for TrackBoundarySource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        match self.inner.next() {
            Some(sample) => Some(sample),
            None => {
                if !self.signaled {
                    self.finished.store(true, Ordering::Release);
                    self.signaled = true;
                }
                None
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> Source for TrackBoundarySource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        self.inner.channels()
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

// --- TappedSource adapter for visualizer PCM feed ---

/// A `Source` wrapper that copies each sample to a shared PCM buffer
/// before yielding it. Used by the ProjectM visualizer to read audio data.
#[cfg(feature = "visualizer")]
pub struct TappedSource<S> {
    inner: S,
    pcm_buffer: Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>,
}

#[cfg(feature = "visualizer")]
impl<S> TappedSource<S> {
    /// Wrap a source, copying samples to the shared PCM buffer.
    pub fn new(
        inner: S,
        pcm_buffer: Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>,
    ) -> Self {
        Self { inner, pcm_buffer }
    }
}

#[cfg(feature = "visualizer")]
impl<S> Iterator for TappedSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        // Copy to shared buffer (best-effort, don't block audio on lock contention)
        if let Ok(mut buf) = self.pcm_buffer.try_lock() {
            buf.write(&[sample]);
        } else {
            // Log occasional lock contention (debug build only)
            #[cfg(debug_assertions)]
            {
                static WARN_COUNTER: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                if WARN_COUNTER
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .is_multiple_of(48000)
                {
                    tracing::debug!("PCM buffer lock contention (visualizer may be lagging)");
                }
            }
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[cfg(feature = "visualizer")]
impl<S> Source for TappedSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        self.inner.channels()
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}
