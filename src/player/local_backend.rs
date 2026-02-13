// SPDX-License-Identifier: GPL-3.0

//! Local audio playback backend using rodio.
//!
//! Handles `TrackSource::LocalFile` and `TrackSource::HttpStream`
//! by decoding audio locally and outputting to the system sound device.

use super::backend::{PlaybackBackend, PlayerError};
use super::PlaybackState;
use crate::library::TrackSource;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Rodio-based local audio playback backend.
pub struct LocalBackend {
    stream: OutputStream,
    sink: Arc<Mutex<Sink>>,
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
}

impl LocalBackend {
    /// Create a new local backend with the default audio output.
    pub fn new() -> Result<Self, PlayerError> {
        let stream = OutputStreamBuilder::open_default_stream()
            .map_err(|e| PlayerError(format!("Failed to open audio output: {e}")))?;
        let sink = Sink::connect_new(stream.mixer());
        let volume = 0.8;
        sink.set_volume(volume);

        Ok(Self {
            stream,
            sink: Arc::new(Mutex::new(sink)),
            state: PlaybackState::Stopped,
            volume,
            current_duration: Arc::new(Mutex::new(Duration::ZERO)),
            base_position: Duration::ZERO,
            play_started_at: Arc::new(Mutex::new(None)),
            loading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Lock the sink mutex.
    fn lock_sink(&self) -> Result<MutexGuard<'_, Sink>, PlayerError> {
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
    /// Downloads the audio in a background thread so the UI stays responsive.
    /// The sink is created immediately (in paused/empty state), and once the
    /// download+decode finishes the source is appended and playback begins.
    fn play_http_stream(&mut self, url: String) -> Result<(), PlayerError> {
        // Stop current playback
        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);

        // Create a new sink that starts paused — the background thread
        // will append the decoded source and un-pause.
        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);
        new_sink.pause(); // Don't play until audio is ready
        let sink_arc = Arc::new(Mutex::new(new_sink));
        self.sink = Arc::clone(&sink_arc);

        // Transition to Playing state with zero duration (updated later).
        self.state = PlaybackState::Playing;
        *self.current_duration.lock().unwrap() = Duration::ZERO;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = None; // Set once download completes
        self.loading.store(true, Ordering::Release);

        let duration_arc = Arc::clone(&self.current_duration);
        let loading = Arc::clone(&self.loading);
        let started_at = Arc::clone(&self.play_started_at);

        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let bytes = reqwest::blocking::get(&url)
                    .map_err(|e| format!("HTTP stream request failed: {e}"))?
                    .bytes()
                    .map_err(|e| format!("HTTP stream read failed: {e}"))?;

                log::info!(
                    "HTTP stream downloaded: {} bytes from {}",
                    bytes.len(),
                    url.split('?').next().unwrap_or(&url)
                );

                let cursor = Cursor::new(bytes.to_vec());
                let source =
                    Decoder::new(cursor).map_err(|e| format!("Cannot decode HTTP stream: {e}"))?;

                let duration = source.total_duration().unwrap_or(Duration::ZERO);

                // Append decoded audio and start playback.
                if let Ok(sink) = sink_arc.lock() {
                    sink.append(source);
                    sink.play();
                }

                *duration_arc.lock().unwrap() = duration;
                *started_at.lock().unwrap() = Some(Instant::now());
                loading.store(false, Ordering::Release);

                Ok(())
            })();

            if let Err(e) = result {
                log::error!("HTTP stream playback failed: {e}");
                loading.store(false, Ordering::Release);
                // Stop the (empty) sink so is_finished() can detect failure.
                if let Ok(sink) = sink_arc.lock() {
                    sink.stop();
                }
            }
        });

        Ok(())
    }

    /// Common playback start: stop current sink, create new one, append source.
    fn start_source<S>(&mut self, source: S) -> Result<(), PlayerError>
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);

        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);
        new_sink.append(source);

        self.sink = Arc::new(Mutex::new(new_sink));
        self.state = PlaybackState::Playing;
        *self.current_duration.lock().unwrap() = duration;
        self.base_position = Duration::ZERO;
        *self.play_started_at.lock().unwrap() = Some(Instant::now());
        self.loading.store(false, Ordering::Release);

        Ok(())
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
        let sink = self.lock_sink()?;
        Ok(sink.empty())
    }
}
