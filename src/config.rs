// SPDX-License-Identifier: GPL-3.0

use cosmic::cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Serializable configuration for an MPD server connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MpdConfigEntry {
    /// Unique provider ID (e.g., "mpd-home").
    pub id: String,
    /// Human-readable name (e.g., "Home MPD Server").
    pub name: String,
    /// MPD server hostname.
    pub host: String,
    /// MPD server port (default: 6600).
    #[serde(default = "default_mpd_port")]
    pub port: u16,
    /// Optional password for authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

fn default_mpd_port() -> u16 {
    6600
}

impl Default for MpdConfigEntry {
    fn default() -> Self {
        Self {
            id: "mpd".to_string(),
            name: "MPD Server".to_string(),
            host: "localhost".to_string(),
            port: 6600,
            password: None,
        }
    }
}

/// Serializable configuration for an OpenSubsonic/Navidrome server connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsonicConfigEntry {
    /// Unique provider ID (e.g., "subsonic-home").
    pub id: String,
    /// Human-readable name (e.g., "Navidrome").
    pub name: String,
    /// Server base URL (e.g., "https://music.example.com").
    pub url: String,
    /// Subsonic username.
    pub username: String,
    /// Password (stored as plaintext for now; keyring TODO).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Accept invalid TLS certificates (self-signed, Tailscale, etc.).
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

impl Default for SubsonicConfigEntry {
    fn default() -> Self {
        Self {
            id: "subsonic".to_string(),
            name: "Subsonic Server".to_string(),
            url: "https://music.example.com".to_string(),
            username: String::new(),
            password: None,
            accept_invalid_certs: false,
        }
    }
}

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
    /// Configured MPD server connections.
    pub mpd_servers: Vec<MpdConfigEntry>,
    /// Configured OpenSubsonic/Navidrome server connections.
    pub subsonic_servers: Vec<SubsonicConfigEntry>,
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
            mpd_servers: Vec::new(),
            subsonic_servers: Vec::new(),
        }
    }
}
