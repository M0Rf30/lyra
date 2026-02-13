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
use std::cell::RefCell;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

/// How often we query MPD for fresh status (position, state, duration).
const STATUS_TTL: Duration = Duration::from_millis(200);

/// Cached MPD status fields, refreshed on demand via `ensure_fresh()`.
struct CachedStatus {
    state: PlaybackState,
    position: Duration,
    duration: Duration,
    volume: f32,
    last_refresh: Instant,
}

/// MPD-based playback backend.
///
/// Playback happens on the MPD server. This backend sends protocol commands
/// and queries status. The `Client` is `Clone + Send + Sync` so we hold
/// it directly. A tokio `Handle` is used to bridge sync calls to async.
///
/// The cached status uses `RefCell` for interior mutability so that
/// `position()`, `state()`, `duration()`, and `is_finished()` (which take
/// `&self` per the `PlaybackBackend` trait) can transparently refresh the
/// cache when it goes stale.
pub struct MpdBackend {
    client: Client,
    runtime: Handle,
    cache: RefCell<CachedStatus>,
    /// Set `true` on `play()`, cleared on `stop()`.
    /// Used by `is_finished()` to distinguish "track ended" from "never played".
    was_playing: bool,
}

impl MpdBackend {
    /// Create a new MPD backend from a connected client.
    pub fn new(client: Client, runtime: Handle) -> Self {
        Self {
            client,
            runtime,
            cache: RefCell::new(CachedStatus {
                state: PlaybackState::Stopped,
                position: Duration::ZERO,
                duration: Duration::ZERO,
                volume: 1.0,
                last_refresh: Instant::now(),
            }),
            was_playing: false,
        }
    }

    /// Refresh cached state from MPD status (unconditionally).
    fn refresh_status(&self) -> Result<(), PlayerError> {
        let status = self
            .runtime
            .block_on(self.client.command(Status))
            .map_err(|e| PlayerError(format!("MPD status: {e}")))?;

        let mut cache = self.cache.borrow_mut();
        cache.state = match status.state {
            PlayState::Playing => PlaybackState::Playing,
            PlayState::Paused => PlaybackState::Paused,
            PlayState::Stopped => PlaybackState::Stopped,
        };
        cache.position = status.elapsed.unwrap_or(Duration::ZERO);
        cache.duration = status.duration.unwrap_or(Duration::ZERO);
        cache.volume = status.volume as f32 / 100.0;
        cache.last_refresh = Instant::now();

        Ok(())
    }

    /// Refresh the cache only if it is older than `STATUS_TTL`.
    fn ensure_fresh(&self) {
        let stale = self.cache.borrow().last_refresh.elapsed() > STATUS_TTL;
        if stale
            && let Err(e) = self.refresh_status()
        {
            log::warn!("MpdBackend: failed to refresh status: {e}");
        }
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

                self.was_playing = true;
                {
                    let mut cache = self.cache.borrow_mut();
                    cache.state = PlaybackState::Playing;
                    cache.position = Duration::ZERO;
                }
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
        self.cache.borrow_mut().state = PlaybackState::Paused;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(SetPause(false)))
            .map_err(|e| PlayerError(format!("MPD resume: {e}")))?;
        self.cache.borrow_mut().state = PlaybackState::Playing;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(Stop))
            .map_err(|e| PlayerError(format!("MPD stop: {e}")))?;
        self.was_playing = false;
        {
            let mut cache = self.cache.borrow_mut();
            cache.state = PlaybackState::Stopped;
            cache.position = Duration::ZERO;
            cache.duration = Duration::ZERO;
        }
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        self.runtime
            .block_on(self.client.command(Seek(SeekMode::Absolute(position))))
            .map_err(|e| PlayerError(format!("MPD seek: {e}")))?;
        let mut cache = self.cache.borrow_mut();
        cache.position = position;
        // Mark cache as fresh so we don't immediately override
        // the seek target with a stale status query.
        cache.last_refresh = Instant::now();
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        let vol_int = (volume.clamp(0.0, 1.0) * 100.0) as u8;
        self.runtime
            .block_on(self.client.command(SetVolume(vol_int)))
            .map_err(|e| PlayerError(format!("MPD setvol: {e}")))?;
        self.cache.borrow_mut().volume = volume.clamp(0.0, 1.0);
        Ok(())
    }

    fn volume(&self) -> f32 {
        self.cache.borrow().volume
    }

    fn state(&self) -> PlaybackState {
        self.ensure_fresh();
        self.cache.borrow().state
    }

    fn position(&self) -> Duration {
        self.ensure_fresh();
        self.cache.borrow().position
    }

    fn duration(&self) -> Duration {
        self.ensure_fresh();
        self.cache.borrow().duration
    }

    fn is_finished(&self) -> Result<bool, PlayerError> {
        self.ensure_fresh();
        let state = self.cache.borrow().state;
        // Track ended naturally: MPD reports Stopped, and we had been playing.
        Ok(state == PlaybackState::Stopped && self.was_playing)
    }
}
