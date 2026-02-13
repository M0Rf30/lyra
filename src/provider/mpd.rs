// SPDX-License-Identifier: GPL-3.0

//! MPD server music provider.
//!
//! Connects to an MPD server using the `mpd_client` crate, browses its
//! library, controls remote playback, and receives real-time idle events.

use super::{MusicProvider, ProviderError, ProviderType};
use crate::library::{Album, Artist, CoverSource, Track, TrackSource};
use mpd_client::client::{ConnectWithPasswordError, ConnectionEvents};
use mpd_client::commands::{self, Find, List, Status, CurrentSong, Update};
use mpd_client::filter::Filter;
use mpd_client::responses;
use mpd_client::tag::Tag;
use mpd_client::Client;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

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
        Self {
            id: entry.id,
            name: entry.name,
            host: entry.host,
            port: entry.port,
            password: entry.password,
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
    /// Command connection — used for browse, search, playback, etc.
    client: Arc<Mutex<Option<Client>>>,
    runtime: tokio::runtime::Handle,
}

impl MpdProvider {
    /// Create a new MPD provider. Does NOT connect yet — call `connect_idle()` first.
    pub fn new(config: MpdConfig, runtime: tokio::runtime::Handle) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(None)),
            runtime,
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

        log::info!(
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
    pub async fn connect_command(&self) -> Result<(), ProviderError> {
        let (client, _events) = self.open_connection().await?;
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
            .map_err(|e| ProviderError::Io(format!("MPD list album: {e}")))?;

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
                .map_err(|e| ProviderError::Io(format!("MPD find: {e}")))?;

            if songs.is_empty() {
                continue;
            }

            let mut tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            tracks.sort_by(|a, b| {
                a.disc_number
                    .cmp(&b.disc_number)
                    .then(a.track_number.cmp(&b.track_number))
            });

            let artist = tracks
                .first()
                .map(|t| t.album_artist.clone())
                .unwrap_or_default();
            let year = tracks.first().map(|t| t.year).unwrap_or(0);
            let cover_source = tracks
                .first()
                .map(|t| CoverSource::MpdAlbumArt(t.source_uri.clone()));

            albums.push(Album {
                name: album_name.clone(),
                artist,
                year,
                tracks,
                cover_source,
            });
        }

        Ok(albums)
    }

    /// Query MPD status (async).
    pub async fn status_async(&self) -> Result<responses::Status, ProviderError> {
        let client = self.get_client().await?;
        client
            .command(Status)
            .await
            .map_err(|e| ProviderError::Io(format!("MPD status: {e}")))
    }

    /// Query current song (async).
    pub async fn current_song_async(
        &self,
    ) -> Result<Option<responses::SongInQueue>, ProviderError> {
        let client = self.get_client().await?;
        client
            .command(CurrentSong)
            .await
            .map_err(|e| ProviderError::Io(format!("MPD currentsong: {e}")))
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
            provider_id: self.config.id.clone(),
            source_uri: uri,
        }
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

    fn browse_albums(&self) -> Result<Vec<Album>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            // Get all unique album names
            let album_list = client
                .command(List::new(Tag::Album))
                .await
                .map_err(|e| ProviderError::Io(format!("MPD list album: {e}")))?;

            let album_names: Vec<String> = album_list.into_iter().collect();
            let mut albums = Vec::new();

            for album_name in &album_names {
                if album_name.is_empty() {
                    continue;
                }

                // Find all songs in this album
                let filter = Filter::tag(Tag::Album, album_name);
                let songs = client
                    .command(Find::new(filter))
                    .await
                    .map_err(|e| ProviderError::Io(format!("MPD find: {e}")))?;

                if songs.is_empty() {
                    continue;
                }

                let mut tracks: Vec<Track> =
                    songs.iter().map(|s| self.song_to_track(s)).collect();
                tracks.sort_by(|a, b| {
                    a.disc_number
                        .cmp(&b.disc_number)
                        .then(a.track_number.cmp(&b.track_number))
                });

                let artist = tracks
                    .first()
                    .map(|t| t.album_artist.clone())
                    .unwrap_or_default();
                let year = tracks.first().map(|t| t.year).unwrap_or(0);
                let cover_source = tracks
                    .first()
                    .map(|t| CoverSource::MpdAlbumArt(t.source_uri.clone()));

                albums.push(Album {
                    name: album_name.clone(),
                    artist,
                    year,
                    tracks,
                    cover_source,
                });
            }

            albums.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(albums)
        })
    }

    fn browse_artists(&self) -> Result<Vec<Artist>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            let artist_list = client
                .command(List::new(Tag::AlbumArtist))
                .await
                .map_err(|e| ProviderError::Io(format!("MPD list albumartist: {e}")))?;

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
                    .map_err(|e| ProviderError::Io(format!("MPD list album: {e}")))?;

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
                        .map_err(|e| ProviderError::Io(format!("MPD find: {e}")))?;

                    let mut tracks: Vec<Track> =
                        songs.iter().map(|s| self.song_to_track(s)).collect();
                    tracks.sort_by(|a, b| {
                        a.disc_number
                            .cmp(&b.disc_number)
                            .then(a.track_number.cmp(&b.track_number))
                    });

                    let year = tracks.first().map(|t| t.year).unwrap_or(0);
                    let cover_source = tracks
                        .first()
                        .map(|t| CoverSource::MpdAlbumArt(t.source_uri.clone()));

                    artist_albums.push(Album {
                        name: album_name.clone(),
                        artist: artist_name.clone(),
                        year,
                        tracks,
                        cover_source,
                    });
                }

                artist_albums.sort_by(|a, b| a.year.cmp(&b.year));

                artists.push(Artist {
                    name: artist_name.clone(),
                    albums: artist_albums,
                });
            }

            artists.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(artists)
        })
    }

    fn browse_tracks(&self) -> Result<Vec<Track>, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;

            // Use listallinfo to get all songs from the root
            let songs = client
                .command(commands::ListAllIn::root())
                .await
                .map_err(|e| ProviderError::Io(format!("MPD listallinfo: {e}")))?;

            let mut tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            tracks.sort_by(|a, b| a.title.cmp(&b.title));
            Ok(tracks)
        })
    }

    fn search(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        let query_owned = query.to_string();
        self.block_on(async {
            let client = self.get_client().await?;

            // Search by title using the Find command with a tag filter.
            // The mpd_client typed API doesn't have a "search any" command,
            // so we search by title as a reasonable approximation.
            let filter = Filter::tag(Tag::Title, &query_owned);
            let songs = client
                .command(Find::new(filter))
                .await
                .map_err(|e| ProviderError::Io(format!("MPD search: {e}")))?;

            let tracks: Vec<Track> = songs.iter().map(|s| self.song_to_track(s)).collect();
            Ok(tracks)
        })
    }

    fn resolve_audio(&self, track: &Track) -> Result<TrackSource, ProviderError> {
        Ok(TrackSource::MpdFile(track.source_uri.clone()))
    }

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
                    log::warn!("Failed to get MPD album art for {uri}: {e}");
                    Ok(None)
                }
            }
        })
    }

    fn get_lyrics(&self, _track: &Track) -> Result<Option<String>, ProviderError> {
        // MPD doesn't serve lyrics; the app falls back to LRCLIB.
        Ok(None)
    }

    fn sync_library(&self) -> Result<usize, ProviderError> {
        self.block_on(async {
            let client = self.get_client().await?;
            client
                .command(Update::new())
                .await
                .map_err(|e| ProviderError::Io(format!("MPD update: {e}")))?;
            Ok(0)
        })
    }
}
