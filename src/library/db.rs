// SPDX-License-Identifier: GPL-3.0

//! SQLite-backed music library database.

use super::{Album, Artist, Track};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    CREATE INDEX IF NOT EXISTS idx_tracks_album_grouping ON tracks(album_artist, album, disc_number, track_number);

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

        // Performance-critical PRAGMAs — set before schema creation.
        // WAL enables concurrent reads during background scans;
        // synchronous=NORMAL is safe with WAL and reduces fsync overhead;
        // cache_size=-8000 gives an 8 MB page cache.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;",
        )
        .map_err(|e| format!("DB PRAGMA error: {e}"))?;

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
        if !has_provider {
            tracing::info!("Running database migration: adding provider columns");

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

            tracing::info!("Database migration complete");
        }

        // --- v2 migration: favorites, ratings, replay gain, playlists ---
        let has_favorite = self.column_exists("tracks", "is_favorite")?;
        if !has_favorite {
            tracing::info!(
                "Running database migration v2: adding favorites, ratings, replay gain, playlists"
            );

            self.conn
                .execute_batch(
                    "ALTER TABLE tracks ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE tracks ADD COLUMN rating INTEGER;
                     ALTER TABLE tracks ADD COLUMN rg_track_gain REAL;
                     ALTER TABLE tracks ADD COLUMN rg_album_gain REAL;

                     CREATE TABLE IF NOT EXISTS playlists (
                         id         INTEGER PRIMARY KEY AUTOINCREMENT,
                         name       TEXT NOT NULL,
                         created_at TEXT NOT NULL DEFAULT (datetime('now'))
                     );

                     CREATE TABLE IF NOT EXISTS playlist_tracks (
                         playlist_id INTEGER NOT NULL,
                         track_id    INTEGER NOT NULL,
                         position    INTEGER NOT NULL,
                         FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                         FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
                     );

                     CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist
                         ON playlist_tracks(playlist_id, position);",
                )
                .map_err(|e| format!("Migration v2 error: {e}"))?;

            tracing::info!("Database migration v2 complete");
        }

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
    #[tracing::instrument(skip(self, track), level = "debug")]
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
                    &*track.provider_id,
                    track.source_uri,
                ],
            )
            .map_err(|e| format!("Upsert error: {e}"))?;
        Ok(())
    }

    /// Remove tracks whose paths no longer exist on disk (local provider only).
    ///
    /// Uses batched `DELETE ... WHERE id IN (...)` inside a transaction for
    /// efficiency instead of one DELETE per row.
    #[tracing::instrument(skip(self), level = "debug")]
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
        if count == 0 {
            return Ok(0);
        }

        // Batch deletes in chunks of 999 (SQLite variable limit) inside a transaction.
        self.conn
            .execute_batch("BEGIN")
            .map_err(|e| format!("Transaction begin error: {e}"))?;

        // SQLite max variable number is 999 by default.
        for chunk in missing.chunks(999) {
            let placeholders: String = (1..=chunk.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM tracks WHERE id IN ({placeholders})");
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            self.conn
                .execute(&sql, params.as_slice())
                .map_err(|e| format!("Batch delete error: {e}"))?;
        }

        self.conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("Transaction commit error: {e}"))?;

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
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn all_tracks(&self, provider: Option<&str>) -> Result<Vec<Track>, String> {
        let (sql, param): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE provider = ?1
                 ORDER BY album_artist, album, disc_number, track_number"
                    .to_string(),
                vec![Box::new(p.to_string())],
            ),
            None => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
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
    ///
    /// Uses a HashMap index for O(1) grouping instead of linear search.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn all_albums(&self, provider: Option<&str>) -> Result<Vec<Album>, String> {
        let tracks = self.all_tracks(provider)?;

        if tracks.is_empty() {
            return Ok(Vec::new());
        }

        let mut albums: Vec<Album> = Vec::with_capacity(tracks.len() / 10);
        let mut index: HashMap<(String, String), usize> = HashMap::with_capacity(tracks.len() / 10);

        for track in tracks {
            let key = (track.album.clone(), track.album_artist.clone());
            if let Some(&idx) = index.get(&key) {
                albums[idx].tracks.push(track);
            } else {
                let idx = albums.len();
                index.insert(key, idx);
                albums.push(Album {
                    name: track.album.clone(),
                    artist: track.album_artist.clone(),
                    year: track.year,
                    cover_source: None,
                    tracks: vec![track],
                });
            }
        }

        albums.sort_unstable_by(|a, b| a.artist.cmp(&b.artist).then(a.year.cmp(&b.year)));
        Ok(albums)
    }

    /// Get all artists (grouped from albums), optionally filtered by provider.
    ///
    /// Uses a HashMap index for O(1) grouping instead of linear search.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn all_artists(&self, provider: Option<&str>) -> Result<Vec<Artist>, String> {
        let albums = self.all_albums(provider)?;

        if albums.is_empty() {
            return Ok(Vec::new());
        }

        let mut artists: Vec<Artist> = Vec::with_capacity(albums.len() / 3);
        let mut index: HashMap<String, usize> = HashMap::with_capacity(albums.len() / 3);

        for album in albums {
            if let Some(&idx) = index.get(&album.artist) {
                artists[idx].albums.push(album);
            } else {
                let idx = artists.len();
                let name = album.artist.clone();
                index.insert(name.clone(), idx);
                artists.push(Artist {
                    name,
                    albums: vec![album],
                });
            }
        }

        artists.sort_unstable_by(|a, b| a.name.cmp(&b.name));
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

    /// Remove a single track by its file path.
    ///
    /// Used by the incremental filesystem-watcher scan to remove deleted files.
    pub fn remove_track_by_path(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM tracks WHERE path = ?1", params![path])
            .map_err(|e| format!("Delete by path error: {e}"))?;
        Ok(())
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

    // ---- Search ----

    /// Search tracks by matching query against title, artist, album, or genre.
    pub fn search_tracks(&self, query: &str, provider: Option<&str>) -> Result<Vec<Track>, String> {
        let pattern = format!("%{query}%");
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE provider = ?1
                   AND (title LIKE ?2 OR artist LIKE ?2 OR album LIKE ?2 OR genre LIKE ?2)
                 ORDER BY
                   CASE WHEN title LIKE ?2 THEN 0 ELSE 1 END,
                   title"
                    .to_string(),
                vec![
                    Box::new(p.to_string()),
                    Box::new(pattern),
                ],
            ),
            None => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1 OR genre LIKE ?1
                 ORDER BY
                   CASE WHEN title LIKE ?1 THEN 0 ELSE 1 END,
                   title"
                    .to_string(),
                vec![Box::new(pattern)],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("Search query error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let tracks = stmt
            .query_map(params_ref.as_slice(), Self::row_to_track)
            .map_err(|e| format!("Search query error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    // ---- Playlists ----

    /// List all playlists.
    pub fn list_playlists(&self) -> Result<Vec<super::Playlist>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name,
                        COUNT(pt.track_id) as track_count,
                        COALESCE(SUM(t.duration_ms), 0) as total_duration_ms
                 FROM playlists p
                 LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
                 LEFT JOIN tracks t ON t.id = pt.track_id
                 GROUP BY p.id
                 ORDER BY p.name",
            )
            .map_err(|e| format!("List playlists error: {e}"))?;

        let playlists = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let track_count: u32 = row.get(2)?;
                let total_ms: i64 = row.get(3)?;
                Ok(super::Playlist {
                    id: id.to_string(),
                    name,
                    tracks: Vec::new(),
                    track_count,
                    total_duration: Duration::from_millis(total_ms as u64),
                })
            })
            .map_err(|e| format!("List playlists error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(playlists)
    }

    /// Get a playlist with its tracks.
    pub fn get_playlist(&self, playlist_id: &str) -> Result<super::Playlist, String> {
        let id: i64 = playlist_id
            .parse()
            .map_err(|_| "Invalid playlist ID".to_string())?;

        let (name, created_at): (String, String) = self
            .conn
            .query_row(
                "SELECT name, created_at FROM playlists WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| format!("Get playlist error: {e}"))?;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.id, t.path, t.title, t.artist, t.album_artist, t.album, t.genre,
                        t.track_number, t.disc_number, t.year, t.duration_ms, t.bitrate, t.sample_rate,
                        t.provider, t.provider_track_id, t.is_favorite, t.rating, t.rg_track_gain, t.rg_album_gain
                 FROM playlist_tracks pt
                 JOIN tracks t ON t.id = pt.track_id
                 WHERE pt.playlist_id = ?1
                 ORDER BY pt.position",
            )
            .map_err(|e| format!("Get playlist tracks error: {e}"))?;

        let tracks: Vec<Track> = stmt
            .query_map(params![id], Self::row_to_track)
            .map_err(|e| format!("Get playlist tracks error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        let total_duration = tracks.iter().map(|t| t.duration).sum();
        let track_count = tracks.len() as u32;
        let _ = created_at; // available if needed later

        Ok(super::Playlist {
            id: playlist_id.to_string(),
            name,
            tracks,
            track_count,
            total_duration,
        })
    }

    /// Create a new playlist.
    pub fn create_playlist(&self, name: &str) -> Result<super::Playlist, String> {
        self.conn
            .execute("INSERT INTO playlists (name) VALUES (?1)", params![name])
            .map_err(|e| format!("Create playlist error: {e}"))?;

        let id = self.conn.last_insert_rowid();
        Ok(super::Playlist {
            id: id.to_string(),
            name: name.to_string(),
            tracks: Vec::new(),
            track_count: 0,
            total_duration: Duration::ZERO,
        })
    }

    /// Delete a playlist.
    pub fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
        let id: i64 = playlist_id
            .parse()
            .map_err(|_| "Invalid playlist ID".to_string())?;

        // CASCADE handles playlist_tracks deletion.
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", params![id])
            .map_err(|e| format!("Delete playlist error: {e}"))?;
        Ok(())
    }

    /// Rename a playlist.
    pub fn rename_playlist(&self, playlist_id: &str, new_name: &str) -> Result<(), String> {
        let id: i64 = playlist_id
            .parse()
            .map_err(|_| "Invalid playlist ID".to_string())?;

        self.conn
            .execute(
                "UPDATE playlists SET name = ?1 WHERE id = ?2",
                params![new_name, id],
            )
            .map_err(|e| format!("Rename playlist error: {e}"))?;
        Ok(())
    }

    /// Add tracks to a playlist by their IDs.
    pub fn add_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<(), String> {
        let pid: i64 = playlist_id
            .parse()
            .map_err(|_| "Invalid playlist ID".to_string())?;

        // Get current max position.
        let max_pos: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![pid],
                |row| row.get(0),
            )
            .unwrap_or(-1);

        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| format!("Add to playlist prepare error: {e}"))?;

        for (i, tid) in track_ids.iter().enumerate() {
            let track_id: i64 = tid
                .parse()
                .map_err(|_| format!("Invalid track ID: {tid}"))?;
            stmt.execute(params![pid, track_id, max_pos + 1 + i as i64])
                .map_err(|e| format!("Add to playlist error: {e}"))?;
        }
        Ok(())
    }

    // ---- Favorites and Ratings ----

    /// Toggle the favorite status of a track. Returns the new status.
    pub fn toggle_favorite(&self, track_id: i64) -> Result<bool, String> {
        self.conn
            .execute(
                "UPDATE tracks SET is_favorite = CASE WHEN is_favorite = 0 THEN 1 ELSE 0 END
                 WHERE id = ?1",
                params![track_id],
            )
            .map_err(|e| format!("Toggle favorite error: {e}"))?;

        let new_val: bool = self
            .conn
            .query_row(
                "SELECT is_favorite FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get::<_, i32>(0).map(|v| v != 0),
            )
            .map_err(|e| format!("Toggle favorite read error: {e}"))?;

        Ok(new_val)
    }

    /// Check if a track is a favorite.
    pub fn is_favorite(&self, track_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT is_favorite FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get::<_, i32>(0).map(|v| v != 0),
            )
            .map_err(|e| format!("Is favorite error: {e}"))
    }

    /// Set a rating (1-5) for a track. Pass 0 to clear.
    pub fn set_rating(&self, track_id: i64, rating: u8) -> Result<(), String> {
        let val: Option<u8> = if rating == 0 { None } else { Some(rating) };
        self.conn
            .execute(
                "UPDATE tracks SET rating = ?1 WHERE id = ?2",
                params![val, track_id],
            )
            .map_err(|e| format!("Set rating error: {e}"))?;
        Ok(())
    }

    /// Get the rating for a track.
    pub fn get_rating(&self, track_id: i64) -> Result<Option<u8>, String> {
        self.conn
            .query_row(
                "SELECT rating FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Get rating error: {e}"))
    }

    /// List all favorite tracks.
    pub fn list_favorites(&self, provider: Option<&str>) -> Result<Vec<Track>, String> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE provider = ?1 AND is_favorite = 1
                 ORDER BY title"
                    .to_string(),
                vec![Box::new(p.to_string())],
            ),
            None => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE is_favorite = 1
                 ORDER BY title"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("List favorites error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let tracks = stmt
            .query_map(params_ref.as_slice(), Self::row_to_track)
            .map_err(|e| format!("List favorites error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    // ---- Genres ----

    /// List all distinct genres.
    pub fn list_genres(&self, provider: Option<&str>) -> Result<Vec<String>, String> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT DISTINCT genre FROM tracks
                 WHERE provider = ?1 AND genre != ''
                 ORDER BY genre"
                    .to_string(),
                vec![Box::new(p.to_string())],
            ),
            None => (
                "SELECT DISTINCT genre FROM tracks
                 WHERE genre != ''
                 ORDER BY genre"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("List genres error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let genres = stmt
            .query_map(params_ref.as_slice(), |row| row.get(0))
            .map_err(|e| format!("List genres error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(genres)
    }

    /// Get all tracks matching a genre.
    pub fn tracks_by_genre(
        &self,
        genre: &str,
        provider: Option<&str>,
    ) -> Result<Vec<Track>, String> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match provider {
            Some(p) => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE provider = ?1 AND genre = ?2
                 ORDER BY album_artist, album, disc_number, track_number"
                    .to_string(),
                vec![Box::new(p.to_string()), Box::new(genre.to_string())],
            ),
            None => (
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate,
                        provider, provider_track_id, is_favorite, rating, rg_track_gain, rg_album_gain
                 FROM tracks
                 WHERE genre = ?1
                 ORDER BY album_artist, album, disc_number, track_number"
                    .to_string(),
                vec![Box::new(genre.to_string())],
            ),
        };

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("Tracks by genre error: {e}"))?;

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let tracks = stmt
            .query_map(params_ref.as_slice(), Self::row_to_track)
            .map_err(|e| format!("Tracks by genre error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
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
            provider_id: Arc::from(row.get::<_, String>(13)?),
            source_uri: row.get(14)?,
            is_favorite: row.get::<_, i32>(15).unwrap_or(0) != 0,
            rating: row.get::<_, Option<u8>>(16).unwrap_or(None),
            rg_track_gain: row.get::<_, Option<f32>>(17).unwrap_or(None),
            rg_album_gain: row.get::<_, Option<f32>>(18).unwrap_or(None),
        })
    }
}
