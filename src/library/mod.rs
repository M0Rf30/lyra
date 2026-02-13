// SPDX-License-Identifier: GPL-3.0

//! Music library: scanning, database, metadata, and cover art.

mod cover_art;
mod db;
mod lyrics;
mod scanner;

pub use cover_art::CoverArt;
pub use db::LibraryDb;
pub use lyrics::LyricsProvider;
pub use scanner::LibraryScanner;

use std::path::PathBuf;
use std::time::Duration;

/// A single track in the music library.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: i64,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    pub genre: String,
    pub track_number: u32,
    pub disc_number: u32,
    pub year: u32,
    pub duration: Duration,
    pub bitrate: u32,
    pub sample_rate: u32,
}

impl Track {
    /// Format duration as MM:SS.
    pub fn duration_string(&self) -> String {
        let secs = self.duration.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// An album aggregated from library tracks.
#[derive(Debug, Clone)]
pub struct Album {
    pub name: String,
    pub artist: String,
    pub year: u32,
    pub tracks: Vec<Track>,
    pub cover_path: Option<PathBuf>,
}

impl Album {
    /// Total duration of all tracks.
    pub fn total_duration(&self) -> Duration {
        self.tracks.iter().map(|t| t.duration).sum()
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }
}

/// An artist aggregated from library tracks.
#[derive(Debug, Clone)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<Album>,
}

impl Artist {
    pub fn album_count(&self) -> usize {
        self.albums.len()
    }

    pub fn track_count(&self) -> usize {
        self.albums.iter().map(|a| a.tracks.len()).sum()
    }
}
