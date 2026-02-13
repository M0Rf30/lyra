// SPDX-License-Identifier: GPL-3.0

//! OpenSubsonic/Navidrome music provider.
//!
//! Connects to a Subsonic-compatible server using the `opensubsonic` crate,
//! browses the library via the ID3-based API, and streams audio via HTTP URLs
//! decoded locally by the `LocalBackend`.

use super::{MusicProvider, ProviderError, ProviderType};
use crate::library::{Album, Artist, CoverSource, Track, TrackSource};
use opensubsonic::{Auth, Client};
use std::path::PathBuf;
use std::time::Duration;

/// Configuration for connecting to a Subsonic-compatible server.
#[derive(Debug, Clone)]
pub struct SubsonicConfig {
    /// Unique provider ID (e.g., "subsonic-home").
    pub id: String,
    /// Human-readable name (e.g., "Navidrome").
    pub name: String,
    /// Server base URL (e.g., "https://music.example.com").
    pub url: String,
    /// Subsonic username.
    pub username: String,
    /// Password for authentication.
    pub password: String,
}

impl Default for SubsonicConfig {
    fn default() -> Self {
        Self {
            id: "subsonic".to_string(),
            name: "Subsonic Server".to_string(),
            url: "https://music.example.com".to_string(),
            username: String::new(),
            password: String::new(),
        }
    }
}

impl From<crate::config::SubsonicConfigEntry> for SubsonicConfig {
    fn from(entry: crate::config::SubsonicConfigEntry) -> Self {
        Self {
            id: entry.id,
            name: entry.name,
            url: entry.url,
            username: entry.username,
            password: entry.password.unwrap_or_default(),
        }
    }
}

/// Subsonic music provider backed by the `opensubsonic` crate.
///
/// All API calls are async (via the opensubsonic `Client`). The `MusicProvider`
/// trait is synchronous, so we bridge using `runtime.block_on()`.
pub struct SubsonicProvider {
    config: SubsonicConfig,
    client: Client,
    runtime: tokio::runtime::Handle,
}

impl SubsonicProvider {
    /// Create a new Subsonic provider. The client is constructed immediately
    /// but no network call is made until `ping()` or a browse method is called.
    pub fn new(config: SubsonicConfig, runtime: tokio::runtime::Handle) -> Result<Self, ProviderError> {
        let auth = Auth::token(&config.password);
        let client = Client::new(&config.url, &config.username, auth)
            .map_err(|e| ProviderError::NotConnected(format!("Invalid Subsonic URL: {e}")))?
            .with_client_name("cosmic-music-player");

        Ok(Self {
            config,
            client,
            runtime,
        })
    }

    /// Validate connectivity by sending a `ping` request.
    pub async fn ping(&self) -> Result<(), ProviderError> {
        self.client
            .ping()
            .await
            .map_err(|e| ProviderError::NotConnected(format!("Subsonic ping failed: {e}")))
    }

    /// Run an async block on the tokio runtime (bridging sync MusicProvider to async).
    fn block_on<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.runtime.block_on(future)
    }

    /// Convert an opensubsonic `Child` (song) to our `Track` model.
    ///
    /// The `source_uri` is set to the full authenticated stream URL so that
    /// `resolve_track_source()` in the player can create an `HttpStream`
    /// without needing access to the provider/client.
    fn child_to_track(&self, child: &opensubsonic::data::Child) -> Track {
        let title = child.title.clone();
        let artist = child.artist.clone().unwrap_or_else(|| "Unknown Artist".to_string());
        let album_artist = child
            .display_album_artist
            .clone()
            .or_else(|| {
                child
                    .album_artists
                    .as_ref()
                    .and_then(|a| a.first())
                    .map(|a| a.name.clone())
            })
            .unwrap_or_else(|| artist.clone());
        let album = child.album.clone().unwrap_or_else(|| "Unknown Album".to_string());
        let genre = child.genre.clone().unwrap_or_default();
        let track_number = child.track.unwrap_or(0) as u32;
        let disc_number = child.disc_number.unwrap_or(1) as u32;
        let year = child.year.unwrap_or(0) as u32;
        let duration = Duration::from_secs(child.duration.unwrap_or(0) as u64);
        let bitrate = child.bit_rate.unwrap_or(0) as u32;
        let sample_rate = child.sampling_rate.unwrap_or(0) as u32;

        // Pre-build the authenticated stream URL so the player can use it
        // directly without needing access to the Subsonic client.
        let source_uri = self
            .client
            .stream_url(&child.id, None, None)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| child.id.clone());

        Track {
            id: 0,
            path: PathBuf::from(&child.id),
            title,
            artist,
            album_artist,
            album,
            genre,
            track_number,
            disc_number,
            year,
            duration,
            bitrate,
            sample_rate,
            provider_id: self.config.id.clone(),
            source_uri,
        }
    }

    /// Scrobble a track (mark as played) on the Subsonic server.
    ///
    /// The `song_id` is the Subsonic song ID (not the stream URL).
    /// Call this when a track reaches 50% or 4 minutes of playback.
    pub fn scrobble(&self, song_id: &str) {
        let id = song_id.to_string();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            if let Err(e) = client.scrobble(&id, None, Some(true)).await {
                log::warn!("Subsonic scrobble failed for '{id}': {e}");
            } else {
                log::debug!("Scrobbled song '{id}'");
            }
        });
    }

    /// Send a "now playing" notification to the Subsonic server.
    pub fn now_playing(&self, song_id: &str) {
        let id = song_id.to_string();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            if let Err(e) = client.scrobble(&id, None, Some(false)).await {
                log::debug!("Subsonic now-playing failed for '{id}': {e}");
            }
        });
    }
}

impl MusicProvider for SubsonicProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Subsonic
    }

    fn browse_albums(&self) -> Result<Vec<Album>, ProviderError> {
        self.block_on(async {
            // Paginate through all albums using getAlbumList2 (ID3-based).
            let mut all_albums = Vec::new();
            let page_size = 500;
            let mut offset = 0;

            loop {
                let album_list = self
                    .client
                    .get_album_list2(
                        opensubsonic::AlbumListType::AlphabeticalByName,
                        Some(page_size),
                        Some(offset),
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| ProviderError::Io(format!("Subsonic getAlbumList2: {e}")))?;

                let count = album_list.len();
                if count == 0 {
                    break;
                }

                for album_id3 in &album_list {
                    // Fetch full album details (with songs) for each album.
                    let album_detail = self
                        .client
                        .get_album(&album_id3.id)
                        .await
                        .map_err(|e| ProviderError::Io(format!("Subsonic getAlbum: {e}")))?;

                    let mut tracks: Vec<Track> = album_detail
                        .song
                        .iter()
                        .map(|s| self.child_to_track(s))
                        .collect();
                    tracks.sort_by(|a, b| {
                        a.disc_number
                            .cmp(&b.disc_number)
                            .then(a.track_number.cmp(&b.track_number))
                    });

                    let artist = album_detail
                        .artist
                        .clone()
                        .unwrap_or_else(|| "Unknown Artist".to_string());
                    let year = album_detail.year.unwrap_or(0) as u32;

                    // Build cover source from cover art ID.
                    let cover_source = album_detail.cover_art.as_ref().and_then(|cover_id| {
                        self.client
                            .cover_art_url(cover_id, Some(300))
                            .ok()
                            .map(|url| CoverSource::Url(url.to_string()))
                    });

                    all_albums.push(Album {
                        name: album_detail.name.clone(),
                        artist,
                        year,
                        tracks,
                        cover_source,
                    });
                }

                if count < page_size as usize {
                    break;
                }
                offset += page_size;
            }

            all_albums.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(all_albums)
        })
    }

    fn browse_artists(&self) -> Result<Vec<Artist>, ProviderError> {
        self.block_on(async {
            let artists_id3 = self
                .client
                .get_artists(None)
                .await
                .map_err(|e| ProviderError::Io(format!("Subsonic getArtists: {e}")))?;

            let mut artists = Vec::new();

            for index in &artists_id3.index {
                for artist_id3 in &index.artist {
                    // Get artist details with album list.
                    let artist_detail = self
                        .client
                        .get_artist(&artist_id3.id)
                        .await
                        .map_err(|e| ProviderError::Io(format!("Subsonic getArtist: {e}")))?;

                    let mut artist_albums = Vec::new();

                    for album_id3 in &artist_detail.album {
                        // Fetch full album (with songs).
                        let album_detail = self
                            .client
                            .get_album(&album_id3.id)
                            .await
                            .map_err(|e| ProviderError::Io(format!("Subsonic getAlbum: {e}")))?;

                        let mut tracks: Vec<Track> = album_detail
                            .song
                            .iter()
                            .map(|s| self.child_to_track(s))
                            .collect();
                        tracks.sort_by(|a, b| {
                            a.disc_number
                                .cmp(&b.disc_number)
                                .then(a.track_number.cmp(&b.track_number))
                        });

                        let year = album_detail.year.unwrap_or(0) as u32;
                        let cover_source =
                            album_detail.cover_art.as_ref().and_then(|cover_id| {
                                self.client
                                    .cover_art_url(cover_id, Some(300))
                                    .ok()
                                    .map(|url| CoverSource::Url(url.to_string()))
                            });

                        artist_albums.push(Album {
                            name: album_detail.name.clone(),
                            artist: artist_id3.name.clone(),
                            year,
                            tracks,
                            cover_source,
                        });
                    }

                    artist_albums.sort_by(|a, b| a.year.cmp(&b.year));

                    artists.push(Artist {
                        name: artist_id3.name.clone(),
                        albums: artist_albums,
                    });
                }
            }

            artists.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(artists)
        })
    }

    fn browse_tracks(&self) -> Result<Vec<Track>, ProviderError> {
        self.block_on(async {
            // Collect all tracks by paginating through albums and extracting songs.
            let mut all_tracks = Vec::new();
            let page_size = 500;
            let mut offset = 0;

            loop {
                let album_list = self
                    .client
                    .get_album_list2(
                        opensubsonic::AlbumListType::AlphabeticalByName,
                        Some(page_size),
                        Some(offset),
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| ProviderError::Io(format!("Subsonic getAlbumList2: {e}")))?;

                let count = album_list.len();
                if count == 0 {
                    break;
                }

                for album_id3 in &album_list {
                    let album_detail = self
                        .client
                        .get_album(&album_id3.id)
                        .await
                        .map_err(|e| ProviderError::Io(format!("Subsonic getAlbum: {e}")))?;

                    for song in &album_detail.song {
                        all_tracks.push(self.child_to_track(song));
                    }
                }

                if count < page_size as usize {
                    break;
                }
                offset += page_size;
            }

            all_tracks.sort_by(|a, b| a.title.cmp(&b.title));
            Ok(all_tracks)
        })
    }

    fn search(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        let query_owned = query.to_string();
        self.block_on(async {
            let results = self
                .client
                .search3(
                    &query_owned,
                    Some(0),  // no artists
                    None,
                    Some(0),  // no albums
                    None,
                    Some(50), // up to 50 songs
                    None,
                    None,
                )
                .await
                .map_err(|e| ProviderError::Io(format!("Subsonic search3: {e}")))?;

            let tracks: Vec<Track> = results
                .song
                .iter()
                .map(|s| self.child_to_track(s))
                .collect();
            Ok(tracks)
        })
    }

    fn resolve_audio(&self, track: &Track) -> Result<TrackSource, ProviderError> {
        // Build an authenticated streaming URL for this song ID.
        let url = self
            .client
            .stream_url(&track.source_uri, None, None)
            .map_err(|e| ProviderError::Io(format!("Subsonic stream_url: {e}")))?;
        Ok(TrackSource::HttpStream(url.to_string()))
    }

    fn get_cover_art(&self, album: &Album) -> Result<Option<Vec<u8>>, ProviderError> {
        // The cover_source should already contain a Url from browse_albums().
        // Fetch the image bytes for display.
        match &album.cover_source {
            Some(CoverSource::Url(url)) => {
                self.block_on(async {
                    let resp = reqwest::get(url)
                        .await
                        .map_err(|e| ProviderError::Io(format!("Cover art fetch: {e}")))?;
                    let bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| ProviderError::Io(format!("Cover art read: {e}")))?;
                    Ok(Some(bytes.to_vec()))
                })
            }
            _ => {
                // Try to get cover art from the first track's album ID.
                if let Some(first_track) = album.tracks.first() {
                    self.block_on(async {
                        // The source_uri is the song ID; we can try using it as the cover art ID.
                        match self.client.get_cover_art(&first_track.source_uri, Some(300)).await {
                            Ok(bytes) => Ok(Some(bytes.to_vec())),
                            Err(e) => {
                                log::warn!("Subsonic cover art for '{}': {e}", album.name);
                                Ok(None)
                            }
                        }
                    })
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn get_lyrics(&self, track: &Track) -> Result<Option<String>, ProviderError> {
        self.block_on(async {
            // Try getLyricsBySongId first (OpenSubsonic extension).
            match self.client.get_lyrics_by_song_id(&track.source_uri).await {
                Ok(lyrics_list) => {
                    if let Some(structured) = lyrics_list.structured_lyrics.first() {
                        let text: String = structured
                            .line
                            .iter()
                            .map(|l| l.value.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.is_empty() {
                            return Ok(Some(text));
                        }
                    }
                }
                Err(e) => {
                    log::debug!("getLyricsBySongId not available: {e}");
                }
            }

            // Fallback: legacy getLyrics with artist + title.
            match self
                .client
                .get_lyrics(Some(&track.artist), Some(&track.title))
                .await
            {
                Ok(lyrics) => {
                    if let Some(text) = lyrics.value
                        && !text.is_empty()
                    {
                        return Ok(Some(text));
                    }
                }
                Err(e) => {
                    log::debug!("getLyrics not available: {e}");
                }
            }

            Ok(None)
        })
    }

    fn sync_library(&self) -> Result<usize, ProviderError> {
        // Subsonic servers manage their own library scanning.
        // We can trigger a scan via startScan, but the actual library
        // data is fetched on demand via browse_*() methods.
        self.block_on(async {
            match self.client.start_scan().await {
                Ok(_) => log::info!("Triggered Subsonic library scan"),
                Err(e) => log::warn!("Subsonic startScan: {e}"),
            }
            Ok(0)
        })
    }
}
