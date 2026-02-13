// SPDX-License-Identifier: GPL-3.0

//! Audio playback engine backed by rodio.

pub mod equalizer;


use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Represents the current playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// The currently loaded track info (runtime state, not metadata).
#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub path: PathBuf,
    pub duration: Duration,
    pub position: Duration,
}

/// Core audio player that manages playback through rodio.
pub struct Player {
    stream: OutputStream,
    sink: Arc<Mutex<Sink>>,
    state: PlaybackState,
    current_track: Option<NowPlaying>,
    volume: f32,
    queue: Vec<PathBuf>,
    queue_index: usize,
}

impl Player {
    /// Create a new player instance.
    pub fn new() -> Result<Self, String> {
        let stream = OutputStreamBuilder::open_default_stream()
            .map_err(|e| format!("Failed to open audio output: {e}"))?;
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(0.8);

        Ok(Self {
            stream,
            sink: Arc::new(Mutex::new(sink)),
            state: PlaybackState::Stopped,
            current_track: None,
            volume: 0.8,
            queue: Vec::new(),
            queue_index: 0,
        })
    }

    /// Acquire the sink mutex lock, returning a descriptive error if poisoned.
    fn lock_sink(&self) -> Result<MutexGuard<'_, Sink>, String> {
        self.sink
            .lock()
            .map_err(|e| format!("Audio sink lock poisoned: {e}"))
    }

    /// Load and immediately play a file.
    pub fn play_file(&mut self, path: &Path) -> Result<(), String> {
        let file = File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
        let reader = BufReader::new(file);
        let source = Decoder::new(reader).map_err(|e| format!("Cannot decode file: {e}"))?;

        let duration = source.total_duration().unwrap_or(Duration::ZERO);

        let sink = self.lock_sink()?;
        sink.stop();
        // After stop, we need a new sink
        drop(sink);

        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);
        new_sink.append(source);

        self.sink = Arc::new(Mutex::new(new_sink));
        self.state = PlaybackState::Playing;
        self.current_track = Some(NowPlaying {
            path: path.to_path_buf(),
            duration,
            position: Duration::ZERO,
        });

        Ok(())
    }

    /// Toggle play/pause.
    pub fn toggle_playback(&mut self) -> Result<(), String> {
        let sink = self.lock_sink()?;
        let new_state = match self.state {
            PlaybackState::Playing => {
                sink.pause();
                Some(PlaybackState::Paused)
            }
            PlaybackState::Paused => {
                sink.play();
                Some(PlaybackState::Playing)
            }
            PlaybackState::Stopped => None,
        };
        drop(sink);
        if let Some(state) = new_state {
            self.state = state;
        }
        Ok(())
    }

    /// Stop playback entirely.
    pub fn stop(&mut self) -> Result<(), String> {
        let sink = self.lock_sink()?;
        sink.stop();
        drop(sink);
        self.state = PlaybackState::Stopped;
        self.current_track = None;
        Ok(())
    }

    /// Set volume (0.0 - 1.0).
    pub fn set_volume(&mut self, volume: f32) -> Result<(), String> {
        self.volume = volume.clamp(0.0, 1.0);
        let sink = self.lock_sink()?;
        sink.set_volume(self.volume);
        Ok(())
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Seek to a position (requires re-decoding for rodio).
    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        let sink = self.lock_sink()?;
        sink.try_seek(position)
            .map_err(|e| format!("Seek failed: {e}"))?;
        drop(sink);
        if let Some(ref mut np) = self.current_track {
            np.position = position;
        }
        Ok(())
    }

    /// Get the current playback state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Get current track info.
    pub fn now_playing(&self) -> Option<&NowPlaying> {
        self.current_track.as_ref()
    }

    /// Check if playback is finished (sink empty).
    pub fn is_finished(&self) -> Result<bool, String> {
        let sink = self.lock_sink()?;
        Ok(sink.empty())
    }

    // -- Queue management --

    /// Set the play queue.
    pub fn set_queue(&mut self, tracks: Vec<PathBuf>) {
        self.queue = tracks;
        self.queue_index = 0;
    }

    /// Play the next track in the queue.
    pub fn next(&mut self) -> Result<(), String> {
        if self.queue.is_empty() {
            return Ok(());
        }
        self.queue_index = (self.queue_index + 1) % self.queue.len();
        let path = self.queue[self.queue_index].clone();
        self.play_file(&path)
    }

    /// Play the previous track in the queue.
    pub fn previous(&mut self) -> Result<(), String> {
        if self.queue.is_empty() {
            return Ok(());
        }
        if self.queue_index == 0 {
            self.queue_index = self.queue.len() - 1;
        } else {
            self.queue_index -= 1;
        }
        let path = self.queue[self.queue_index].clone();
        self.play_file(&path)
    }

    /// Play a specific index in the queue.
    pub fn play_index(&mut self, index: usize) -> Result<(), String> {
        if index >= self.queue.len() {
            return Err("Index out of bounds".into());
        }
        self.queue_index = index;
        let path = self.queue[index].clone();
        self.play_file(&path)
    }

    pub fn queue(&self) -> &[PathBuf] {
        &self.queue
    }

    pub fn queue_index(&self) -> usize {
        self.queue_index
    }
}
