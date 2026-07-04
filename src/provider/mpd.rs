// SPDX-License-Identifier: GPL-3.0

//! MPD server music provider.
//!
//! Connects to an MPD server using the `mpd_client` crate, browses its
//! library, controls remote playback, and receives real-time idle events.

use super::{MusicProvider, ProviderError, ProviderType};
use crate::library::{Album, Artist, CoverSource, Playlist, Track, TrackSource};
use mpd_client::Client;
use mpd_client::client::{ConnectWithPasswordError, ConnectionEvents};
use mpd_client::commands::{
    self, AddToPlaylist, Crossfade, CurrentSong, DeletePlaylist, Find, GetPlaylist, GetPlaylists,
    List, ReplayGainMode, SaveQueueAsPlaylist, SetRandom, SetRepeat, SetReplayGainMode, SetSingle,
    SingleMode, Status, StickerDelete, StickerFind, StickerGet, StickerSet, Update,
};
use mpd_client::filter::{Filter, Operator};
use mpd_client::responses;
use mpd_client::tag::Tag;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Helper to wrap MPD errors into `ProviderError::Io` with a labeled context.
fn mpd_err<E: std::fmt::Display>(op: &str) -> impl FnOnce(E) -> ProviderError + '_ {
    move |e| ProviderError::Io(format!("MPD {op}: {e}"))
}

/// Configuration for connecting to an MPD server.
#[derive(Debug, Clone)]
pub struct MpdConfig {
    /// Unique provider ID (e.g., "mpd-home").
    pub id: String,
    /// Human-readable name (e.g., "Home MPD Server").
    pub name: String,
    /// MPD server hostname.
    pub host: String,
    /// MPD server port (default: 6600).
    pub port: u16,
    /// Optional password for authentication.
    pub password: Option<String>,
}

impl Default for MpdConfig {
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

impl From<crate::config::MpdConfigEntry> for MpdConfig {
    fn from(entry: crate::config::MpdConfigEntry) -> Self {
        // Task 85/86: Retrieve password from keyring when password_in_keyring is set.
        let password = if entry.password_in_keyring {
            match crate::credentials::retrieve_password(&entry.id) {
                Ok(Some(pw)) => Some(pw),
                Ok(None) => {
                    // Task 86: password_in_keyring is true but no keyring entry found.
                    tracing::error!(
                        "MPD provider '{}': password marked as stored in keyring but not found. \
                         Please re-enter the password in provider settings.",
                        entry.id
                    );
                    None
                }
                Err(e) => {
                    tracing::error!(
                        "MPD provider '{}': failed to retrieve password from keyring: {e}",
                        entry.id
                    );
                    None
                }
            }
        } else {
            entry.password
        };

        Self {
            id: entry.id,
            name: entry.name,
            host: entry.host,
            port: entry.port,
            password,
        }
    }
}

/// MPD music provider backed by the `mpd_client` crate.
///
/// Uses **two separate TCP connections** to the same MPD server:
/// - **Idle connection**: owned by the COSMIC subscription, used solely
///   for receiving idle events via `ConnectionEvents`. This connection
///   stays in MPD's "idle" mode permanently.
/// - **Command connection**: stored in `self.client`, used for all
///   browse/search/playback commands. This connection is never in idle
///   mode, so commands execute without conflicting with idle events.
///
/// This dual-connection approach avoids the `invalid message` protocol
/// errors caused by multiplexing idle and command traffic on a single
/// connection.
pub struct MpdProvider {
    config: MpdConfig,
    /// Shared provider ID for zero-copy assignment to tracks.
    provider_id: Arc<str>,
    /// Command connection — used for browse, search, playback, etc.
    client: Arc<Mutex<Option<Client>>>,
    runtime: tokio::runtime::Handle,
    /// Whether MPD stickers are supported (detected on connect).
    stickers_supported: AtomicBool,
}

impl MpdProvider {
    /// Create a new MPD provider. Does NOT connect yet — call `connect_idle()` first.
    pub fn new(config: MpdConfig, runtime: tokio::runtime::Handle) -> Self {
        let provider_id: Arc<str> = Arc::from(config.id.as_str());
        Self {
            config,
            provider_id,
            client: Arc::new(Mutex::new(None)),
            runtime,
            stickers_supported: AtomicBool::new(false),
        }
    }

    /// Helper: open a fresh TCP+MPD connection, returning `(Client, ConnectionEvents)`.
    async fn open_connection(&self) -> Result<(Client, ConnectionEvents), ProviderError> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| ProviderError::NotConnected(format!("TCP connect to {addr}: {e}")))?;

        let (client, events) =
            Client::connect_with_password_opt(stream, self.config.password.as_deref())
                .await
                .map_err(|e| match e {
                    ConnectWithPasswordError::IncorrectPassword => {
                        ProviderError::NotConnected("Incorrect MPD password".into())
                    }
                    ConnectWithPasswordError::ProtocolError(pe) => {
                        ProviderError::NotConnected(format!("MPD protocol error: {pe}"))
                    }
                })?;

        tracing::info!(
            "Connected to MPD at {} (protocol {})",
            addr,
            client.protocol_version()
        );

        Ok((client, events))
    }

    /// Establish the **idle connection** and return its event stream.
    ///
    /// This connection is used only by the COSMIC subscription for
    /// receiving idle events. It does NOT store the client in `self.client`
    /// — that is done by `connect_command()`.
    ///
    /// Returns `(Client, ConnectionEvents)` — the caller **must** keep the
    /// `Client` alive for the duration of the idle loop, because dropping it
    /// signals the background task to exit (which closes `ConnectionEvents`).
    pub async fn connect_idle(&self) -> Result<(Client, ConnectionEvents), ProviderError> {
        self.open_connection().await
    }

    /// Establish the **command connection** and store it for browse/search/playback.
    ///
    /// Must be called after `connect_idle()` succeeds. The `ConnectionEvents`
    /// from this connection is intentionally dropped — we don't need idle
    /// events from the command connection.
    ///
    /// Also probes sticker support by attempting a harmless `sticker get`.
    pub async fn connect_command(&self) -> Result<(), ProviderError> {
        let (client, _events) = self.open_connection().await?;

        // Probe sticker support: try a harmless sticker get on a nonexistent URI.
        // If the server returns an error about unknown command, stickers are disabled.
        let stickers_ok = match client.command(StickerGet::new("", "probe")).await {
            Ok(_) => true,
            Err(e) => {
                let msg = format!("{e}");
                // "unknown command" means stickers disabled; any other error
                // (e.g., "no such sticker") means the command itself is supported.
                !msg.contains("unknown command")
            }
        };
        self.stickers_supported
            .store(stickers_ok, Ordering::Relaxed);
        tracing::info!("MPD sticker support: {stickers_ok}");

        let mut guard = self.client.lock().await;
        *guard = Some(client);
        Ok(())
    }

    /// Disconnect the command connection (e.g. before reconnecting).
    pub async fn disconnect(&self) {
        let mut guard = self.client.lock().await;
        *guard = None;
    }

    /// Get a clone of the command client, or error if not connected.
    async fn get_client(&self) -> Result<Client, ProviderError> {
        let guard = self.client.lock().await;
        guard
            .clone()
            .ok_or_else(|| ProviderError::NotConnected("Not connected to MPD".into()))
    }

    /// Get the `Client` for use in the `MpdBackend` (public for player module).
    pub fn client_clone(&self) -> Option<Client> {
        self.runtime.block_on(async {
            let guard = self.client.lock().await;
            guard.clone()
        })
    }

    /// Get a clone of the tokio runtime handle (for MpdBackend creation).
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.clone()
    }

    /// Run an async block on the tokio runtime (bridging sync MusicProvider to async mpd_client).
    fn block_on<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.runtime.block_on(future)
    }

    /// Get the full list of unique album names from MPD.
    ///
    /// This is a single fast command (`list album`). The returned names
    /// can then be chunked and passed to [`browse_albums_batch`] for
    /// incremental loading.
    pub async fn list_album_names(&self) -> Result<Vec<String>, ProviderError> {
        let client = self.get_client().await?;
        let album_list = client
            .command(List::new(Tag::Album))
            .await
            .map_err(mpd_err("list album"))?;

        Ok(album_list
            .into_iter()
            .filter(|name| !name.is_empty())
            .collect())
    }

    /// Fetch full album details (with songs) for a batch of album names.
    ///
    /// Returns a `Vec<Album>` built by issuing one `find album "<name>"`
    /// per album name. This is the building block for incremental library
    /// loading — call with a chunk of 20-50 album names at a time.
    pub async fn browse_albums_batch(
        &self,
        album_names: &[String],
    ) -> Result<Vec<Album>, ProviderError> {
        let client = self.get_client().await?;
        let mut albums = Vec::with_capacity(album_names.len());

        for album_name in album_names {
            let filter = Filter::tag(Tag::Album, album_name);
            let songs = client
                .command(Find::new(filter))
                .await
                .map_err(mpd_err("find"))?;

            if songs.is_empty() {
                continue;
            }

            let mut tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            Track::sort_by_disc_and_track(&mut tracks);

            let cover_source = tracks
                .first()
                .map(|t| CoverSource::MpdAlbumArt(t.source_uri.clone()));

            albums.push(Album::from_tracks(album_name.clone(), tracks, cover_source));
        }

        Ok(albums)
    }

    /// Query MPD status (async).
    pub async fn status_async(&self) -> Result<responses::Status, ProviderError> {
        let client = self.get_client().await?;
        client.command(Status).await.map_err(mpd_err("status"))
    }

    /// Query current song (async).
    pub async fn current_song_async(
        &self,
    ) -> Result<Option<responses::SongInQueue>, ProviderError> {
        let client = self.get_client().await?;
        client
            .command(CurrentSong)
            .await
            .map_err(mpd_err("currentsong"))
    }

    /// Convert an `mpd_client` Song to our Track model.
    fn song_to_track(&self, song: &responses::Song) -> Track {
        let title = song.title().unwrap_or("Unknown Title").to_string();
        let artist = song
            .artists()
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let album_artist = song
            .album_artists()
            .first()
            .cloned()
            .unwrap_or_else(|| artist.clone());
        let album = song.album().unwrap_or("Unknown Album").to_string();
        let genre = song
            .tags
            .get(&Tag::Genre)
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default();
        let (disc_num, track_num) = song.number();
        let year = song
            .tags
            .get(&Tag::Date)
            .and_then(|v| v.first())
            .and_then(|s| s.get(..4))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let duration = song.duration.unwrap_or(Duration::ZERO);
        let uri = song.url.clone();

        Track {
            id: 0,
            path: PathBuf::from(&uri),
            title,
            artist,
            album_artist,
            album,
            genre,
            track_number: track_num as u32,
            disc_number: disc_num as u32,
            year,
            duration,
            bitrate: 0,
            sample_rate: 0,
            provider_id: Arc::clone(&self.provider_id),
            source_uri: uri,
            is_favorite: false,
            rating: None,
            rg_track_gain: None,
            rg_album_gain: None,
        }
    }

    // --- MPD state control methods (Tasks 56-59) ---

    /// Send `random 0/1` to MPD.
    pub fn send_random(&self, enabled: bool) -> Result<(), ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(SetRandom(enabled))
                .await
                .map_err(mpd_err("random"))
        })
    }

    /// Send `repeat 0/1` to MPD.
    pub fn send_repeat(&self, enabled: bool) -> Result<(), ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(SetRepeat(enabled))
                .await
                .map_err(mpd_err("repeat"))
        })
    }

    /// Send `single 0/1/oneshot` to MPD.
    pub fn send_single(&self, mode: SingleMode) -> Result<(), ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(SetSingle(mode))
                .await
                .map_err(mpd_err("single"))
        })
    }

    /// Send `crossfade <seconds>` to MPD.
    pub fn send_crossfade(&self, seconds: u64) -> Result<(), ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(Crossfade(Duration::from_secs(seconds)))
                .await
                .map_err(mpd_err("crossfade"))
        })
    }

    /// Send `replay_gain_mode` to MPD.
    pub fn send_replay_gain_mode(&self, mode: ReplayGainMode) -> Result<(), ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(SetReplayGainMode(mode))
                .await
                .map_err(mpd_err("replay_gain_mode"))
        })
    }

    /// Read MPD status (sync). Returns the parsed status for reading
    /// `random`, `repeat`, `single`, `crossfade`, etc.
    pub fn status(&self) -> Result<responses::Status, ProviderError> {
        self.block_on(self.status_async())
    }
}

impl MusicProvider for MpdProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Mpd
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn browse_albums(&self) -> Result<Vec<Album>, ProviderError> {
        self.block_on(async {
            let names = self.list_album_names().await?;
            let mut albums = self.browse_albums_batch(&names).await?;
            albums.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(albums)
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn browse_artists(&self) -> Result<Vec<Artist>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            let artist_list = client
                .command(List::new(Tag::AlbumArtist))
                .await
                .map_err(mpd_err("list albumartist"))?;

            let artist_names: Vec<String> = artist_list.into_iter().collect();
            let mut artists = Vec::new();

            for artist_name in &artist_names {
                if artist_name.is_empty() {
                    continue;
                }

                // Get albums for this artist
                let filter = Filter::tag(Tag::AlbumArtist, artist_name);
                let album_list = client
                    .command(List::new(Tag::Album).filter(filter))
                    .await
                    .map_err(mpd_err("list album"))?;

                let album_names: Vec<String> = album_list.into_iter().collect();
                let mut artist_albums = Vec::new();

                for album_name in &album_names {
                    if album_name.is_empty() {
                        continue;
                    }

                    let filter = Filter::tag(Tag::Album, album_name)
                        .and(Filter::tag(Tag::AlbumArtist, artist_name));
                    let songs = client
                        .command(Find::new(filter))
                        .await
                        .map_err(mpd_err("find"))?;

                    let mut tracks: Vec<Track> =
                        songs.iter().map(|s| self.song_to_track(s)).collect();
                    Track::sort_by_disc_and_track(&mut tracks);

                    let cover_source = tracks
                        .first()
                        .map(|t| CoverSource::MpdAlbumArt(t.source_uri.clone()));

                    let mut album = Album::from_tracks(album_name.clone(), tracks, cover_source);
                    album.artist = artist_name.clone();
                    artist_albums.push(album);
                }

                artist_albums.sort_by_key(|a| a.year);

                artists.push(Artist {
                    name: artist_name.clone(),
                    albums: artist_albums,
                });
            }

            artists.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(artists)
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn browse_tracks(&self) -> Result<Vec<Track>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            // Use listallinfo to get all songs from the root
            let songs = client
                .command(commands::ListAllIn::root())
                .await
                .map_err(mpd_err("listallinfo"))?;

            let mut tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            tracks.sort_by(|a, b| a.title.cmp(&b.title));
            Ok(tracks)
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn search(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        let query_owned = query.to_string();
        self.block_on(async {
            let client = self.get_client().await?;

            // Use the `any` pseudo-tag with `contains` operator to search
            // across all metadata fields (Title, Artist, Album, etc.).
            let filter = Filter::new(Tag::any(), Operator::Contain, &query_owned);
            let songs = client
                .command(Find::new(filter))
                .await
                .map_err(mpd_err("search"))?;

            let tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            Ok(tracks)
        })
    }

    fn resolve_audio(&self, track: &Track) -> Result<TrackSource, ProviderError> {
        Ok(TrackSource::MpdFile(track.source_uri.clone()))
    }

    #[tracing::instrument(skip(self, album), level = "debug")]
    fn get_cover_art(&self, album: &Album) -> Result<Option<Vec<u8>>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            let uri = match album.tracks.first() {
                Some(t) => &t.source_uri,
                None => return Ok(None),
            };

            match client.album_art(uri).await {
                Ok(Some((data, _mime))) => Ok(Some(data.to_vec())),
                Ok(None) => Ok(None),
                Err(e) => {
                    tracing::warn!("Failed to get MPD album art for {uri}: {e}");
                    Ok(None)
                }
            }
        })
    }

    fn get_lyrics(&self, _track: &Track) -> Result<Option<crate::library::Lyrics>, ProviderError> {
        // MPD doesn't serve lyrics; the app falls back to LRCLIB.
        Ok(None)
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn sync_library(&self) -> Result<usize, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(Update::new())
                .await
                .map_err(mpd_err("update"))?;
            Ok(0)
        })
    }

    // --- Playlist methods (Tasks 47-51) ---

    #[tracing::instrument(skip(self), level = "debug")]
    fn list_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            let playlists = client
                .command(GetPlaylists)
                .await
                .map_err(mpd_err("listplaylists"))?;

            Ok(playlists
                .into_iter()
                .map(|p| Playlist {
                    id: p.name.clone(),
                    name: p.name,
                    tracks: Vec::new(),
                    track_count: 0,
                    total_duration: Duration::ZERO,
                })
                .collect())
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        let id_owned = id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            let songs = client
                .command(GetPlaylist(&id_owned))
                .await
                .map_err(mpd_err("listplaylistinfo"))?;

            let tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            let total_duration = tracks.iter().map(|t| t.duration).sum();
            let track_count = tracks.len() as u32;

            Ok(Playlist {
                id: id_owned.clone(),
                name: id_owned,
                tracks,
                track_count,
                total_duration,
            })
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn create_playlist(&self, name: &str) -> Result<Playlist, ProviderError> {
        let name_owned = name.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            // MPD `save` saves the current queue as a playlist.
            // We create an empty playlist by saving and then clearing it.
            client
                .command(SaveQueueAsPlaylist(&name_owned))
                .await
                .map_err(mpd_err("save"))?;

            Ok(Playlist {
                id: name_owned.clone(),
                name: name_owned,
                tracks: Vec::new(),
                track_count: 0,
                total_duration: Duration::ZERO,
            })
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn delete_playlist(&self, id: &str) -> Result<(), ProviderError> {
        let id_owned = id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(DeletePlaylist(&id_owned))
                .await
                .map_err(mpd_err("rm"))
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn add_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), ProviderError> {
        let playlist_owned = playlist_id.to_string();
        let uris: Vec<String> = track_ids.to_vec();
        self.block_on(async {
            let client = self.get_client().await?;
            for uri in &uris {
                client
                    .command(AddToPlaylist::new(&playlist_owned, uri))
                    .await
                    .map_err(mpd_err("playlistadd"))?;
            }
            Ok(())
        })
    }

    // --- Favorites and ratings via stickers (Tasks 52-53) ---

    fn toggle_favorite(&self, track_id: &str) -> Result<bool, ProviderError> {
        if !self.stickers_supported.load(Ordering::Relaxed) {
            return Err(ProviderError::NotSupported(
                "MPD stickers not enabled".into(),
            ));
        }
        let uri = track_id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            // Check current favorite state
            let is_fav = match client.command(StickerGet::new(&uri, "favorite")).await {
                Ok(s) => s.value == "1",
                Err(_) => false,
            };

            if is_fav {
                // Remove favorite sticker
                let _ = client.command(StickerDelete::new(&uri, "favorite")).await;
                Ok(false)
            } else {
                client
                    .command(StickerSet::new(&uri, "favorite", "1"))
                    .await
                    .map_err(mpd_err("sticker set favorite"))?;
                Ok(true)
            }
        })
    }

    fn is_favorite(&self, track_id: &str) -> Result<bool, ProviderError> {
        if !self.stickers_supported.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let uri = track_id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            match client.command(StickerGet::new(&uri, "favorite")).await {
                Ok(s) => Ok(s.value == "1"),
                Err(_) => Ok(false),
            }
        })
    }

    fn set_rating(&self, track_id: &str, rating: u8) -> Result<(), ProviderError> {
        if !self.stickers_supported.load(Ordering::Relaxed) {
            return Err(ProviderError::NotSupported(
                "MPD stickers not enabled".into(),
            ));
        }
        let uri = track_id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            if rating == 0 {
                // Clear rating
                let _ = client.command(StickerDelete::new(&uri, "rating")).await;
                Ok(())
            } else {
                let val = rating.min(5).to_string();
                client
                    .command(StickerSet::new(&uri, "rating", &val))
                    .await
                    .map_err(mpd_err("sticker set rating"))
            }
        })
    }

    fn get_rating(&self, track_id: &str) -> Result<Option<u8>, ProviderError> {
        if !self.stickers_supported.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let uri = track_id.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            match client.command(StickerGet::new(&uri, "rating")).await {
                Ok(s) => Ok(s.value.parse::<u8>().ok()),
                Err(_) => Ok(None),
            }
        })
    }

    fn list_favorites(&self) -> Result<Vec<Track>, ProviderError> {
        if !self.stickers_supported.load(Ordering::Relaxed) {
            return Err(ProviderError::NotSupported(
                "MPD stickers not enabled".into(),
            ));
        }
        self.block_on(async {
            let client = self.get_client().await?;
            // Find all songs with favorite=1 sticker
            let results = client
                .command(StickerFind::new("", "favorite").where_eq("1"))
                .await
                .map_err(mpd_err("sticker find favorite"))?;

            let mut tracks = Vec::with_capacity(results.value.len());
            for uri in results.value.keys() {
                // Look up each song's metadata
                let filter = Filter::new(Tag::Other("file".into()), Operator::Equal, uri.as_str());
                if let Ok(songs) = client.command(Find::new(filter)).await {
                    for song in &songs {
                        let mut track = self.song_to_track(song);
                        track.is_favorite = true;
                        tracks.push(track);
                    }
                }
            }
            Ok(tracks)
        })
    }

    // --- Genre methods (Tasks 54-55) ---

    #[tracing::instrument(skip(self), level = "debug")]
    fn list_genres(&self) -> Result<Vec<String>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            let genre_list = client
                .command(List::new(Tag::Genre))
                .await
                .map_err(mpd_err("list genre"))?;

            Ok(genre_list.into_iter().filter(|g| !g.is_empty()).collect())
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    fn get_tracks_by_genre(&self, genre: &str) -> Result<Vec<Track>, ProviderError> {
        let genre_owned = genre.to_string();
        self.block_on(async {
            let client = self.get_client().await?;
            let filter = Filter::tag(Tag::Genre, &genre_owned);
            let songs = client
                .command(Find::new(filter))
                .await
                .map_err(mpd_err("find genre"))?;

            let tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            Ok(tracks)
        })
    }
}
