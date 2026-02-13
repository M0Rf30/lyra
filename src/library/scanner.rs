// SPDX-License-Identifier: GPL-3.0

//! Scans directories for audio files and extracts metadata via lofty.

use super::{db::LibraryDb, Track};
use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Supported audio file extensions.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wma", "ape", "wv",
];

/// Scans music directories and populates the library database.
pub struct LibraryScanner;

impl LibraryScanner {
    /// Scan the given directories and upsert tracks into the database.
    /// Returns the number of new or updated tracks.
    pub fn scan(db: &LibraryDb, dirs: &[PathBuf]) -> Result<usize, String> {
        let mut count = 0;

        for dir in dirs {
            if !dir.exists() {
                log::warn!("Music directory does not exist: {}", dir.display());
                continue;
            }

            for entry in WalkDir::new(dir)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                if !Self::is_audio_file(path) {
                    continue;
                }

                // Check if we need to rescan based on mtime
                let mtime = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                let path_str = path.to_string_lossy();
                if let Some(existing_mtime) = db.get_track_mtime(&path_str) {
                    if existing_mtime == mtime {
                        continue; // File hasn't changed
                    }
                }

                match Self::read_metadata(path) {
                    Ok(track) => {
                        if let Err(e) = db.upsert_track(&track, mtime) {
                            log::error!("Failed to insert track {}: {e}", path.display());
                        } else {
                            count += 1;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to read metadata for {}: {e}", path.display());
                    }
                }
            }
        }

        // Clean up tracks that no longer exist
        if let Ok(removed) = db.remove_missing_tracks() {
            if removed > 0 {
                log::info!("Removed {removed} missing tracks from library");
            }
        }

        Ok(count)
    }

    /// Check if a path is a supported audio file.
    fn is_audio_file(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
    }

    /// Read metadata from an audio file using lofty.
    fn read_metadata(path: &Path) -> Result<Track, String> {
        let tagged_file = Probe::open(path)
            .map_err(|e| format!("Cannot open: {e}"))?
            .read()
            .map_err(|e| format!("Cannot read: {e}"))?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let properties = tagged_file.properties();
        let duration = properties.duration();

        let (title, artist, album_artist, album, genre, track_number, disc_number, year) =
            if let Some(tag) = tag {
                (
                    tag.title().map(|s| s.to_string()).unwrap_or_default(),
                    tag.artist().map(|s| s.to_string()).unwrap_or_default(),
                    tag.get_string(&ItemKey::AlbumArtist)
                        .unwrap_or_default()
                        .to_string(),
                    tag.album().map(|s| s.to_string()).unwrap_or_default(),
                    tag.genre().map(|s| s.to_string()).unwrap_or_default(),
                    tag.track().unwrap_or(0),
                    tag.disk().unwrap_or(0),
                    tag.year().unwrap_or(0),
                )
            } else {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    0,
                    0,
                    0,
                )
            };

        // Use filename as title if tag is missing
        let title = if title.is_empty() {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string()
        } else {
            title
        };

        // Fall back album_artist -> artist
        let album_artist = if album_artist.is_empty() {
            artist.clone()
        } else {
            album_artist
        };

        Ok(Track {
            id: 0,
            path: path.to_path_buf(),
            title,
            artist,
            album_artist,
            album,
            genre,
            track_number,
            disc_number,
            year,
            duration,
            bitrate: properties.audio_bitrate().unwrap_or(0),
            sample_rate: properties.sample_rate().unwrap_or(0),
        })
    }
}
