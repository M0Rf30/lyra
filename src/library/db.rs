// SPDX-License-Identifier: GPL-3.0

//! SQLite-backed music library database.

use super::{Album, Artist, Track};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Base schema for the tracks table.
const SCHEMA_BASE: &str = "
    CREATE TABLE IF NOT EXISTS tracks (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        path         TEXT NOT NULL UNIQUE,
        title        TEXT NOT NULL DEFAULT '',
        artist       TEXT NOT NULL DEFAULT '',
        album_artist TEXT NOT NULL DEFAULT '',
        album        TEXT NOT NULL DEFAULT '',
        genre        TEXT NOT NULL DEFAULT '',
        track_number INTEGER NOT NULL DEFAULT 0,
        disc_number  INTEGER NOT NULL DEFAULT 0,
        year         INTEGER NOT NULL DEFAULT 0,
        duration_ms  INTEGER NOT NULL DEFAULT 0,
        bitrate      INTEGER NOT NULL DEFAULT 0,
        sample_rate  INTEGER NOT NULL DEFAULT 0,
        mtime        INTEGER NOT NULL DEFAULT 0
    );

    CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
    CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
    CREATE INDEX IF NOT EXISTS idx_tracks_album_artist ON tracks(album_artist);

    CREATE TABLE IF NOT EXISTS cover_cache (
        album_key TEXT PRIMARY KEY,
        image_data BLOB
    );
";

/// The music library database.
pub struct LibraryDb {
    conn: Connection,
}

impl LibraryDb {
    /// Open or create the database at the given path.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("DB open error: {e}"))?;
        conn.execute_batch(SCHEMA_BASE)
            .map_err(|e| format!("DB init error: {e}"))?;

        let db = Self { conn };
        db.run_migration()?;
        Ok(db)
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("DB error: {e}"))?;
        conn.execute_batch(SCHEMA_BASE)
            .map_err(|e| format!("DB init error: {e}"))?;

        let db = Self { conn };
        db.run_migration()?;
        Ok(db)
    }

    /// Run idempotent migration: add provider columns if missing.
    fn run_migration(&self) -> Result<(), String> {
        let has_provider = self.column_exists("tracks", "provider")?;
        if has_provider {
            return Ok(());
        }

        log::info!("Running database migration: adding provider columns");

        self.conn
            .execute_batch(
                "ALTER TABLE tracks ADD COLUMN provider TEXT NOT NULL DEFAULT 'local';
                 ALTER TABLE tracks ADD COLUMN provider_track_id TEXT NOT NULL DEFAULT '';",
            )
            .map_err(|e| format!("Migration error (add columns): {e}"))?;

        self.conn
            .execute(
                "UPDATE tracks SET provider_track_id = path WHERE provider_track_id = ''",
                [],
            )
            .map_err(|e| format!("Migration error (backfill): {e}"))?;

        self.conn
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_tracks_provider_id
                     ON tracks(provider, provider_track_id);
                 CREATE INDEX IF NOT EXISTS idx_tracks_provider ON tracks(provider);",
            )
            .map_err(|e| format!("Migration error (indexes): {e}"))?;

        log::info!("Database migration complete");
        Ok(())
    }

    /// Check if a column exists in a table using PRAGMA table_info.
    fn column_exists(&self, table: &str, column: &str) -> Result<bool, String> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| format!("PRAGMA error: {e}"))?;

        let exists = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("PRAGMA query error: {e}"))?
            .filter_map(|r| r.ok())
            .any(|name| name == column);

        Ok(exists)
    }

    /// Insert or update a track.
    pub fn upsert_track(&self, track: &Track, mtime: i64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO tracks (path, title, artist, album_artist, album, genre,
                 track_number, disc_number, year, duration_ms, bitrate, sample_rate, mtime,
                 provider, provider_track_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(path) DO UPDATE SET
                    title=excluded.title, artist=excluded.artist,
                    album_artist=excluded.album_artist, album=excluded.album,
                    genre=excluded.genre, track_number=excluded.track_number,
                    disc_number=excluded.disc_number, year=excluded.year,
                    duration_ms=excluded.duration_ms, bitrate=excluded.bitrate,
                    sample_rate=excluded.sample_rate, mtime=excluded.mtime,
                    provider=excluded.provider, provider_track_id=excluded.provider_track_id",
                params![
                    track.path.to_string_lossy().as_ref(),
                    track.title,
                    track.artist,
                    track.album_artist,
                    track.album,
                    track.genre,
                    track.track_number,
                    track.disc_number,
                    track.year,
                    track.duration.as_millis() as i64,
                    track.bitrate,
                    track.sample_rate,
                    mtime,
                    track.provider_id,
                    track.source_uri,
                ],
            )
            .map_err(|e| format!("Upsert error: {e}"))?;
        Ok(())
    }

    /// Remove tracks whose paths no longer exist on disk (local provider only).
    pub fn remove_missing_tracks(&self) -> Result<usize, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM tracks WHERE provider = 'local'")
            .map_err(|e| format!("Query error: {e}"))?;

        let missing: Vec<i64> = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let path: String = row.get(1)?;
                Ok((id, path))
            })
            .map_err(|e| format!("Query error: {e}"))?
            .filter_map(|r| r.ok())
            .filter(|(_, path)| !Path::new(path).exists())
            .map(|(id, _)| id)
            .collect();

        let count = missing.len();
        for id in &missing {
            self.conn
                .execute("DELETE FROM tracks WHERE id = ?1", params![id])
                .ok();
        }

        Ok(count)
    }

    /// Remove all tracks for a given provider (used when a provider is removed from config).
    pub fn remove_provider_tracks(&self, provider_id: &str) -> Result<usize, String> {
        let count = self
            .conn
            .execute(
                "DELETE FROM tracks WHERE provider = ?1",
                params![provider_id],
            )
            .map_err(|e| format!("Delete error: {e}"))?;
        Ok(count)
    }

    /// Get all tracks, optionally filtered by provider, ordered by album then track number.
    pub fn all_tracks(&self, provider: Option<&str>) -> Result<Vec<Track>, String> {
        let (sql, param): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id
                 FROM tracks
                 WHERE provider = ?1
                 ORDER BY album_artist, album, disc_number, track_number"
                    .to_string(),
                vec![Box::new(p.to_string())],
            ),
            None => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id
                 FROM tracks
                 ORDER BY album_artist, album, disc_number, track_number"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("Query error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param.iter().map(|p| p.as_ref()).collect();

        let tracks = stmt
            .query_map(params_ref.as_slice(), Self::row_to_track)
            .map_err(|e| format!("Query error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    /// Get all albums (grouped from tracks), optionally filtered by provider.
    pub fn all_albums(&self, provider: Option<&str>) -> Result<Vec<Album>, String> {
        let tracks = self.all_tracks(provider)?;
        let mut albums: Vec<Album> = Vec::new();

        for track in tracks {
            if let Some(album) = albums.iter_mut().find(|a| {
                a.name.as_str() == track.album.as_str()
                    && a.artist.as_str() == track.album_artist.as_str()
            }) {
                album.tracks.push(track);
            } else {
                albums.push(Album {
                    name: track.album.clone(),
                    artist: track.album_artist.clone(),
                    year: track.year,
                    cover_source: None,
                    tracks: vec![track],
                });
            }
        }

        albums.sort_by(|a, b| a.artist.cmp(&b.artist).then(a.year.cmp(&b.year)));
        Ok(albums)
    }

    /// Get all artists (grouped from albums), optionally filtered by provider.
    pub fn all_artists(&self, provider: Option<&str>) -> Result<Vec<Artist>, String> {
        let albums = self.all_albums(provider)?;
        let mut artists: Vec<Artist> = Vec::new();

        for album in albums {
            if let Some(artist) = artists.iter_mut().find(|a| a.name == album.artist) {
                artist.albums.push(album);
            } else {
                let name = album.artist.clone();
                artists.push(Artist {
                    name,
                    albums: vec![album],
                });
            }
        }

        artists.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(artists)
    }

    /// Cache cover art for an album. Key includes provider to avoid collisions.
    pub fn cache_cover(&self, album_key: &str, data: &[u8]) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO cover_cache (album_key, image_data) VALUES (?1, ?2)",
                params![album_key, data],
            )
            .map_err(|e| format!("Cover cache error: {e}"))?;
        Ok(())
    }

    /// Retrieve cached cover art.
    pub fn get_cached_cover(&self, album_key: &str) -> Option<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT image_data FROM cover_cache WHERE album_key = ?1",
                params![album_key],
                |row| row.get(0),
            )
            .ok()
    }

    /// Get track mtime to determine if rescan is needed.
    pub fn get_track_mtime(&self, path: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT mtime FROM tracks WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok()
    }

    /// Count total tracks, optionally filtered by provider.
    pub fn track_count(&self, provider: Option<&str>) -> usize {
        match provider {
            Some(p) => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM tracks WHERE provider = ?1",
                    params![p],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0) as usize,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM tracks", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0) as usize,
        }
    }

    /// Map a database row to a Track struct.
    fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
        let path_str: String = row.get(1)?;
        Ok(Track {
            id: row.get(0)?,
            path: PathBuf::from(&path_str),
            title: row.get(2)?,
            artist: row.get(3)?,
            album_artist: row.get(4)?,
            album: row.get(5)?,
            genre: row.get(6)?,
            track_number: row.get(7)?,
            disc_number: row.get(8)?,
            year: row.get(9)?,
            duration: Duration::from_millis(row.get::<_, i64>(10)? as u64),
            bitrate: row.get(11)?,
            sample_rate: row.get(12)?,
            provider_id: row.get(13)?,
            source_uri: row.get(14)?,
        })
    }
}
