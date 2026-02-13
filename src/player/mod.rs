// SPDX-License-Identifier: GPL-3.0

//! Audio playback engine with pluggable backend support.

pub mod backend;
pub mod equalizer;
pub mod local_backend;
pub mod mpd_backend;

use crate::library::{Track, TrackSource};
use backend::{PlaybackBackend, PlayerError};
use local_backend::LocalBackend;
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
    pub track: Track,
    pub duration: Duration,
    pub position: Duration,
}

/// Resolve a `Track` to its `TrackSource` based on the provider_id field.
///
/// This avoids coupling the Player to the ProviderRegistry.
fn resolve_track_source(track: &Track) -> TrackSource {
    if track.provider_id.starts_with("mpd") {
        TrackSource::MpdFile(track.source_uri.clone())
    } else if track.provider_id.starts_with("subsonic") {
        // source_uri contains the pre-built authenticated stream URL.
        TrackSource::HttpStream(track.source_uri.clone())
    } else {
        // Default: local file
        TrackSource::LocalFile(track.path.clone())
    }
}

/// Core audio player that manages playback through a backend.
pub struct Player {
    backend: Box<dyn PlaybackBackend>,
    current_track: Option<NowPlaying>,
    volume: f32,
    queue: Vec<Track>,
    queue_index: usize,
}

impl Player {
    /// Create a new player instance with a local (rodio) backend.
    pub fn new() -> Result<Self, String> {
        let backend = LocalBackend::new().map_err(|e| e.to_string())?;

        Ok(Self {
            backend: Box::new(backend),
            current_track: None,
            volume: 0.8,
            queue: Vec::new(),
            queue_index: 0,
        })
    }

    /// Play a track by resolving its source.
    pub fn play_track(&mut self, track: &Track, source: TrackSource) -> Result<(), String> {
        self.backend.play(source).map_err(|e| e.to_string())?;
        self.volume = self.backend.volume();
        self.current_track = Some(NowPlaying {
            track: track.clone(),
            duration: self.backend.duration(),
            position: Duration::ZERO,
        });
        Ok(())
    }

    /// Toggle play/pause.
    pub fn toggle_playback(&mut self) -> Result<(), String> {
        match self.backend.state() {
            PlaybackState::Playing => self.backend.pause().map_err(|e| e.to_string()),
            PlaybackState::Paused => self.backend.resume().map_err(|e| e.to_string()),
            PlaybackState::Stopped => Ok(()),
        }
    }

    /// Stop playback entirely.
    pub fn stop(&mut self) -> Result<(), String> {
        self.backend.stop().map_err(|e| e.to_string())?;
        self.current_track = None;
        Ok(())
    }

    /// Set volume (0.0 - 1.0).
    pub fn set_volume(&mut self, volume: f32) -> Result<(), String> {
        self.volume = volume.clamp(0.0, 1.0);
        self.backend
            .set_volume(self.volume)
            .map_err(|e| e.to_string())
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Seek to a position.
    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        self.backend.seek(position).map_err(|e| e.to_string())?;
        if let Some(ref mut np) = self.current_track {
            np.position = position;
        }
        Ok(())
    }

    /// Get the current playback state.
    pub fn state(&self) -> PlaybackState {
        self.backend.state()
    }

    /// Get current track info.
    pub fn now_playing(&self) -> Option<&NowPlaying> {
        self.current_track.as_ref()
    }

    /// Check if playback is finished (sink empty).
    pub fn is_finished(&self) -> Result<bool, String> {
        self.backend.is_finished().map_err(|e| e.to_string())
    }

    // -- Queue management --

    /// Set the play queue from a list of tracks.
    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.queue = tracks;
        self.queue_index = 0;
    }

    /// Play the next track in the queue.
    /// Returns the track that is now playing, or None if queue is empty.
    pub fn next(&mut self) -> Result<Option<&Track>, String> {
        if self.queue.is_empty() {
            return Ok(None);
        }
        self.queue_index = (self.queue_index + 1) % self.queue.len();
        let track = self.queue[self.queue_index].clone();
        let source = resolve_track_source(&track);
        self.play_track(&track, source)?;
        Ok(self.queue.get(self.queue_index))
    }

    /// Play the previous track in the queue.
    pub fn previous(&mut self) -> Result<Option<&Track>, String> {
        if self.queue.is_empty() {
            return Ok(None);
        }
        if self.queue_index == 0 {
            self.queue_index = self.queue.len() - 1;
        } else {
            self.queue_index -= 1;
        }
        let track = self.queue[self.queue_index].clone();
        let source = resolve_track_source(&track);
        self.play_track(&track, source)?;
        Ok(self.queue.get(self.queue_index))
    }

    /// Play a specific index in the queue.
    pub fn play_index(&mut self, index: usize) -> Result<(), String> {
        if index >= self.queue.len() {
            return Err("Index out of bounds".into());
        }
        self.queue_index = index;
        let track = self.queue[index].clone();
        let source = resolve_track_source(&track);
        self.play_track(&track, source)
    }

    pub fn queue(&self) -> &[Track] {
        &self.queue
    }

    pub fn queue_index(&self) -> usize {
        self.queue_index
    }
}
