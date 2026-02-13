// SPDX-License-Identifier: GPL-3.0

//! SQLite-backed music library database.

use super::{Album, Artist, Track};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The music library database.
pub struct LibraryDb {
    conn: Connection,
}

impl LibraryDb {
    /// Open or create the database at the given path.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("DB open error: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
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
            );",
        )
        .map_err(|e| format!("DB init error: {e}"))?;

        Ok(Self { conn })
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("DB error: {e}"))?;
        let db = Self { conn };
        // Run the same schema
        db.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS tracks (
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

            CREATE TABLE IF NOT EXISTS cover_cache (
                album_key TEXT PRIMARY KEY,
                image_data BLOB
            );",
            )
            .map_err(|e| format!("DB init error: {e}"))?;
        Ok(db)
    }

    /// Insert or update a track.
    pub fn upsert_track(&self, track: &Track, mtime: i64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO tracks (path, title, artist, album_artist, album, genre,
                 track_number, disc_number, year, duration_ms, bitrate, sample_rate, mtime)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(path) DO UPDATE SET
                    title=excluded.title, artist=excluded.artist,
                    album_artist=excluded.album_artist, album=excluded.album,
                    genre=excluded.genre, track_number=excluded.track_number,
                    disc_number=excluded.disc_number, year=excluded.year,
                    duration_ms=excluded.duration_ms, bitrate=excluded.bitrate,
                    sample_rate=excluded.sample_rate, mtime=excluded.mtime",
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
                ],
            )
            .map_err(|e| format!("Upsert error: {e}"))?;
        Ok(())
    }

    /// Remove tracks whose paths no longer exist on disk.
    pub fn remove_missing_tracks(&self) -> Result<usize, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM tracks")
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

    /// Get all tracks, ordered by album then track number.
    pub fn all_tracks(&self) -> Result<Vec<Track>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, title, artist, album_artist, album, genre,
                        track_number, disc_number, year, duration_ms, bitrate, sample_rate
                 FROM tracks
                 ORDER BY album_artist, album, disc_number, track_number",
            )
            .map_err(|e| format!("Query error: {e}"))?;

        let tracks = stmt
            .query_map([], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
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
                })
            })
            .map_err(|e| format!("Query error: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    /// Get all albums (grouped from tracks).
    pub fn all_albums(&self) -> Result<Vec<Album>, String> {
        let tracks = self.all_tracks()?;
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
                    cover_path: None,
                    tracks: vec![track],
                });
            }
        }

        albums.sort_by(|a, b| a.artist.cmp(&b.artist).then(a.year.cmp(&b.year)));
        Ok(albums)
    }

    /// Get all artists (grouped from albums).
    pub fn all_artists(&self) -> Result<Vec<Artist>, String> {
        let albums = self.all_albums()?;
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

    /// Cache cover art for an album.
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

    /// Count total tracks.
    pub fn track_count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }
}
