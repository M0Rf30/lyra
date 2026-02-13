// SPDX-License-Identifier: GPL-3.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Repeat mode for playback queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    None,
    All,
    One,
}

impl RepeatMode {
    /// Advance to the next repeat mode: None → All → One → None.
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::All,
            Self::All => Self::One,
            Self::One => Self::None,
        }
    }

    /// Icon name for this repeat mode.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::One => "media-playlist-repeat-song-symbolic",
            Self::All => "media-playlist-repeat-symbolic",
            Self::None => "media-playlist-no-repeat-symbolic",
        }
    }
}

/// Persistent configuration stored via cosmic-config.
#[derive(Debug, Clone, CosmicConfigEntry, PartialEq)]
#[version = 1]
pub struct Config {
    /// Music library directories to scan.
    pub music_dirs: Vec<PathBuf>,
    /// Master volume (0.0 - 1.0).
    pub volume: f32,
    /// Whether shuffle is enabled.
    pub shuffle: bool,
    /// Repeat mode for playback queue.
    pub repeat_mode: RepeatMode,
    /// 10-band equalizer gains in dB (-12.0 to +12.0).
    pub equalizer_bands: Vec<f32>,
    /// Whether the equalizer is enabled.
    pub equalizer_enabled: bool,
    /// Last active view ("albums", "artists", "songs", "playlists").
    pub last_view: String,
}

impl Default for Config {
    fn default() -> Self {
        // Default music directory is ~/Music
        let music_dir = dirs::audio_dir().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join("Music")
        });

        Self {
            music_dirs: vec![music_dir],
            volume: 0.8,
            shuffle: false,
            repeat_mode: RepeatMode::None,
            // 10-band EQ: 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz
            equalizer_bands: vec![0.0; 10],
            equalizer_enabled: false,
            last_view: "albums".to_string(),
        }
    }
}
