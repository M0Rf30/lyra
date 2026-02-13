// SPDX-License-Identifier: GPL-3.0

//! Local filesystem music provider.
//!
//! Wraps the existing library scanning, database, cover art, and lyrics
//! functionality behind the [`MusicProvider`] trait.

use super::{MusicProvider, ProviderError, ProviderType};
use crate::library::{
    Album, Artist, CoverArt, LibraryDb, LibraryScanner, LyricsProvider, Track, TrackSource,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// A music provider backed by local filesystem scanning and SQLite storage.
///
/// Wraps `LibraryDb` in a `Mutex` because `rusqlite::Connection` is not `Sync`,
/// but the `MusicProvider` trait requires `Send + Sync`.
pub struct LocalProvider {
    db: Mutex<LibraryDb>,
    music_dirs: Vec<PathBuf>,
}

impl LocalProvider {
    /// Create a new local provider.
    pub fn new(db: LibraryDb, music_dirs: Vec<PathBuf>) -> Self {
        Self {
            db: Mutex::new(db),
            music_dirs,
        }
    }

    /// Lock the database, returning a ProviderError on poisoned mutex.
    fn lock_db(&self) -> Result<std::sync::MutexGuard<'_, LibraryDb>, ProviderError> {
        self.db
            .lock()
            .map_err(|e| ProviderError::Database(format!("DB lock poisoned: {e}")))
    }
}

impl MusicProvider for LocalProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn name(&self) -> &str {
        "Local Music"
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Local
    }

    fn browse_albums(&self) -> Result<Vec<Album>, ProviderError> {
        let db = self.lock_db()?;
        db.all_albums(Some("local"))
            .map_err(ProviderError::Database)
    }

    fn browse_artists(&self) -> Result<Vec<Artist>, ProviderError> {
        let db = self.lock_db()?;
        db.all_artists(Some("local"))
            .map_err(ProviderError::Database)
    }

    fn browse_tracks(&self) -> Result<Vec<Track>, ProviderError> {
        let db = self.lock_db()?;
        db.all_tracks(Some("local"))
            .map_err(ProviderError::Database)
    }

    fn search(&self, _query: &str) -> Result<Vec<Track>, ProviderError> {
        // TODO: Implement SQL LIKE search across title, artist, album.
        // For now, return all tracks (search filtering will be added in a later phase).
        self.browse_tracks()
    }

    fn resolve_audio(&self, track: &Track) -> Result<TrackSource, ProviderError> {
        Ok(TrackSource::LocalFile(PathBuf::from(&track.source_uri)))
    }

    fn get_cover_art(&self, album: &Album) -> Result<Option<Vec<u8>>, ProviderError> {
        let bytes = album
            .tracks
            .first()
            .and_then(|t| CoverArt::get_cover_art(&t.path));
        Ok(bytes)
    }

    fn get_lyrics(&self, track: &Track) -> Result<Option<String>, ProviderError> {
        // Try embedded tags first, then .lrc sidecar file.
        // Online LRCLIB fetch is async and handled separately by the app.
        let lyrics = LyricsProvider::from_tags(&track.path)
            .or_else(|| LyricsProvider::from_lrc_file(&track.path));
        Ok(lyrics)
    }

    fn sync_library(&self) -> Result<usize, ProviderError> {
        let db = self.lock_db()?;
        LibraryScanner::scan(&db, &self.music_dirs).map_err(ProviderError::Database)
    }
}
