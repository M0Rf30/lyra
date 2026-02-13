// SPDX-License-Identifier: GPL-3.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};
use std::path::PathBuf;

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
    /// Repeat mode: "none", "one", "all".
    pub repeat_mode: String,
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
            repeat_mode: "none".to_string(),
            // 10-band EQ: 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz
            equalizer_bands: vec![0.0; 10],
            equalizer_enabled: false,
            last_view: "albums".to_string(),
        }
    }
}
