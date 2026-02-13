// SPDX-License-Identifier: GPL-3.0

//! Local audio playback backend using rodio.
//!
//! Handles `TrackSource::LocalFile` (and in future, `TrackSource::HttpStream`)
//! by decoding audio locally and outputting to the system sound device.

use super::backend::{PlaybackBackend, PlayerError};
use super::PlaybackState;
use crate::library::TrackSource;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Rodio-based local audio playback backend.
pub struct LocalBackend {
    stream: OutputStream,
    sink: Arc<Mutex<Sink>>,
    state: PlaybackState,
    volume: f32,
    current_duration: Duration,
    current_position: Duration,
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
            current_duration: Duration::ZERO,
            current_position: Duration::ZERO,
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

        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);

        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);
        new_sink.append(source);

        self.sink = Arc::new(Mutex::new(new_sink));
        self.state = PlaybackState::Playing;
        self.current_duration = duration;
        self.current_position = Duration::ZERO;

        Ok(())
    }
}

impl PlaybackBackend for LocalBackend {
    fn play(&mut self, source: TrackSource) -> Result<(), PlayerError> {
        match source {
            TrackSource::LocalFile(path) => self.play_local_file(path),
            TrackSource::HttpStream(_url) => {
                // TODO: Phase 3 — HTTP streaming via reqwest + symphonia
                Err(PlayerError(
                    "HTTP streaming not yet implemented".to_string(),
                ))
            }
            TrackSource::MpdFile(_) => Err(PlayerError(
                "MPD files should use MpdBackend, not LocalBackend".to_string(),
            )),
        }
    }

    fn pause(&mut self) -> Result<(), PlayerError> {
        if self.state == PlaybackState::Playing {
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
            self.state = PlaybackState::Playing;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);
        self.state = PlaybackState::Stopped;
        self.current_duration = Duration::ZERO;
        self.current_position = Duration::ZERO;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        let sink = self.lock_sink()?;
        sink.try_seek(position)
            .map_err(|e| PlayerError(format!("Seek failed: {e}")))?;
        drop(sink);
        self.current_position = position;
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
        self.current_position
    }

    fn duration(&self) -> Duration {
        self.current_duration
    }

    fn is_finished(&self) -> Result<bool, PlayerError> {
        let sink = self.lock_sink()?;
        Ok(sink.empty())
    }
}
