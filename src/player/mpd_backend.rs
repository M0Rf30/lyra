// SPDX-License-Identifier: GPL-3.0

//! MPD playback backend.
//!
//! Sends play/pause/seek/volume commands to an MPD server instead of
//! decoding audio locally. The MPD server owns the audio output.

use super::backend::{PlaybackBackend, PlayerError};
use super::PlaybackState;
use crate::library::TrackSource;
use mpd_client::commands::{
    Add, ClearQueue, Play, Seek, SeekMode, SetPause, SetVolume, Status, Stop,
};
use mpd_client::responses::PlayState;
use mpd_client::Client;
use std::time::Duration;
use tokio::runtime::Handle;

/// MPD-based playback backend.
///
/// Playback happens on the MPD server. This backend sends protocol commands
/// and queries status. The `Client` is `Clone + Send + Sync` so we hold
/// it directly. A tokio `Handle` is used to bridge sync calls to async.
pub struct MpdBackend {
    client: Client,
    runtime: Handle,
    volume: f32,
    cached_state: PlaybackState,
    cached_position: Duration,
    cached_duration: Duration,
}

impl MpdBackend {
    /// Create a new MPD backend from a connected client.
    pub fn new(client: Client, runtime: Handle) -> Self {
        Self {
            client,
            runtime,
            volume: 1.0,
            cached_state: PlaybackState::Stopped,
            cached_position: Duration::ZERO,
            cached_duration: Duration::ZERO,
        }
    }

    /// Refresh cached state from MPD status.
    pub fn refresh_status(&mut self) -> Result<(), PlayerError> {
        let status = self
            .runtime
            .block_on(self.client.command(Status))
            .map_err(|e| PlayerError(format!("MPD status: {e}")))?;

        self.cached_state = match status.state {
            PlayState::Playing => PlaybackState::Playing,
            PlayState::Paused => PlaybackState::Paused,
            PlayState::Stopped => PlaybackState::Stopped,
        };
        self.cached_position = status.elapsed.unwrap_or(Duration::ZERO);
        self.cached_duration = status.duration.unwrap_or(Duration::ZERO);
        self.volume = status.volume as f32 / 100.0;

        Ok(())
    }
}

impl PlaybackBackend for MpdBackend {
    fn play(&mut self, source: TrackSource) -> Result<(), PlayerError> {
        match source {
            TrackSource::MpdFile(uri) => {
                self.runtime
                    .block_on(async {
                        self.client.command(ClearQueue).await?;
                        self.client.command(Add::uri(&uri)).await?;
                        self.client.command(Play::current()).await
                    })
                    .map_err(|e| PlayerError(format!("MPD play: {e}")))?;

                self.cached_state = PlaybackState::Playing;
                self.cached_position = Duration::ZERO;
                // Refresh to get duration
                self.refresh_status().ok();
                Ok(())
            }
            _ => Err(PlayerError(
                "MpdBackend only handles TrackSource::MpdFile".into(),
            )),
        }
    }

    fn pause(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(SetPause(true)))
            .map_err(|e| PlayerError(format!("MPD pause: {e}")))?;
        self.cached_state = PlaybackState::Paused;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(SetPause(false)))
            .map_err(|e| PlayerError(format!("MPD resume: {e}")))?;
        self.cached_state = PlaybackState::Playing;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(Stop))
            .map_err(|e| PlayerError(format!("MPD stop: {e}")))?;
        self.cached_state = PlaybackState::Stopped;
        self.cached_position = Duration::ZERO;
        self.cached_duration = Duration::ZERO;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(Seek(SeekMode::Absolute(position))))
            .map_err(|e| PlayerError(format!("MPD seek: {e}")))?;
        self.cached_position = position;
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        let vol_int = (volume.clamp(0.0, 1.0) * 100.0) as u8;
        self.runtime
            .block_on(self.client.command(SetVolume(vol_int)))
            .map_err(|e| PlayerError(format!("MPD setvol: {e}")))?;
        self.volume = volume.clamp(0.0, 1.0);
        Ok(())
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn state(&self) -> PlaybackState {
        self.cached_state
    }

    fn position(&self) -> Duration {
        self.cached_position
    }

    fn duration(&self) -> Duration {
        self.cached_duration
    }

    fn is_finished(&self) -> Result<bool, PlayerError> {
        // For MPD, "finished" means stopped and we had been playing.
        // We rely on refresh_status() being called periodically via tick/idle.
        Ok(self.cached_state == PlaybackState::Stopped && self.cached_position > Duration::ZERO)
    }
}
