// SPDX-License-Identifier: GPL-3.0

use crate::config::Config;
use crate::fl;
use crate::library::{Album, Artist, LibraryDb, LibraryScanner, LyricsProvider, Track};
use crate::player::mpd_backend::MpdBackend;
use crate::player::{PlaybackState, Player};
use crate::provider::local::LocalProvider;
use crate::provider::mpd::{MpdConfig, MpdProvider};
use crate::provider::subsonic::{SubsonicConfig, SubsonicProvider};
use crate::provider::{MusicProvider, ProviderRegistry};
use crate::views::{albums, artists, equalizer, lyrics, now_playing, providers, songs};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use cosmic::{iced_futures, prelude::*};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

/// Wrapper around `Arc<MpdProvider>` that implements `MusicProvider`.
///
/// This is needed so we can share the provider between the registry
/// (which owns `Box<dyn MusicProvider>`) and async connection tasks
/// (which need `Arc<MpdProvider>`).
struct MpdProviderWrapper(Arc<MpdProvider>);

impl crate::provider::MusicProvider for MpdProviderWrapper {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn provider_type(&self) -> crate::provider::ProviderType {
        self.0.provider_type()
    }
    fn browse_albums(&self) -> Result<Vec<Album>, crate::provider::ProviderError> {
        self.0.browse_albums()
    }
    fn browse_artists(&self) -> Result<Vec<Artist>, crate::provider::ProviderError> {
        self.0.browse_artists()
    }
    fn browse_tracks(&self) -> Result<Vec<Track>, crate::provider::ProviderError> {
        self.0.browse_tracks()
    }
    fn search(&self, query: &str) -> Result<Vec<Track>, crate::provider::ProviderError> {
        self.0.search(query)
    }
    fn resolve_audio(
        &self,
        track: &Track,
    ) -> Result<crate::library::TrackSource, crate::provider::ProviderError> {
        self.0.resolve_audio(track)
    }
    fn get_cover_art(
        &self,
        album: &Album,
    ) -> Result<Option<Vec<u8>>, crate::provider::ProviderError> {
        self.0.get_cover_art(album)
    }
    fn get_lyrics(
        &self,
        track: &Track,
    ) -> Result<Option<String>, crate::provider::ProviderError> {
        self.0.get_lyrics(track)
    }
    fn sync_library(&self) -> Result<usize, crate::provider::ProviderError> {
        self.0.sync_library()
    }
}

/// Wrapper around `Arc<SubsonicProvider>` that implements `MusicProvider`.
struct SubsonicProviderWrapper(Arc<SubsonicProvider>);

impl crate::provider::MusicProvider for SubsonicProviderWrapper {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn provider_type(&self) -> crate::provider::ProviderType {
        self.0.provider_type()
    }
    fn browse_albums(&self) -> Result<Vec<Album>, crate::provider::ProviderError> {
        self.0.browse_albums()
    }
    fn browse_artists(&self) -> Result<Vec<Artist>, crate::provider::ProviderError> {
        self.0.browse_artists()
    }
    fn browse_tracks(&self) -> Result<Vec<Track>, crate::provider::ProviderError> {
        self.0.browse_tracks()
    }
    fn search(&self, query: &str) -> Result<Vec<Track>, crate::provider::ProviderError> {
        self.0.search(query)
    }
    fn resolve_audio(
        &self,
        track: &Track,
    ) -> Result<crate::library::TrackSource, crate::provider::ProviderError> {
        self.0.resolve_audio(track)
    }
    fn get_cover_art(
        &self,
        album: &Album,
    ) -> Result<Option<Vec<u8>>, crate::provider::ProviderError> {
        self.0.get_cover_art(album)
    }
    fn get_lyrics(
        &self,
        track: &Track,
    ) -> Result<Option<String>, crate::provider::ProviderError> {
        self.0.get_lyrics(track)
    }
    fn sync_library(&self) -> Result<usize, crate::provider::ProviderError> {
        self.0.sync_library()
    }
}

/// Main application model.
pub struct AppModel {
    core: cosmic::Core,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    about: About,
    config: Config,
    context_page: ContextPage,

    // Providers
    registry: ProviderRegistry,
    /// Shared references to MPD providers for idle event subscriptions.
    mpd_providers: Vec<Arc<MpdProvider>>,
    /// Ordered list of (provider_id, display_name) for the selector dropdown.
    provider_list: Vec<(String, String)>,
    /// Index of the active provider in `provider_list`.
    active_provider_index: Option<usize>,

    // Library data
    all_tracks: Vec<Track>,
    all_albums: Vec<Album>,
    all_artists: Vec<Artist>,
    library_scanning: bool,

    // Player
    player: Option<Player>,
    playback_position: Duration,
    current_track: Option<Track>,
    /// While the user is dragging the seek slider, holds the preview fraction
    /// (0.0–1.0). `None` when not dragging. The actual backend seek happens
    /// only on release (`SeekCommit`).
    seeking_preview: Option<f32>,

    // Scrobble state (Subsonic)
    /// Whether a "now playing" notification has been sent for the current track.
    scrobble_now_playing_sent: bool,
    /// Whether the current track has been scrobbled (to avoid duplicates).
    scrobble_sent: bool,

    // View state
    selected_album: Option<usize>,
    selected_artist: Option<usize>,
    songs_sort: songs::SortField,
    cover_images: HashMap<String, widget::icon::Handle>,
    artist_avatars: HashMap<String, widget::icon::Handle>,

    // Lyrics
    lyrics_text: Option<String>,
    lyrics_loading: bool,

    // Equalizer
    eq_preset: Option<crate::player::equalizer::EqPreset>,

    // Provider settings (editing state)
    mpd_edit_states: Vec<providers::MpdEditState>,
    mpd_connection_status: Vec<Option<String>>,
    subsonic_edit_states: Vec<providers::SubsonicEditState>,
    subsonic_connection_status: Vec<Option<String>>,
    /// Shared references to Subsonic providers for scrobbling.
    subsonic_providers: Vec<Arc<SubsonicProvider>>,
}

/// All application messages.
#[derive(Debug, Clone)]
pub enum Message {
    // Navigation / chrome
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    Surface(cosmic::surface::Action),

    // Library
    ScanLibrary,
    LibraryScanComplete(usize),
    LibraryLoaded {
        tracks: Vec<Track>,
        albums: Vec<Album>,
        artists: Vec<Artist>,
        cover_images: HashMap<String, widget::icon::Handle>,
        artist_avatars: HashMap<String, widget::icon::Handle>,
    },
    /// Incremental batch of albums from a remote provider (e.g. Subsonic).
    /// Each batch appends albums, derives tracks/artists, and updates the UI.
    LibraryBatch {
        albums: Vec<Album>,
        cover_images: HashMap<String, widget::icon::Handle>,
    },
    /// Signals that incremental loading is complete.
    LibraryLoadComplete,

    // Player transport
    TogglePlayback,
    NextTrack,
    PreviousTrack,
    /// Visual-only update while dragging the seek slider (no backend seek).
    SeekPreview(f32),
    /// Performs the actual backend seek when the slider is released.
    SeekCommit,
    SetVolume(f32),
    ToggleShuffle,
    CycleRepeat,
    PlaybackTick,

    // Track selection
    PlayTrackIndex(usize),
    PlayAlbum(usize),
    PlayAlbumTrack(usize, usize),
    PlayArtistAlbum(usize, usize),
    PlayArtistTrack(usize, usize, usize),

    // Albums view
    SelectAlbum(usize),
    BackToAlbumGrid,

    // Artists view
    SelectArtist(usize),
    BackToArtistList,

    // Songs view
    SortSongs(songs::SortField),

    // Lyrics
    ShowLyrics,
    LyricsLoaded(Option<String>),
    FetchLyricsOnline,

    // Equalizer
    EqSetBand(usize, f32),
    EqSetPreset(crate::player::equalizer::EqPreset),
    EqToggle(bool),

    // Settings
    AddMusicDir,
    UpdateConfig(Config),

    // Provider switching
    SwitchProvider(usize),

    // MPD provider configuration
    MpdAddServer,
    MpdEditName(usize, String),
    MpdEditHost(usize, String),
    MpdEditPort(usize, String),
    MpdEditPassword(usize, String),
    MpdSaveServer(usize),
    MpdRemoveServer(usize),
    MpdTestConnection(usize),
    MpdTestResult(usize, Result<(), String>),

    // MPD provider events
    MpdConnected(String),
    MpdConnectionFailed(String, String),
    MpdIdleEvent(String),

    // Subsonic provider configuration
    SubsonicAddServer,
    SubsonicEditName(usize, String),
    SubsonicEditUrl(usize, String),
    SubsonicEditUsername(usize, String),
    SubsonicEditPassword(usize, String),
    SubsonicToggleCerts(usize, bool),
    SubsonicSaveServer(usize),
    SubsonicRemoveServer(usize),
    SubsonicTestConnection(usize),
    SubsonicTestResult(usize, Result<(), String>),

    // Application lifecycle
    Quit,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.m0rf30.CosmicMusicPlayer";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut nav = nav_bar::Model::default();

        nav.insert()
            .text(fl!("albums"))
            .data::<Page>(Page::Albums)
            .icon(icon::from_name("media-optical-cd-audio-symbolic"))
            .activate();

        nav.insert()
            .text(fl!("artists"))
            .data::<Page>(Page::Artists)
            .icon(icon::from_name("system-users-symbolic"));

        nav.insert()
            .text(fl!("songs"))
            .data::<Page>(Page::Songs)
            .icon(icon::from_name("audio-x-generic-symbolic"));

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license("GPL-3.0");

        // Load config
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        // Open library database and initialize provider registry
        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cosmic-music-player")
            .join("library.db");

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let mut registry = ProviderRegistry::new();
        if let Ok(db) = LibraryDb::open(&db_path) {
            let local = LocalProvider::new(db, config.music_dirs.clone());
            registry.register(Box::new(local));
        } else {
            log::error!("Failed to open library database");
        }

        // Initialize MPD providers from config.
        // Providers are registered immediately (browse returns NotConnected until
        // the subscription establishes the connection). The actual TCP connect +
        // idle-event loop runs inside a COSMIC subscription (see `subscription()`).
        let rt_handle = tokio::runtime::Handle::current();
        let mut mpd_providers = Vec::new();
        for entry in &config.mpd_servers {
            let mpd_config: MpdConfig = entry.clone().into();
            let provider = Arc::new(MpdProvider::new(mpd_config, rt_handle.clone()));
            mpd_providers.push(Arc::clone(&provider));
            registry.register(Box::new(MpdProviderWrapper(provider)));
        }

        // Initialize Subsonic providers from config.
        let mut subsonic_providers = Vec::new();
        for entry in &config.subsonic_servers {
            let subsonic_config: SubsonicConfig = entry.clone().into();
            match SubsonicProvider::new(subsonic_config, rt_handle.clone()) {
                Ok(provider) => {
                    let provider = Arc::new(provider);
                    subsonic_providers.push(Arc::clone(&provider));
                    registry.register(Box::new(SubsonicProviderWrapper(provider)));
                }
                Err(e) => {
                    log::error!("Failed to create Subsonic provider '{}': {e}", entry.name);
                }
            }
        }

        // Build provider list for the dropdown selector
        let provider_entries = registry.list();
        let provider_list: Vec<(String, String)> = provider_entries
            .iter()
            .map(|(id, name, _)| (id.clone(), name.clone()))
            .collect();
        let active_provider_index = provider_list
            .iter()
            .position(|(id, _)| id == registry.active_id());

        // Build editing state for MPD servers
        let mpd_edit_states: Vec<providers::MpdEditState> = config
            .mpd_servers
            .iter()
            .map(providers::MpdEditState::from_config)
            .collect();
        let mpd_connection_status: Vec<Option<String>> =
            vec![None; mpd_edit_states.len()];

        // Build editing state for Subsonic servers
        let subsonic_edit_states: Vec<providers::SubsonicEditState> = config
            .subsonic_servers
            .iter()
            .map(providers::SubsonicEditState::from_config)
            .collect();
        let subsonic_connection_status: Vec<Option<String>> =
            vec![None; subsonic_edit_states.len()];

        // Initialize player
        let player = match Player::new(None) {
            Ok(p) => Some(p),
            Err(e) => {
                log::error!("Failed to initialize audio player: {e}");
                None
            }
        };

        let mut app = AppModel {
            core,
            nav,
            key_binds: HashMap::new(),
            about,
            config,
            context_page: ContextPage::default(),
            registry,
            mpd_providers,
            provider_list,
            active_provider_index,
            all_tracks: Vec::new(),
            all_albums: Vec::new(),
            all_artists: Vec::new(),
            library_scanning: false,
            player,
            playback_position: Duration::ZERO,
            current_track: None,
            seeking_preview: None,
            scrobble_now_playing_sent: false,
            scrobble_sent: false,
            selected_album: None,
            selected_artist: None,
            songs_sort: songs::SortField::Title,
            cover_images: HashMap::new(),
            artist_avatars: HashMap::new(),
            lyrics_text: None,
            lyrics_loading: false,
            eq_preset: None,
            mpd_edit_states,
            mpd_connection_status,
            subsonic_edit_states,
            subsonic_connection_status,
            subsonic_providers,
        };

        let title_cmd = app.update_title();

        // Trigger initial library scan
        let scan_cmd = cosmic::task::message(cosmic::Action::App(Message::ScanLibrary));

        (app, Task::batch([title_cmd, scan_cmd]))
    }

    /// Header bar start: menu bar.
    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![
            menu::Tree::with_children(
                menu::root(fl!("file")).apply(Element::from),
                menu::items(
                    &self.key_binds,
                    vec![
                        menu::Item::Button(fl!("add-music-folder"), None, MenuAction::AddMusicDir),
                        menu::Item::Button(fl!("scan-library"), None, MenuAction::ScanLibrary),
                        menu::Item::Divider,
                        menu::Item::Button(fl!("quit"), None, MenuAction::Quit),
                    ],
                ),
            ),
            menu::Tree::with_children(
                menu::root(fl!("view")).apply(Element::from),
                menu::items(
                    &self.key_binds,
                    vec![
                        menu::Item::Button(fl!("equalizer"), None, MenuAction::Equalizer),
                        menu::Item::Button(fl!("providers"), None, MenuAction::Providers),
                        menu::Item::Divider,
                        menu::Item::Button(fl!("about"), None, MenuAction::About),
                    ],
                ),
            ),
        ]);

        vec![menu_bar.into()]
    }

    /// Header bar center: empty (playback controls are in the bottom bar).
    fn header_center(&self) -> Vec<Element<'_, Self::Message>> {
        vec![]
    }

    /// Header bar end: provider selector (shown when multiple providers are configured).
    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        if self.provider_list.len() <= 1 {
            return vec![];
        }

        let provider_names: Vec<String> =
            self.provider_list.iter().map(|(_, name)| name.clone()).collect();

        let dropdown = widget::dropdown(
            provider_names,
            self.active_provider_index,
            Message::SwitchProvider,
        );

        vec![dropdown.into()]
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::Equalizer => {
                let eq_content = equalizer::equalizer_view(
                    &self.config.equalizer_bands,
                    self.config.equalizer_enabled,
                    self.eq_preset,
                )
                .map(|msg| match msg {
                    equalizer::EqualizerMessage::SetBand(i, v) => Message::EqSetBand(i, v),
                    equalizer::EqualizerMessage::SetPreset(p) => Message::EqSetPreset(p),
                    equalizer::EqualizerMessage::ToggleEnabled(e) => Message::EqToggle(e),
                });

                context_drawer::context_drawer(
                    eq_content,
                    Message::ToggleContextPage(ContextPage::Equalizer),
                )
                .title(fl!("equalizer"))
            }
            ContextPage::Providers => {
                let providers_content = providers::providers_view(
                    &self.mpd_edit_states,
                    &self.mpd_connection_status,
                    &self.subsonic_edit_states,
                    &self.subsonic_connection_status,
                )
                .map(|msg| match msg {
                    // MPD
                    providers::ProvidersMessage::AddMpd => Message::MpdAddServer,
                    providers::ProvidersMessage::EditName(i, v) => Message::MpdEditName(i, v),
                    providers::ProvidersMessage::EditHost(i, v) => Message::MpdEditHost(i, v),
                    providers::ProvidersMessage::EditPort(i, v) => Message::MpdEditPort(i, v),
                    providers::ProvidersMessage::EditPassword(i, v) => {
                        Message::MpdEditPassword(i, v)
                    }
                    providers::ProvidersMessage::Save(i) => Message::MpdSaveServer(i),
                    providers::ProvidersMessage::Remove(i) => Message::MpdRemoveServer(i),
                    providers::ProvidersMessage::TestConnection(i) => {
                        Message::MpdTestConnection(i)
                    }
                    // Subsonic
                    providers::ProvidersMessage::AddSubsonic => Message::SubsonicAddServer,
                    providers::ProvidersMessage::SubsonicEditName(i, v) => {
                        Message::SubsonicEditName(i, v)
                    }
                    providers::ProvidersMessage::SubsonicEditUrl(i, v) => {
                        Message::SubsonicEditUrl(i, v)
                    }
                    providers::ProvidersMessage::SubsonicEditUsername(i, v) => {
                        Message::SubsonicEditUsername(i, v)
                    }
                    providers::ProvidersMessage::SubsonicEditPassword(i, v) => {
                        Message::SubsonicEditPassword(i, v)
                    }
                    providers::ProvidersMessage::SubsonicToggleCerts(i, v) => {
                        Message::SubsonicToggleCerts(i, v)
                    }
                    providers::ProvidersMessage::SubsonicSave(i) => {
                        Message::SubsonicSaveServer(i)
                    }
                    providers::ProvidersMessage::SubsonicRemove(i) => {
                        Message::SubsonicRemoveServer(i)
                    }
                    providers::ProvidersMessage::SubsonicTestConnection(i) => {
                        Message::SubsonicTestConnection(i)
                    }
                });

                context_drawer::context_drawer(
                    providers_content,
                    Message::ToggleContextPage(ContextPage::Providers),
                )
                .title(fl!("providers"))
            }
            ContextPage::Lyrics => {
                let (title, artist) = self
                    .current_track
                    .as_ref()
                    .map(|t| (t.title.as_str(), t.artist.as_str()))
                    .unwrap_or(("", ""));

                let lyrics_content = lyrics::lyrics_view(
                    self.lyrics_text.as_deref(),
                    title,
                    artist,
                    self.lyrics_loading,
                )
                .map(|msg| match msg {
                    lyrics::LyricsMessage::FetchLyrics => Message::FetchLyricsOnline,
                    lyrics::LyricsMessage::Close => {
                        Message::ToggleContextPage(ContextPage::Lyrics)
                    }
                });

                context_drawer::context_drawer(
                    lyrics_content,
                    Message::ToggleContextPage(ContextPage::Lyrics),
                )
                .title(fl!("lyrics"))
            }
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let page = self.nav.active_data::<Page>().cloned().unwrap_or(Page::Albums);

        let content: Element<'_, Self::Message> = match page {
            Page::Albums => {
                if let Some(album_idx) = self.selected_album {
                    if let Some(album) = self.all_albums.get(album_idx) {
                        albums::album_detail_view(album, album_idx, &self.cover_images).map(|msg| match msg {
                            albums::AlbumMessage::PlayAlbum(i) => Message::PlayAlbum(i),
                            albums::AlbumMessage::PlayTrack(ai, ti) => {
                                Message::PlayAlbumTrack(ai, ti)
                            }
                            albums::AlbumMessage::SelectAlbum(i) => Message::SelectAlbum(i),
                            albums::AlbumMessage::BackToGrid => Message::BackToAlbumGrid,
                        })
                    } else {
                        widget::text("Album not found").into()
                    }
                } else {
                    albums::album_grid_view(&self.all_albums, &self.cover_images).map(|msg| {
                        match msg {
                            albums::AlbumMessage::SelectAlbum(i) => Message::SelectAlbum(i),
                            albums::AlbumMessage::PlayAlbum(i) => Message::PlayAlbum(i),
                            albums::AlbumMessage::PlayTrack(ai, ti) => {
                                Message::PlayAlbumTrack(ai, ti)
                            }
                            albums::AlbumMessage::BackToGrid => Message::BackToAlbumGrid,
                        }
                    })
                }
            }

            Page::Artists => {
                if let Some(artist_idx) = self.selected_artist {
                    if let Some(artist) = self.all_artists.get(artist_idx) {
                        artists::artist_detail_view(artist, artist_idx, &self.artist_avatars, &self.cover_images).map(|msg| match msg {
                            artists::ArtistMessage::PlayArtistAlbum(ai, ali) => {
                                Message::PlayArtistAlbum(ai, ali)
                            }
                            artists::ArtistMessage::PlayTrack(ai, ali, ti) => {
                                Message::PlayArtistTrack(ai, ali, ti)
                            }
                            artists::ArtistMessage::SelectArtist(i) => Message::SelectArtist(i),
                            artists::ArtistMessage::BackToList => Message::BackToArtistList,
                        })
                    } else {
                        widget::text("Artist not found").into()
                    }
                } else {
                    artists::artist_list_view(&self.all_artists, &self.artist_avatars).map(|msg| match msg {
                        artists::ArtistMessage::SelectArtist(i) => Message::SelectArtist(i),
                        artists::ArtistMessage::PlayArtistAlbum(ai, ali) => {
                            Message::PlayArtistAlbum(ai, ali)
                        }
                        artists::ArtistMessage::PlayTrack(ai, ali, ti) => {
                            Message::PlayArtistTrack(ai, ali, ti)
                        }
                        artists::ArtistMessage::BackToList => Message::BackToArtistList,
                    })
                }
            }

            Page::Songs => {
                songs::songs_list_view(&self.all_tracks, self.songs_sort).map(|msg| match msg {
                    songs::SongMessage::PlayTrack(i) => Message::PlayTrackIndex(i),
                    songs::SongMessage::SortBy(f) => Message::SortSongs(f),
                })
            }
        };

        // Build bottom playback bar
        let state = self
            .player
            .as_ref()
            .map(|p| p.state())
            .unwrap_or(PlaybackState::Stopped);
        let duration = self
            .current_track
            .as_ref()
            .map(|t| t.duration)
            .unwrap_or(Duration::ZERO);
        let volume = self
            .player
            .as_ref()
            .map(|p| p.volume())
            .unwrap_or(self.config.volume);
        let current_cover = self.current_track.as_ref().and_then(|track| {
            let key = crate::library::CoverArt::album_key(&track.artist, &track.album);
            self.cover_images.get(&key)
        });

        let bar = now_playing::playback_bar(
            self.current_track.as_ref(),
            state,
            self.playback_position,
            duration,
            volume,
            self.config.shuffle,
            self.config.repeat_mode,
            current_cover,
            self.seeking_preview,
        )
        .map(|msg| match msg {
            now_playing::NowPlayingMessage::TogglePlayback => Message::TogglePlayback,
            now_playing::NowPlayingMessage::Next => Message::NextTrack,
            now_playing::NowPlayingMessage::Previous => Message::PreviousTrack,
            now_playing::NowPlayingMessage::SeekPreview(v) => Message::SeekPreview(v),
            now_playing::NowPlayingMessage::SeekCommit => Message::SeekCommit,
            now_playing::NowPlayingMessage::SetVolume(v) => Message::SetVolume(v),
            now_playing::NowPlayingMessage::ToggleShuffle => Message::ToggleShuffle,
            now_playing::NowPlayingMessage::CycleRepeat => Message::CycleRepeat,
            now_playing::NowPlayingMessage::ShowLyrics => Message::ShowLyrics,
        });

        // Main layout: content + optional scanning indicator + bottom playback bar
        let mut layout = widget::column().push(
            widget::container(content)
                .width(Length::Fill)
                .height(Length::Fill),
        );

        if self.library_scanning {
            layout = layout.push(
                widget::container(
                    widget::row()
                        .push(widget::text::caption(fl!("scanning-library")))
                        .spacing(8)
                        .align_y(Alignment::Center),
                )
                .padding(4)
                .width(Length::Fill),
            );
        }

        layout = layout.push(bar);

        widget::container(layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subs = vec![
            // Watch config changes
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];

        // Playback position ticker (every 500ms when playing)
        let is_playing = self
            .player
            .as_ref()
            .is_some_and(|p| p.state() == PlaybackState::Playing);

        if is_playing {
            subs.push(Subscription::run(|| {
                iced_futures::stream::channel(1, |mut emitter| async move {
                    let mut interval = tokio::time::interval(Duration::from_millis(500));
                    loop {
                        interval.tick().await;
                        _ = emitter.send(Message::PlaybackTick).await;
                    }
                })
            }));
        }

        // MPD idle event subscriptions — one per configured MPD provider.
        //
        // Each subscription opens TWO separate TCP connections to the MPD server:
        // 1. Idle connection — stays in idle mode, streams events via ConnectionEvents
        // 2. Command connection — stored in MpdProvider for browse/search/playback
        //
        // This avoids protocol conflicts between idle mode and command execution.
        for (idx, provider) in self.mpd_providers.iter().enumerate() {
            let provider = Arc::clone(provider);
            let stream = iced_futures::stream::channel(4, |mut emitter| async move {
                loop {
                    let pid = provider.id().to_string();

                    // Step 1: Establish the idle connection
                    // We must keep `_idle_client` alive — dropping it would
                    // close the background task and end the `events` stream.
                    let idle_result = provider.connect_idle().await;
                    let (_idle_client, mut events) = match idle_result {
                        Ok(pair) => pair,
                        Err(e) => {
                            _ = emitter
                                .send(Message::MpdConnectionFailed(pid, e.to_string()))
                                .await;
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    };

                    // Step 2: Establish the command connection
                    if let Err(e) = provider.connect_command().await {
                        log::error!(
                            "MPD provider '{pid}' command connection failed: {e}"
                        );
                        _ = emitter
                            .send(Message::MpdConnectionFailed(pid, e.to_string()))
                            .await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }

                    // Both connections established
                    _ = emitter.send(Message::MpdConnected(pid.clone())).await;

                    // Loop on idle events until the connection drops
                    while let Some(_event) = events.next().await {
                        _ = emitter.send(Message::MpdIdleEvent(pid.clone())).await;
                    }

                    // Idle stream ended — connection lost. Disconnect command too.
                    log::warn!("MPD provider '{pid}' connection lost, reconnecting...");
                    provider.disconnect().await;

                    // Backoff before reconnect
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
            subs.push(Subscription::run_with_id(("mpd-idle", idx), stream));
        }

        Subscription::batch(subs)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::LaunchUrl(url) => {
                open::that_detached(&url).ok();
            }

            Message::ToggleContextPage(page) => {
                if self.context_page == page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = page;
                    self.core.window.show_context = true;
                }
            }

            Message::Surface(_) => {}

            // -- Library --
            Message::ScanLibrary => {
                if let Some(provider) = self.registry.active_shared() {
                    match provider.provider_type() {
                        crate::provider::ProviderType::Local => {
                            self.library_scanning = true;
                            return cosmic::task::future(async move {
                                let count = tokio::task::spawn_blocking(move || {
                                    provider.sync_library().unwrap_or_else(|e| {
                                        log::error!("sync_library failed: {e}");
                                        0
                                    })
                                })
                                .await
                                .unwrap_or(0);
                                cosmic::Action::App(Message::LibraryScanComplete(count))
                            });
                        }
                        crate::provider::ProviderType::Subsonic => {
                            // Subsonic providers connect on demand — no idle subscription.
                            // Trigger a library reload directly (sets library_scanning).
                            return self.reload_library();
                        }
                        crate::provider::ProviderType::Mpd => {
                            // MPD providers: don't scan at startup; the idle
                            // subscription fires MpdConnected which triggers reload.
                            log::info!(
                                "Skipping scan for MPD provider '{}' — waiting for connection",
                                self.registry.active_id()
                            );
                        }
                    }
                }
            }

            Message::LibraryScanComplete(count) => {
                self.library_scanning = false;
                log::info!("Library scan complete: {count} tracks updated");
                // Reload library data
                return self.reload_library();
            }

            Message::LibraryLoaded {
                tracks,
                albums,
                artists,
                cover_images,
                artist_avatars,
            } => {
                self.library_scanning = false;
                self.all_tracks = tracks;
                self.all_albums = albums;
                self.all_artists = artists;
                self.cover_images = cover_images;
                self.artist_avatars = artist_avatars;
            }

            Message::LibraryBatch {
                albums,
                cover_images,
            } => {
                // Append new albums
                for album in &albums {
                    // Extract tracks from the album
                    for track in &album.tracks {
                        self.all_tracks.push(track.clone());
                    }
                    self.all_albums.push(album.clone());
                }
                // Merge cover images
                self.cover_images.extend(cover_images);

                // Rebuild artists from all albums accumulated so far
                self.rebuild_artists_from_albums();
            }

            Message::LibraryLoadComplete => {
                self.library_scanning = false;
                // Final sort
                self.all_tracks.sort_by(|a, b| a.title.cmp(&b.title));
                self.all_albums.sort_by(|a, b| a.name.cmp(&b.name));
                self.all_artists.sort_by(|a, b| a.name.cmp(&b.name));
                log::info!(
                    "Library load complete: {} albums, {} tracks, {} artists",
                    self.all_albums.len(),
                    self.all_tracks.len(),
                    self.all_artists.len()
                );
            }

            // -- Playback --
            Message::TogglePlayback => {
                if let Some(ref mut player) = self.player {
                    if player.state() == PlaybackState::Stopped && !self.all_tracks.is_empty() {
                        // If stopped, start playing first track
                        player.set_queue(self.all_tracks.clone());
                        if player.play_index(0).is_ok() {
                            self.current_track = self.all_tracks.first().cloned();
                            self.playback_position = Duration::ZERO;
                        }
                    } else if let Err(e) = player.toggle_playback() {
                        log::error!("Playback toggle failed: {e}");
                    }
                }
            }

            Message::NextTrack => {
                if let Some(ref mut player) = self.player {
                    match player.next() {
                        Ok(Some(track)) => {
                            self.current_track = Some(track.clone());
                            self.playback_position = Duration::ZERO;
                            self.lyrics_text = None;
                        }
                        Err(e) => log::error!("Next track failed: {e}"),
                        _ => {}
                    }
                }
            }

            Message::PreviousTrack => {
                if let Some(ref mut player) = self.player {
                    match player.previous() {
                        Ok(Some(track)) => {
                            self.current_track = Some(track.clone());
                            self.playback_position = Duration::ZERO;
                            self.lyrics_text = None;
                        }
                        Err(e) => log::error!("Previous track failed: {e}"),
                        _ => {}
                    }
                }
            }

            Message::SeekPreview(fraction) => {
                // Visual-only: store the preview fraction so the slider and
                // time label reflect the drag position without touching the
                // audio backend. This avoids the rapid seek storm that
                // causes stuttering, snapback, and restarts.
                self.seeking_preview = Some(fraction);
            }

            Message::SeekCommit => {
                // Mouse released on the seek slider — perform the actual seek.
                if let Some(fraction) = self.seeking_preview.take()
                    && let Some(ref mut player) = self.player
                    && let Some(ref track) = self.current_track
                {
                    let target =
                        Duration::from_secs_f32(fraction * track.duration.as_secs_f32());
                    match player.seek(target) {
                        Ok(()) => {
                            self.playback_position = target;
                        }
                        Err(e) => log::warn!("Seek failed: {e}"),
                    }
                }
            }

            Message::SetVolume(vol) => {
                if let Some(ref mut player) = self.player
                    && let Err(e) = player.set_volume(vol) {
                        log::error!("Set volume failed: {e}");
                    }
                self.config.volume = vol;
            }

            Message::ToggleShuffle => {
                self.config.shuffle = !self.config.shuffle;
            }

            Message::CycleRepeat => {
                self.config.repeat_mode = self.config.repeat_mode.next();
            }

            Message::PlaybackTick => {
                // Read accurate position from the active backend.
                if let Some(ref mut player) = self.player {
                    // Don't overwrite playback_position while the user is
                    // dragging the seek slider — the preview fraction is
                    // shown instead, and we seek only on release.
                    if self.seeking_preview.is_none() {
                        self.playback_position = player.position();

                        // Clamp to track duration to avoid overshooting.
                        if let Some(ref track) = self.current_track
                            && self.playback_position > track.duration
                        {
                            self.playback_position = track.duration;
                        }
                    }

                    // Check if track ended
                    if player.is_finished().unwrap_or(false) {
                        // Auto-advance
                        if let Ok(Some(track)) = player.next() {
                            self.current_track = Some(track.clone());
                            self.playback_position = Duration::ZERO;
                            self.lyrics_text = None;
                            self.scrobble_now_playing_sent = false;
                            self.scrobble_sent = false;
                        }
                    }
                }

                // Subsonic scrobbling: send "now playing" on first tick,
                // scrobble at 50% or 4 minutes (whichever is first).
                // Clone track to avoid borrow conflict with &mut self.
                if let Some(track) = self.current_track.clone() {
                    self.handle_scrobble(track);
                }
            }

            // -- Track selection --
            Message::PlayTrackIndex(index) => {
                self.play_track_list(&self.all_tracks.clone(), index);
            }

            Message::PlayAlbum(album_idx) => {
                if let Some(album) = self.all_albums.get(album_idx) {
                    let tracks = album.tracks.clone();
                    self.play_track_list(&tracks, 0);
                }
            }

            Message::PlayAlbumTrack(album_idx, track_idx) => {
                if let Some(album) = self.all_albums.get(album_idx) {
                    let tracks = album.tracks.clone();
                    self.play_track_list(&tracks, track_idx);
                }
            }

            Message::PlayArtistAlbum(artist_idx, album_idx) => {
                if let Some(artist) = self.all_artists.get(artist_idx)
                    && let Some(album) = artist.albums.get(album_idx) {
                        let tracks = album.tracks.clone();
                        self.play_track_list(&tracks, 0);
                    }
            }

            Message::PlayArtistTrack(artist_idx, album_idx, track_idx) => {
                if let Some(artist) = self.all_artists.get(artist_idx)
                    && let Some(album) = artist.albums.get(album_idx) {
                        let tracks = album.tracks.clone();
                        self.play_track_list(&tracks, track_idx);
                    }
            }

            // -- View navigation --
            Message::SelectAlbum(idx) => {
                self.selected_album = Some(idx);
            }

            Message::BackToAlbumGrid => {
                self.selected_album = None;
            }

            Message::SelectArtist(idx) => {
                self.selected_artist = Some(idx);
            }

            Message::BackToArtistList => {
                self.selected_artist = None;
            }

            Message::SortSongs(field) => {
                self.songs_sort = field;
                self.sort_tracks(field);
            }

            // -- Lyrics --
            Message::ShowLyrics => {
                self.context_page = ContextPage::Lyrics;
                self.core.window.show_context = true;

                // Try to load embedded lyrics
                if let Some(ref track) = self.current_track {
                    self.lyrics_text = LyricsProvider::from_tags(&track.path)
                        .or_else(|| LyricsProvider::from_lrc_file(&track.path));
                }
            }

            Message::FetchLyricsOnline => {
                if let Some(ref track) = self.current_track {
                    self.lyrics_loading = true;
                    let artist = track.artist.clone();
                    let title = track.title.clone();
                    return cosmic::task::future(async move {
                        let result = LyricsProvider::fetch_online(&artist, &title).await;
                        cosmic::Action::App(Message::LyricsLoaded(result))
                    });
                }
            }

            Message::LyricsLoaded(text) => {
                self.lyrics_loading = false;
                self.lyrics_text = text;
            }

            // -- Equalizer --
            Message::EqSetBand(index, value) => {
                if index < self.config.equalizer_bands.len() {
                    self.config.equalizer_bands[index] = value.clamp(-12.0, 12.0);
                }
                self.eq_preset = None;
            }

            Message::EqSetPreset(preset) => {
                let gains = preset.gains();
                self.config.equalizer_bands = gains.to_vec();
                self.eq_preset = Some(preset);
            }

            Message::EqToggle(enabled) => {
                self.config.equalizer_enabled = enabled;
            }

            // -- Settings --
            Message::AddMusicDir => {
                // TODO: Open a directory picker dialog
                log::info!("Add music directory requested");
            }

            Message::UpdateConfig(config) => {
                self.config = config;
            }

            // -- Provider switching --
            Message::SwitchProvider(index) => {
                if let Some((id, _name)) = self.provider_list.get(index)
                    && self.registry.set_active(id)
                {
                    self.active_provider_index = Some(index);
                    log::info!("Switched to provider: {id}");

                    // Recreate player with the correct backend for the new provider.
                    self.recreate_player();

                    // Clear current library data and reload from new provider
                    self.all_tracks.clear();
                    self.all_albums.clear();
                    self.all_artists.clear();
                    self.cover_images.clear();
                    self.artist_avatars.clear();
                    return self.reload_library();
                }
            }

            // -- MPD server configuration --
            Message::MpdAddServer => {
                let idx = self.mpd_edit_states.len();
                self.mpd_edit_states
                    .push(providers::MpdEditState::new_default(idx));
                self.mpd_connection_status.push(None);
            }

            Message::MpdEditName(i, v) => {
                if let Some(state) = self.mpd_edit_states.get_mut(i) {
                    state.name = v;
                }
            }

            Message::MpdEditHost(i, v) => {
                if let Some(state) = self.mpd_edit_states.get_mut(i) {
                    state.host = v;
                }
            }

            Message::MpdEditPort(i, v) => {
                if let Some(state) = self.mpd_edit_states.get_mut(i) {
                    state.port = v;
                }
            }

            Message::MpdEditPassword(i, v) => {
                if let Some(state) = self.mpd_edit_states.get_mut(i) {
                    state.password = v;
                }
            }

            Message::MpdSaveServer(i) => {
                if let Some(state) = self.mpd_edit_states.get(i) {
                    let entry = state.to_config();
                    // Update or add in the config
                    if i < self.config.mpd_servers.len() {
                        self.config.mpd_servers[i] = entry;
                    } else {
                        self.config.mpd_servers.push(entry);
                    }
                    log::info!("MPD server config saved: {}", state.name);
                    // Persist config via cosmic-config
                    self.save_config();
                    // Re-initialize providers
                    return self.reinit_mpd_providers();
                }
            }

            Message::MpdRemoveServer(i) => {
                if i < self.mpd_edit_states.len() {
                    self.mpd_edit_states.remove(i);
                    self.mpd_connection_status.remove(i);
                    if i < self.config.mpd_servers.len() {
                        self.config.mpd_servers.remove(i);
                    }
                    log::info!("MPD server removed at index {i}");
                    self.save_config();
                    return self.reinit_mpd_providers();
                }
            }

            Message::MpdTestConnection(i) => {
                if let Some(state) = self.mpd_edit_states.get(i) {
                    let host = state.host.clone();
                    let port: u16 = state.port.parse().unwrap_or(6600);
                    let password = if state.password.is_empty() {
                        None
                    } else {
                        Some(state.password.clone())
                    };

                    return cosmic::task::future(async move {
                        let addr = format!("{host}:{port}");
                        let result = async {
                            let stream = tokio::net::TcpStream::connect(&addr)
                                .await
                                .map_err(|e| format!("TCP: {e}"))?;
                            mpd_client::Client::connect_with_password_opt(
                                stream,
                                password.as_deref(),
                            )
                            .await
                            .map_err(|e| format!("MPD: {e}"))?;
                            Ok(())
                        }
                        .await;
                        cosmic::Action::App(Message::MpdTestResult(i, result))
                    });
                }
            }

            Message::MpdTestResult(i, result) => {
                let status = match result {
                    Ok(()) => fl!("connected"),
                    Err(e) => format!("{}: {e}", fl!("connection-failed")),
                };
                if let Some(s) = self.mpd_connection_status.get_mut(i) {
                    *s = Some(status);
                }
            }

            // -- MPD provider events --
            Message::MpdConnected(provider_id) => {
                log::info!("MPD provider '{provider_id}' is now connected");

                // Update connection status for the matching provider card.
                if let Some(idx) = self
                    .mpd_edit_states
                    .iter()
                    .position(|s| s.id == provider_id)
                    && let Some(s) = self.mpd_connection_status.get_mut(idx)
                {
                    *s = Some(fl!("connected"));
                }

                // If this is the active provider, recreate the player with MpdBackend
                // and reload the library.
                if self.registry.active_id() == provider_id {
                    self.recreate_player();
                    return self.reload_library();
                }
            }

            Message::MpdConnectionFailed(provider_id, error) => {
                log::error!("MPD provider '{provider_id}' failed to connect: {error}");

                // Update connection status for the matching provider card.
                if let Some(idx) = self
                    .mpd_edit_states
                    .iter()
                    .position(|s| s.id == provider_id)
                    && let Some(s) = self.mpd_connection_status.get_mut(idx)
                {
                    *s = Some(format!("{}: {error}", fl!("connection-failed")));
                }
            }

            Message::MpdIdleEvent(provider_id) => {
                log::debug!("MPD idle event from provider '{provider_id}'");
                // If this is the active provider, reload the library to pick up changes
                if self.registry.active_id() == provider_id {
                    return self.reload_library();
                }
            }

            // -- Subsonic server configuration --
            Message::SubsonicAddServer => {
                let idx = self.subsonic_edit_states.len();
                self.subsonic_edit_states
                    .push(providers::SubsonicEditState::new_default(idx));
                self.subsonic_connection_status.push(None);
            }

            Message::SubsonicEditName(i, v) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.name = v;
                }
            }

            Message::SubsonicEditUrl(i, v) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.url = v;
                }
            }

            Message::SubsonicEditUsername(i, v) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.username = v;
                }
            }

            Message::SubsonicEditPassword(i, v) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.password = v;
                }
            }

            Message::SubsonicToggleCerts(i, v) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.accept_invalid_certs = v;
                }
            }

            Message::SubsonicSaveServer(i) => {
                if let Some(state) = self.subsonic_edit_states.get(i) {
                    let entry = state.to_config();
                    if i < self.config.subsonic_servers.len() {
                        self.config.subsonic_servers[i] = entry;
                    } else {
                        self.config.subsonic_servers.push(entry);
                    }
                    log::info!("Subsonic server config saved: {}", state.name);
                    self.save_config();
                    return self.reinit_subsonic_providers();
                }
            }

            Message::SubsonicRemoveServer(i) => {
                if i < self.subsonic_edit_states.len() {
                    self.subsonic_edit_states.remove(i);
                    self.subsonic_connection_status.remove(i);
                    if i < self.config.subsonic_servers.len() {
                        self.config.subsonic_servers.remove(i);
                    }
                    log::info!("Subsonic server removed at index {i}");
                    self.save_config();
                    return self.reinit_subsonic_providers();
                }
            }

            Message::SubsonicTestConnection(i) => {
                if let Some(state) = self.subsonic_edit_states.get(i) {
                    let url = state.url.clone();
                    let username = state.username.clone();
                    let password = state.password.clone();
                    let accept_invalid_certs = state.accept_invalid_certs;

                    return cosmic::task::future(async move {
                        let result = async {
                            let auth = opensubsonic::Auth::token(&password);
                            let mut client = opensubsonic::Client::new(&url, &username, auth)
                                .map_err(|e| format!("Client: {e}"))?;
                            if accept_invalid_certs {
                                client = client
                                    .with_danger_accept_invalid_certs()
                                    .map_err(|e| format!("TLS: {e}"))?;
                            }
                            client
                                .ping()
                                .await
                                .map_err(|e| format!("Ping: {e}"))?;
                            Ok(())
                        }
                        .await;
                        cosmic::Action::App(Message::SubsonicTestResult(i, result))
                    });
                }
            }

            Message::SubsonicTestResult(i, result) => {
                let status = match result {
                    Ok(()) => fl!("connected"),
                    Err(e) => format!("{}: {e}", fl!("connection-failed")),
                };
                if let Some(s) = self.subsonic_connection_status.get_mut(i) {
                    *s = Some(status);
                }
            }

            Message::Quit => {
                return cosmic::iced::exit();
            }
        }

        Task::none()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<cosmic::Action<Self::Message>> {
        self.nav.activate(id);
        // Reset sub-view selections when switching pages
        self.selected_album = None;
        self.selected_artist = None;
        self.update_title()
    }
}

impl AppModel {
    fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
        let mut title = fl!("app-title");

        if let Some(page) = self.nav.text(self.nav.active()) {
            title.push_str(" — ");
            title.push_str(page);
        }

        if let Some(id) = self.core.main_window_id() {
            self.set_window_title(title, id)
        } else {
            Task::none()
        }
    }

    fn reload_library(&mut self) -> Task<cosmic::Action<Message>> {
        let provider = match self.registry.active_shared() {
            Some(p) => p,
            None => return Task::none(),
        };
        let provider_type = provider.provider_type();

        match provider_type {
            crate::provider::ProviderType::Local => self.reload_library_local(provider),
            crate::provider::ProviderType::Mpd | crate::provider::ProviderType::Subsonic => {
                // Clear existing data before incremental loading begins.
                self.all_tracks.clear();
                self.all_albums.clear();
                self.all_artists.clear();
                self.cover_images.clear();
                self.artist_avatars.clear();
                self.library_scanning = true;
                self.reload_library_incremental(provider, provider_type)
            }
        }
    }

    /// Single-shot library reload for the local provider (reads from local DB).
    fn reload_library_local(
        &self,
        provider: Arc<dyn MusicProvider + Send + Sync>,
    ) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let provider_clone = Arc::clone(&provider);
            let (tracks, albums, artists) = tokio::task::spawn_blocking(move || {
                let tracks = provider_clone.browse_tracks().unwrap_or_else(|e| {
                    log::error!("browse_tracks failed: {e}");
                    Vec::new()
                });
                let albums = provider_clone.browse_albums().unwrap_or_else(|e| {
                    log::error!("browse_albums failed: {e}");
                    Vec::new()
                });
                let artists = provider_clone.browse_artists().unwrap_or_else(|e| {
                    log::error!("browse_artists failed: {e}");
                    Vec::new()
                });
                (tracks, albums, artists)
            })
            .await
            .unwrap_or_default();

            // Extract cover art
            let mut cover_images = HashMap::new();
            for album in &albums {
                let key = crate::library::CoverArt::album_key(&album.artist, &album.name);
                if let Some(first_track) = album.tracks.first()
                    && let Some(bytes) =
                        crate::library::CoverArt::get_cover_art(&first_track.path)
                {
                    let handle = widget::icon::from_raster_bytes(bytes);
                    cover_images.insert(key, handle);
                }
            }

            let mut artist_avatars = HashMap::new();
            for artist in &artists {
                let bytes = crate::library::CoverArt::generate_artist_avatar(&artist.name, 64);
                let handle = widget::icon::from_raster_bytes(bytes);
                artist_avatars.insert(artist.name.clone(), handle);
            }

            cosmic::Action::App(Message::LibraryLoaded {
                tracks,
                albums,
                artists,
                cover_images,
                artist_avatars,
            })
        })
    }

    /// Incremental library reload for remote providers (MPD, Subsonic).
    ///
    /// Fetches albums in batches and sends a `LibraryBatch` message after
    /// each batch so the UI populates progressively. Cover art for each
    /// batch is fetched inline. Finishes with `LibraryLoadComplete`.
    fn reload_library_incremental(
        &self,
        provider: Arc<dyn MusicProvider + Send + Sync>,
        provider_type: crate::provider::ProviderType,
    ) -> Task<cosmic::Action<Message>> {
        // Downcast to concrete provider types for paged access.
        // We clone the Arc'd provider references from self.
        let mpd_providers = self.mpd_providers.clone();
        let subsonic_providers = self.subsonic_providers.clone();
        let active_id = self.registry.active_id().to_string();

        let stream = iced_futures::stream::channel(8, move |mut emitter| async move {
            const BATCH_SIZE: usize = 50;

            match provider_type {
                crate::provider::ProviderType::Mpd => {
                    // Find the matching MpdProvider by id.
                    let mpd = match mpd_providers.iter().find(|p| p.id() == active_id) {
                        Some(p) => Arc::clone(p),
                        None => return,
                    };

                    // Step 1: Get all album names (single fast command).
                    let album_names = match mpd.list_album_names().await {
                        Ok(names) => names,
                        Err(e) => {
                            log::error!("MPD list_album_names failed: {e}");
                            _ = emitter
                                .send(cosmic::Action::App(Message::LibraryLoadComplete))
                                .await;
                            return;
                        }
                    };

                    log::info!(
                        "MPD incremental load: {} albums in batches of {BATCH_SIZE}",
                        album_names.len()
                    );

                    // Step 2: Process in batches.
                    for chunk in album_names.chunks(BATCH_SIZE) {
                        let albums = match mpd.browse_albums_batch(chunk).await {
                            Ok(a) => a,
                            Err(e) => {
                                log::error!("MPD browse_albums_batch failed: {e}");
                                break;
                            }
                        };

                        // Fetch cover art for this batch.
                        let mut cover_images = HashMap::new();
                        let prov = Arc::clone(&provider);
                        for album in &albums {
                            let key = crate::library::CoverArt::album_key(
                                &album.artist,
                                &album.name,
                            );
                            let prov2 = Arc::clone(&prov);
                            let album_clone = album.clone();
                            if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || {
                                prov2.get_cover_art(&album_clone)
                            })
                            .await
                            .unwrap_or(Ok(None))
                            {
                                let handle = widget::icon::from_raster_bytes(bytes);
                                cover_images.insert(key, handle);
                            }
                        }

                        _ = emitter
                            .send(cosmic::Action::App(Message::LibraryBatch {
                                albums,
                                cover_images,
                            }))
                            .await;
                    }
                }

                crate::provider::ProviderType::Subsonic => {
                    // Find the matching SubsonicProvider by id.
                    let subsonic =
                        match subsonic_providers.iter().find(|p| p.id() == active_id) {
                            Some(p) => Arc::clone(p),
                            None => return,
                        };

                    log::info!("Subsonic incremental load: batches of {BATCH_SIZE}");

                    let mut offset: i32 = 0;
                    let page_size = BATCH_SIZE as i32;

                    loop {
                        let (albums, has_more) =
                            match subsonic.browse_albums_page(offset, page_size).await {
                                Ok(result) => result,
                                Err(e) => {
                                    log::error!("Subsonic browse_albums_page failed: {e}");
                                    break;
                                }
                            };

                        if albums.is_empty() {
                            break;
                        }

                        let batch_count = albums.len();

                        // Fetch cover art for this batch.
                        let mut cover_images = HashMap::new();
                        let prov = Arc::clone(&provider);
                        for album in &albums {
                            let key = crate::library::CoverArt::album_key(
                                &album.artist,
                                &album.name,
                            );
                            let prov2 = Arc::clone(&prov);
                            let album_clone = album.clone();
                            if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || {
                                prov2.get_cover_art(&album_clone)
                            })
                            .await
                            .unwrap_or(Ok(None))
                            {
                                let handle = widget::icon::from_raster_bytes(bytes);
                                cover_images.insert(key, handle);
                            }
                        }

                        log::debug!(
                            "Subsonic batch: offset={offset}, albums={batch_count}"
                        );

                        _ = emitter
                            .send(cosmic::Action::App(Message::LibraryBatch {
                                albums,
                                cover_images,
                            }))
                            .await;

                        if !has_more {
                            break;
                        }
                        offset += page_size;
                    }
                }

                crate::provider::ProviderType::Local => {
                    // Should not reach here — local uses reload_library_local.
                    unreachable!("Local provider should not use incremental reload");
                }
            }

            _ = emitter
                .send(cosmic::Action::App(Message::LibraryLoadComplete))
                .await;
        });

        cosmic::task::stream(stream)
    }

    /// Persist the current config via cosmic-config.
    fn save_config(&self) {
        if let Ok(context) =
            cosmic_config::Config::new(<AppModel as cosmic::Application>::APP_ID, Config::VERSION)
            && let Err(e) = self.config.write_entry(&context)
        {
            log::error!("Failed to save config: {e:?}");
        }
    }

    /// Re-initialize all MPD providers from the current config.
    ///
    /// Removes old MPD providers from the registry, creates new ones,
    /// and rebuilds the provider list for the header dropdown.
    fn reinit_mpd_providers(&mut self) -> Task<cosmic::Action<Message>> {
        // Remove existing MPD providers from registry
        self.registry.remove_by_type(crate::provider::ProviderType::Mpd);
        self.mpd_providers.clear();

        // Re-create from config
        let rt_handle = tokio::runtime::Handle::current();
        for entry in &self.config.mpd_servers {
            let mpd_config: MpdConfig = entry.clone().into();
            let provider = Arc::new(MpdProvider::new(mpd_config, rt_handle.clone()));
            self.mpd_providers.push(Arc::clone(&provider));
            self.registry
                .register(Box::new(MpdProviderWrapper(provider)));
        }

        // Rebuild provider list for the dropdown
        let provider_entries = self.registry.list();
        self.provider_list = provider_entries
            .iter()
            .map(|(id, name, _)| (id.clone(), name.clone()))
            .collect();
        self.active_provider_index = self
            .provider_list
            .iter()
            .position(|(id, _)| id == self.registry.active_id());

        // Rebuild edit states
        self.mpd_edit_states = self
            .config
            .mpd_servers
            .iter()
            .map(providers::MpdEditState::from_config)
            .collect();
        self.mpd_connection_status = vec![None; self.mpd_edit_states.len()];

        // Don't reload library here — for MPD providers, the idle
        // subscription will fire MpdConnected once connected, which
        // triggers reload. For local, reload immediately.
        if self
            .registry
            .active()
            .is_some_and(|p| p.provider_type() == crate::provider::ProviderType::Local)
        {
            self.reload_library()
        } else {
            Task::none()
        }
    }

    /// Re-initialize all Subsonic providers from the current config.
    ///
    /// Removes old Subsonic providers from the registry, creates new ones,
    /// and rebuilds the provider list for the header dropdown.
    fn reinit_subsonic_providers(&mut self) -> Task<cosmic::Action<Message>> {
        // Remove existing Subsonic providers from registry
        self.registry
            .remove_by_type(crate::provider::ProviderType::Subsonic);
        self.subsonic_providers.clear();

        // Re-create from config
        let rt_handle = tokio::runtime::Handle::current();
        for entry in &self.config.subsonic_servers {
            let subsonic_config: SubsonicConfig = entry.clone().into();
            match SubsonicProvider::new(subsonic_config, rt_handle.clone()) {
                Ok(provider) => {
                    let provider = Arc::new(provider);
                    self.subsonic_providers.push(Arc::clone(&provider));
                    self.registry
                        .register(Box::new(SubsonicProviderWrapper(provider)));
                }
                Err(e) => {
                    log::error!("Failed to create Subsonic provider '{}': {e}", entry.name);
                }
            }
        }

        // Rebuild provider list for the dropdown
        let provider_entries = self.registry.list();
        self.provider_list = provider_entries
            .iter()
            .map(|(id, name, _)| (id.clone(), name.clone()))
            .collect();
        self.active_provider_index = self
            .provider_list
            .iter()
            .position(|(id, _)| id == self.registry.active_id());

        // Rebuild edit states
        self.subsonic_edit_states = self
            .config
            .subsonic_servers
            .iter()
            .map(providers::SubsonicEditState::from_config)
            .collect();
        self.subsonic_connection_status = vec![None; self.subsonic_edit_states.len()];

        // For Subsonic providers, we can reload the library immediately
        // since they connect on demand (no idle subscription).
        if self
            .registry
            .active()
            .is_some_and(|p| p.provider_type() == crate::provider::ProviderType::Subsonic)
        {
            self.reload_library()
        } else {
            Task::none()
        }
    }

    /// Try to create an `MpdBackend` for the currently active provider.
    ///
    /// Returns `Some(MpdBackend)` if the active provider is an MPD provider
    /// and has a connected client. Returns `None` otherwise.
    fn make_mpd_backend(&self) -> Option<MpdBackend> {
        let active_id = self.registry.active_id();
        self.mpd_providers
            .iter()
            .find(|p| p.id() == active_id)
            .and_then(|mpd| {
                let client = mpd.client_clone()?;
                let handle = mpd.runtime_handle();
                Some(MpdBackend::new(client, handle))
            })
    }

    /// Recreate the Player with the appropriate backend for the current provider.
    fn recreate_player(&mut self) {
        let mpd_backend = self.make_mpd_backend();
        match Player::new(mpd_backend) {
            Ok(p) => self.player = Some(p),
            Err(e) => {
                log::error!("Failed to recreate player: {e}");
                self.player = None;
            }
        }
    }

    /// Handle scrobble logic for the current track.
    ///
    /// Sends a "now playing" notification on first call for a track, then
    /// scrobbles when playback reaches 50% of duration or 4 minutes
    /// (whichever comes first). Only applies to Subsonic tracks.
    fn handle_scrobble(&mut self, track: Track) {
        // Only scrobble Subsonic tracks.
        let provider = match self
            .subsonic_providers
            .iter()
            .find(|p| p.id() == track.provider_id)
        {
            Some(p) => Arc::clone(p),
            None => return,
        };

        // The Subsonic song ID is stored in track.path (set by child_to_track).
        let song_id = track.path.to_string_lossy().to_string();

        // Send "now playing" notification once per track.
        if !self.scrobble_now_playing_sent {
            self.scrobble_now_playing_sent = true;
            provider.now_playing(&song_id);
        }

        // Scrobble at 50% of duration or 4 minutes, whichever is first.
        if !self.scrobble_sent {
            let half_duration = track.duration / 2;
            let four_minutes = Duration::from_secs(240);
            let threshold = half_duration.min(four_minutes);

            if self.playback_position >= threshold && threshold > Duration::ZERO {
                self.scrobble_sent = true;
                provider.scrobble(&song_id);
            }
        }
    }

    /// Rebuild the `all_artists` list by grouping `all_albums` by artist name.
    ///
    /// Called after each incremental batch to keep the artist view in sync
    /// without requiring a separate API call.
    fn rebuild_artists_from_albums(&mut self) {
        use std::collections::BTreeMap;

        let mut artist_map: BTreeMap<String, Vec<Album>> = BTreeMap::new();
        for album in &self.all_albums {
            artist_map
                .entry(album.artist.clone())
                .or_default()
                .push(album.clone());
        }

        self.all_artists = artist_map
            .into_iter()
            .map(|(name, mut albums)| {
                albums.sort_by(|a, b| a.year.cmp(&b.year));
                // Generate avatar
                let bytes = crate::library::CoverArt::generate_artist_avatar(&name, 64);
                let handle = widget::icon::from_raster_bytes(bytes);
                self.artist_avatars.insert(name.clone(), handle);
                Artist { name, albums }
            })
            .collect();
    }

    fn play_track_list(&mut self, tracks: &[Track], start_index: usize) {
        if let Some(ref mut player) = self.player {
            player.set_queue(tracks.to_vec());
            if player.play_index(start_index).is_ok() {
                self.current_track = tracks.get(start_index).cloned();
                self.playback_position = Duration::ZERO;
                self.lyrics_text = None;
                self.scrobble_now_playing_sent = false;
                self.scrobble_sent = false;
            }
        }
    }

    fn sort_tracks(&mut self, field: songs::SortField) {
        match field {
            songs::SortField::Title => self.all_tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            songs::SortField::Artist => self.all_tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            songs::SortField::Album => self.all_tracks.sort_by(|a, b| a.album.cmp(&b.album)),
            songs::SortField::Duration => {
                self.all_tracks.sort_by(|a, b| a.duration.cmp(&b.duration))
            }
        }
    }
}

/// Navigation pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Albums,
    Artists,
    Songs,
}

/// Context drawer pages.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
    Equalizer,
    Lyrics,
    Providers,
}

/// Menu bar actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Equalizer,
    Providers,
    ScanLibrary,
    AddMusicDir,
    Quit,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Equalizer => Message::ToggleContextPage(ContextPage::Equalizer),
            MenuAction::Providers => Message::ToggleContextPage(ContextPage::Providers),
            MenuAction::ScanLibrary => Message::ScanLibrary,
            MenuAction::AddMusicDir => Message::AddMusicDir,
            MenuAction::Quit => Message::Quit,
        }
    }
}
