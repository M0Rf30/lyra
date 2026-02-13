// SPDX-License-Identifier: GPL-3.0

//! Audio playback engine with pluggable backend support.

pub mod backend;
pub mod equalizer;
mod http_range_reader;
pub mod local_backend;
pub mod mpd_backend;

use crate::library::{Track, TrackSource};
use backend::{PlaybackBackend, PlayerError};
use local_backend::LocalBackend;
use mpd_backend::MpdBackend;
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

/// Which backend is currently active for playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend {
    /// Local rodio-based playback (local files + HTTP streams).
    Local,
    /// MPD server playback.
    Mpd,
}

/// Core audio player that manages playback through concrete backends.
///
/// Instead of dynamic dispatch via `Box<dyn PlaybackBackend>`, we hold
/// each backend in a named field and switch via `active_backend`.
pub struct Player {
    local_backend: LocalBackend,
    mpd_backend: Option<MpdBackend>,
    active_backend: ActiveBackend,
    current_track: Option<NowPlaying>,
    volume: f32,
    queue: Vec<Track>,
    queue_index: usize,
}

impl Player {
    /// Create a new player instance.
    ///
    /// `mpd_backend` should be `Some(...)` when an MPD provider is active.
    pub fn new(mpd_backend: Option<MpdBackend>) -> Result<Self, String> {
        let local_backend = LocalBackend::new().map_err(|e| e.to_string())?;

        Ok(Self {
            local_backend,
            mpd_backend,
            active_backend: ActiveBackend::Local,
            current_track: None,
            volume: 0.8,
            queue: Vec::new(),
            queue_index: 0,
        })
    }

    /// Get a reference to the currently active backend.
    fn active(&self) -> &dyn PlaybackBackend {
        match self.active_backend {
            ActiveBackend::Local => &self.local_backend,
            ActiveBackend::Mpd => self
                .mpd_backend
                .as_ref()
                .expect("MPD backend not available"),
        }
    }

    /// Get a mutable reference to the currently active backend.
    fn active_mut(&mut self) -> &mut dyn PlaybackBackend {
        match self.active_backend {
            ActiveBackend::Local => &mut self.local_backend,
            ActiveBackend::Mpd => self
                .mpd_backend
                .as_mut()
                .expect("MPD backend not available"),
        }
    }

    /// Play a track by resolving its source.
    #[tracing::instrument(skip(self, track, source), level = "debug")]
    pub fn play_track(&mut self, track: &Track, source: TrackSource) -> Result<(), String> {
        // Select the appropriate backend based on the track source.
        match &source {
            TrackSource::LocalFile(_) | TrackSource::HttpStream(_) => {
                self.active_backend = ActiveBackend::Local;
            }
            TrackSource::MpdFile(_) => {
                if self.mpd_backend.is_none() {
                    return Err("MPD backend not available for MpdFile source".into());
                }
                self.active_backend = ActiveBackend::Mpd;
            }
        }

        self.active_mut().play(source).map_err(|e| e.to_string())?;
        self.volume = self.active().volume();
        self.current_track = Some(NowPlaying {
            track: track.clone(),
            duration: self.active().duration(),
        });
        Ok(())
    }

    /// Toggle play/pause.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn toggle_playback(&mut self) -> Result<(), String> {
        match self.active().state() {
            PlaybackState::Playing => self.active_mut().pause().map_err(|e| e.to_string()),
            PlaybackState::Paused => self.active_mut().resume().map_err(|e| e.to_string()),
            PlaybackState::Stopped => Ok(()),
        }
    }

    /// Stop playback entirely.
    pub fn stop(&mut self) -> Result<(), String> {
        self.active_mut().stop().map_err(|e| e.to_string())?;
        self.current_track = None;
        Ok(())
    }

    /// Set volume (0.0 - 1.0).
    pub fn set_volume(&mut self, volume: f32) -> Result<(), String> {
        let clamped = volume.clamp(0.0, 1.0);
        self.active_mut()
            .set_volume(clamped)
            .map_err(|e| e.to_string())?;
        self.volume = clamped;
        Ok(())
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Seek to a position.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        self.active_mut()
            .seek(position)
            .map_err(|e| e.to_string())
    }

    /// Get the current playback position from the active backend.
    pub fn position(&self) -> Duration {
        self.active().position()
    }

    /// Get the current track duration from the active backend.
    pub fn duration(&self) -> Duration {
        self.active().duration()
    }

    /// Get the current playback state.
    pub fn state(&self) -> PlaybackState {
        self.active().state()
    }

    /// Get current track info.
    pub fn now_playing(&self) -> Option<&NowPlaying> {
        self.current_track.as_ref()
    }

    /// Check if playback is finished (sink empty / track ended).
    pub fn is_finished(&self) -> Result<bool, String> {
        self.active().is_finished().map_err(|e| e.to_string())
    }

    // -- Queue management --

    /// Set the play queue from a list of tracks.
    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.queue = tracks;
        self.queue_index = 0;
    }

    /// Play the next track in the queue.
    /// Returns the track that is now playing, or None if queue is empty.
    #[allow(clippy::should_implement_trait)]
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

    /// Which backend type is currently active.
    pub fn active_backend_type(&self) -> ActiveBackend {
        self.active_backend
    }

    /// Get a reference to the MPD backend (if present).
    pub fn mpd_backend_ref(&self) -> Option<&MpdBackend> {
        self.mpd_backend.as_ref()
    }

    /// Get a mutable reference to the MPD backend (if present).
    pub fn mpd_backend_mut(&mut self) -> Option<&mut MpdBackend> {
        self.mpd_backend.as_mut()
    }
}
