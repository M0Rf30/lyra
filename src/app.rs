// SPDX-License-Identifier: GPL-3.0

use crate::config::Config;
use crate::fl;
use crate::library::{Album, Artist, LibraryDb, LibraryScanner, LyricsProvider, Track};
use crate::player::mpd_backend::MpdBackend;
use crate::player::{ActiveBackend, PlaybackState, Player};
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] =
    include_bytes!("../resources/icons/hicolor/scalable/apps/io.github.m0rf30.Lyra.svg");

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

    // Expanded now-playing view
    /// Raw cover art bytes keyed by album_key, for blur processing.
    cover_art_bytes: HashMap<String, Vec<u8>>,
    /// Cached blurred cover art for the current album.
    blurred_cover: Option<widget::icon::Handle>,
    /// Album key for the cached blurred cover.
    blurred_cover_key: Option<String>,
    /// 0.0 = fully collapsed (compact bar), 1.0 = fully expanded.
    expand_progress: f32,
    /// Animation target: 0.0 for collapsing, 1.0 for expanding. None when idle.
    expand_target: Option<f32>,
    /// Timestamp when the current animation started.
    expand_anim_start: Option<std::time::Instant>,
    /// Progress value when the current animation started (for reversals).
    expand_anim_from: f32,

    // ProjectM visualizer (behind feature flag)
    #[cfg(feature = "visualizer")]
    visualizer_active: bool,
    /// Shared frame buffer for the shader-based visualizer widget.
    /// The render subscription writes RGBA pixels here; the Shader widget
    /// reads them in its `prepare()` method via `queue.write_texture()`.
    #[cfg(feature = "visualizer")]
    viz_frame_buf: Arc<Mutex<crate::views::now_playing::viz_shader::VizFrameBuffer>>,
    #[cfg(feature = "visualizer")]
    pcm_buffer: Option<Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>>,
    /// Shared flag to signal preset change to the render thread.
    #[cfg(feature = "visualizer")]
    next_preset_signal: Arc<std::sync::atomic::AtomicBool>,
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
        /// Raw cover art bytes for blur processing.
        cover_art_bytes: HashMap<String, Vec<u8>>,
    },
    /// Incremental batch of albums from a remote provider (e.g. Subsonic).
    /// Each batch appends albums, derives tracks/artists, and updates the UI.
    LibraryBatch {
        albums: Vec<Album>,
        cover_images: HashMap<String, widget::icon::Handle>,
        /// Raw cover art bytes for blur processing.
        cover_art_bytes: HashMap<String, Vec<u8>>,
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
    /// Polled status from the active MPD backend (position, duration, state, volume).
    MpdStatusUpdate {
        position: Duration,
        duration: Duration,
        state: PlaybackState,
        volume: f32,
    },
    /// An async MPD command failed — log and let the next poll self-correct.
    MpdCommandError(String),

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

    // Expanded now-playing view
    ExpandNowPlaying,
    CollapseNowPlaying,
    ExpandAnimTick,
    /// Blurred cover art is ready (album_key, blurred handle).
    BlurReady(String, widget::icon::Handle),

    // Visualizer (behind feature flag)
    #[cfg(feature = "visualizer")]
    ToggleVisualizer,
    #[cfg(feature = "visualizer")]
    NextVisualizerPreset,
    /// A new visualizer frame was written to the shared VizFrameBuffer.
    /// This message carries no data — it just triggers a view redraw.
    #[cfg(feature = "visualizer")]
    VisualizerFrameReady,

    // Application lifecycle
    Quit,
}

impl From<albums::AlbumMessage> for Message {
    fn from(msg: albums::AlbumMessage) -> Self {
        match msg {
            albums::AlbumMessage::PlayAlbum(i) => Message::PlayAlbum(i),
            albums::AlbumMessage::PlayTrack(ai, ti) => Message::PlayAlbumTrack(ai, ti),
            albums::AlbumMessage::SelectAlbum(i) => Message::SelectAlbum(i),
            albums::AlbumMessage::BackToGrid => Message::BackToAlbumGrid,
        }
    }
}

impl From<artists::ArtistMessage> for Message {
    fn from(msg: artists::ArtistMessage) -> Self {
        match msg {
            artists::ArtistMessage::PlayArtistAlbum(ai, ali) => Message::PlayArtistAlbum(ai, ali),
            artists::ArtistMessage::PlayTrack(ai, ali, ti) => {
                Message::PlayArtistTrack(ai, ali, ti)
            }
            artists::ArtistMessage::SelectArtist(i) => Message::SelectArtist(i),
            artists::ArtistMessage::BackToList => Message::BackToArtistList,
        }
    }
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.m0rf30.Lyra";

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
            .join("lyra")
            .join("library.db");

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let mut registry = ProviderRegistry::new();
        if let Ok(db) = LibraryDb::open(&db_path) {
            let local = LocalProvider::new(db, config.music_dirs.clone());
            registry.register(Arc::new(local));
        } else {
            tracing::error!("Failed to open library database");
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
            registry.register(Arc::clone(&provider) as Arc<dyn MusicProvider>);
        }

        // Initialize Subsonic providers from config.
        let mut subsonic_providers = Vec::new();
        for entry in &config.subsonic_servers {
            let subsonic_config: SubsonicConfig = entry.clone().into();
            match SubsonicProvider::new(subsonic_config, rt_handle.clone()) {
                Ok(provider) => {
                    let provider = Arc::new(provider);
                    subsonic_providers.push(Arc::clone(&provider));
                    registry.register(Arc::clone(&provider) as Arc<dyn MusicProvider>);
                }
                Err(e) => {
                    tracing::error!("Failed to create Subsonic provider '{}': {e}", entry.name);
                }
            }
        }

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
        #[allow(unused_mut)]
        let mut player = match Player::new(None) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("Failed to initialize audio player: {e}");
                None
            }
        };

        // Create shared PCM buffer for visualizer audio tapping
        #[cfg(feature = "visualizer")]
        let pcm_buffer = {
            let buf = Arc::new(Mutex::new(
                crate::views::now_playing::visualizer::PcmBuffer::new(8192),
            ));
            if let Some(ref mut p) = player {
                p.set_pcm_buffer(Arc::clone(&buf));
            }
            Some(buf)
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
            provider_list: Vec::new(),
            active_provider_index: None,
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
            cover_art_bytes: HashMap::new(),
            blurred_cover: None,
            blurred_cover_key: None,
            expand_progress: 0.0,
            expand_target: None,
            expand_anim_start: None,
            expand_anim_from: 0.0,
            #[cfg(feature = "visualizer")]
            visualizer_active: false,
            #[cfg(feature = "visualizer")]
            viz_frame_buf: {
                let (w, h) = crate::views::now_playing::visualizer::ProjectMRenderer::resolution();
                Arc::new(Mutex::new(
                    crate::views::now_playing::viz_shader::VizFrameBuffer::new(w, h),
                ))
            },
            #[cfg(feature = "visualizer")]
            pcm_buffer,
            #[cfg(feature = "visualizer")]
            next_preset_signal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        app.rebuild_provider_list();
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
                        albums::album_detail_view(album, album_idx, &self.cover_images)
                            .map(Message::from)
                    } else {
                        widget::text("Album not found").into()
                    }
                } else {
                    albums::album_grid_view(&self.all_albums, &self.cover_images)
                        .map(Message::from)
                }
            }

            Page::Artists => {
                if let Some(artist_idx) = self.selected_artist {
                    if let Some(artist) = self.all_artists.get(artist_idx) {
                        artists::artist_detail_view(
                            artist,
                            artist_idx,
                            &self.artist_avatars,
                            &self.cover_images,
                        )
                        .map(Message::from)
                    } else {
                        widget::text("Artist not found").into()
                    }
                } else {
                    artists::artist_list_view(&self.all_artists, &self.artist_avatars)
                        .map(Message::from)
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
            // Use album_artist to match how albums store cover art.
            // Falls back to track.artist when album_artist is empty.
            let artist = if track.album_artist.is_empty() {
                &track.artist
            } else {
                &track.album_artist
            };
            let key = crate::library::CoverArt::album_key(artist, &track.album);
            self.cover_images.get(&key)
        });

        // Helper closure to map NowPlayingMessage to Message
        let map_now_playing_msg = |msg| match msg {
            now_playing::NowPlayingMessage::TogglePlayback => Message::TogglePlayback,
            now_playing::NowPlayingMessage::Next => Message::NextTrack,
            now_playing::NowPlayingMessage::Previous => Message::PreviousTrack,
            now_playing::NowPlayingMessage::SeekPreview(v) => Message::SeekPreview(v),
            now_playing::NowPlayingMessage::SeekCommit => Message::SeekCommit,
            now_playing::NowPlayingMessage::SetVolume(v) => Message::SetVolume(v),
            now_playing::NowPlayingMessage::ToggleShuffle => Message::ToggleShuffle,
            now_playing::NowPlayingMessage::CycleRepeat => Message::CycleRepeat,
            now_playing::NowPlayingMessage::ShowLyrics => Message::ShowLyrics,
            now_playing::NowPlayingMessage::ExpandToggle => Message::ExpandNowPlaying,
            now_playing::NowPlayingMessage::Collapse => Message::CollapseNowPlaying,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::ToggleVisualizer => Message::ToggleVisualizer,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::NextPreset => Message::NextVisualizerPreset,
        };

        let bar = now_playing::compact_bar::playback_bar(
            self.current_track.as_ref(),
            state,
            self.playback_position,
            duration,
            volume,
            self.config.shuffle,
            self.config.repeat_mode,
            current_cover,
            self.seeking_preview,
            self.blurred_cover.as_ref(),
        )
        .map(map_now_playing_msg);

        // Main layout: content + optional scanning indicator + bottom playback bar
        // When expand_progress > 0, show expanded now-playing view replacing normal content
        let layout: Element<'_, Self::Message> = if self.expand_progress > 0.0 {
            // Expanded/animating: show expanded now-playing view
            let expanded = now_playing::expanded_view::expanded_now_playing(
                self.current_track.as_ref(),
                state,
                self.playback_position,
                duration,
                volume,
                self.config.shuffle,
                self.config.repeat_mode,
                current_cover,
                self.blurred_cover.as_ref(),
                self.seeking_preview,
                self.expand_progress,
                #[cfg(feature = "visualizer")]
                self.visualizer_active,
                #[cfg(feature = "visualizer")]
                Arc::clone(&self.viz_frame_buf),
            )
            .map(map_now_playing_msg);

            widget::container(expanded)
                .width(Length::Fill)
                .into()
        } else {
            // Collapsed state: normal layout
            let mut layout_col = widget::column().push(
                widget::container(content)
                    .width(Length::Fill)
                    .height(Length::Fill),
            );

            if self.library_scanning {
                layout_col = layout_col.push(
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

            layout_col = layout_col.push(bar);

            layout_col.into()
        };

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

        let is_mpd_active = self
            .player
            .as_ref()
            .is_some_and(|p| p.active_backend_type() == ActiveBackend::Mpd);

        if is_playing {
            if is_mpd_active {
                // MPD: poll status from the server every 300ms.
                // This replaces the generic tick for MPD playback — we get
                // position/duration/state/volume from the real MPD status.
                if let Some(ref player) = self.player
                    && let Some(mpd) = player.mpd_backend_ref()
                {
                    let client = mpd.client();
                    subs.push(Subscription::run_with_id(
                        "mpd-status-poll",
                        iced_futures::stream::channel(1, |mut emitter| async move {
                            let mut interval =
                                tokio::time::interval(Duration::from_millis(300));
                            loop {
                                interval.tick().await;
                                match client.command(mpd_client::commands::Status).await {
                                    Ok(status) => {
                                        let state = match status.state {
                                            mpd_client::responses::PlayState::Playing => {
                                                PlaybackState::Playing
                                            }
                                            mpd_client::responses::PlayState::Paused => {
                                                PlaybackState::Paused
                                            }
                                            mpd_client::responses::PlayState::Stopped => {
                                                PlaybackState::Stopped
                                            }
                                        };
                                        _ = emitter
                                            .send(Message::MpdStatusUpdate {
                                                position: status
                                                    .elapsed
                                                    .unwrap_or(Duration::ZERO),
                                                duration: status
                                                    .duration
                                                    .unwrap_or(Duration::ZERO),
                                                state,
                                                volume: status.volume as f32 / 100.0,
                                            })
                                            .await;
                                    }
                                    Err(e) => {
                                        tracing::warn!("MPD status poll failed: {e}");
                                    }
                                }
                            }
                        }),
                    ));
                }
            } else {
                // Local/Subsonic: simple tick for UI updates.
                subs.push(Subscription::run_with_id(
                    "playback-tick",
                    iced_futures::stream::channel(1, |mut emitter| async move {
                        let mut interval =
                            tokio::time::interval(Duration::from_millis(500));
                        loop {
                            interval.tick().await;
                            _ = emitter.send(Message::PlaybackTick).await;
                        }
                    }),
                ));
            }
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
                        tracing::error!(
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
                    tracing::warn!("MPD provider '{pid}' connection lost, reconnecting...");
                    provider.disconnect().await;

                    // Backoff before reconnect
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
            subs.push(Subscription::run_with_id(("mpd-idle", idx), stream));
        }

        // Expand/collapse animation tick (~60fps, only during transitions)
        if self.expand_target.is_some() {
            subs.push(
                cosmic::iced::time::every(Duration::from_millis(16))
                    .map(|_| Message::ExpandAnimTick),
            );
        }

        // Visualizer render subscription (~30fps, only when active and expanded)
        //
        // IMPORTANT: The projectM renderer requires a current EGL/GL context
        // which is thread-local. Tokio's multi-threaded runtime migrates
        // futures between OS threads across `.await` points, which would
        // lose the GL context and produce black/garbage frames (flickering)
        // and prevent PCM audio data from reaching projectM (no beat
        // reactivity). To avoid this, the actual render loop runs on a
        // dedicated `std::thread::spawn` OS thread. The iced subscription
        // channel only relays "frame ready" notifications from that thread
        // back to the UI.
        #[cfg(feature = "visualizer")]
        if self.visualizer_active
            && self.expand_progress > 0.0
            && let Some(ref pcm_buf) = self.pcm_buffer
        {
            let pcm = Arc::clone(pcm_buf);
            let preset_signal = Arc::clone(&self.next_preset_signal);
            let frame_buf = Arc::clone(&self.viz_frame_buf);
            subs.push(Subscription::run_with_id(
                "projectm-render",
                iced_futures::stream::channel(2, move |mut emitter| async move {
                    // Use a one-shot channel to know when the render thread
                    // has produced a new frame so we can notify the UI.
                    let (frame_tx, mut frame_rx) =
                        tokio::sync::mpsc::channel::<()>(2);

                    // Spawn a dedicated OS thread for the GL render loop.
                    // The EGL context created inside `ProjectMRenderer::new`
                    // stays current for the lifetime of this thread.
                    std::thread::Builder::new()
                        .name("projectm-render".into())
                        .spawn(move || {
                            let preset_dir =
                                dirs::data_dir().map(|d| d.join("projectm").join("presets"));
                            let mut renderer =
                                match crate::views::now_playing::visualizer::ProjectMRenderer::new(
                                    preset_dir,
                                ) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to create projectM renderer: {e}"
                                        );
                                        return;
                                    }
                                };

                            loop {
                                // ~30 fps
                                std::thread::sleep(Duration::from_millis(33));

                                // Check if a preset change was requested
                                if preset_signal
                                    .swap(false, std::sync::atomic::Ordering::AcqRel)
                                {
                                    renderer.next_preset();
                                }

                                // Read PCM from shared buffer
                                let pcm_data = pcm
                                    .lock()
                                    .ok()
                                    .map(|buf| buf.read_recent(2048))
                                    .unwrap_or_default();

                                // Render a frame (GL calls, ~3-5ms)
                                let rgba = renderer.render_frame(&pcm_data);

                                // Write pixels into the shared frame buffer
                                if let Ok(mut buf) = frame_buf.lock() {
                                    buf.update(rgba);
                                }

                                // Notify the async side that a frame is ready.
                                // If the channel is full or closed, the render
                                // thread is ahead of the UI — just skip.
                                if frame_tx.try_send(()).is_err() {
                                    // Channel closed → subscription was dropped
                                    // (visualizer deactivated or view collapsed).
                                    if frame_tx.is_closed() {
                                        break;
                                    }
                                }
                            }
                        })
                        .expect("failed to spawn projectm-render thread");

                    // Relay frame-ready signals from the render thread to iced.
                    while frame_rx.recv().await.is_some() {
                        _ = emitter.send(Message::VisualizerFrameReady).await;
                    }
                }),
            ));
        }

        // Escape key to collapse expanded view
        if self.expand_progress > 0.0 || self.expand_target.is_some() {
            subs.push(cosmic::iced::event::listen_with(
                |event, _status, _id| {
                    if let cosmic::iced::Event::Keyboard(
                        cosmic::iced::keyboard::Event::KeyPressed {
                            key:
                                cosmic::iced::keyboard::Key::Named(
                                    cosmic::iced::keyboard::key::Named::Escape,
                                ),
                            ..
                        },
                    ) = event
                    {
                        Some(Message::CollapseNowPlaying)
                    } else {
                        None
                    }
                },
            ));
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
                                        tracing::error!("sync_library failed: {e}");
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
                            tracing::info!(
                                "Skipping scan for MPD provider '{}' — waiting for connection",
                                self.registry.active_id()
                            );
                        }
                    }
                }
            }

            Message::LibraryScanComplete(count) => {
                self.library_scanning = false;
                tracing::info!("Library scan complete: {count} tracks updated");
                // Reload library data
                return self.reload_library();
            }

            Message::LibraryLoaded {
                tracks,
                albums,
                artists,
                cover_images,
                artist_avatars,
                cover_art_bytes,
            } => {
                self.library_scanning = false;
                self.all_tracks = tracks;
                self.all_albums = albums;
                self.all_artists = artists;
                self.cover_images = cover_images;
                self.artist_avatars = artist_avatars;
                self.cover_art_bytes = cover_art_bytes;
                // Re-trigger blur now that cover art bytes are available
                let blur_task = self.maybe_update_blurred_cover();
                return blur_task;
            }

            Message::LibraryBatch {
                albums,
                cover_images,
                cover_art_bytes,
            } => {
                // Append new albums and extract tracks
                for album in &albums {
                    for track in &album.tracks {
                        self.all_tracks.push(track.clone());
                    }
                    self.all_albums.push(album.clone());
                }
                // Merge cover images
                self.cover_images.extend(cover_images);
                // Merge cover art bytes for blur
                self.cover_art_bytes.extend(cover_art_bytes);

                // Incrementally merge only the new batch into artists
                self.merge_artists_from_batch(&albums);

                // Re-trigger blur in case the current track's cover just arrived
                let blur_task = self.maybe_update_blurred_cover();
                return blur_task;
            }

            Message::LibraryLoadComplete => {
                self.library_scanning = false;
                // Final sort
                self.all_tracks.sort_by(|a, b| a.title.cmp(&b.title));
                self.all_albums.sort_by(|a, b| a.name.cmp(&b.name));
                self.all_artists.sort_by(|a, b| a.name.cmp(&b.name));
                tracing::info!(
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
                            return self.dispatch_mpd_after_play();
                        }
                    } else {
                        let was_playing = player.state() == PlaybackState::Playing;
                        if let Err(e) = player.toggle_playback() {
                            tracing::error!("Playback toggle failed: {e}");
                        } else if let Some(client) = self.mpd_client() {
                            // Dispatch async SetPause to MPD.
                            return self.dispatch_mpd(async move {
                                client
                                    .command(mpd_client::commands::SetPause(was_playing))
                                    .await
                                    .map_err(|e| format!("MPD set_pause: {e}"))
                            });
                        }
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
                            let mpd_task = self.dispatch_mpd_after_play();
                            let blur_task = self.maybe_update_blurred_cover();
                            return Task::batch([mpd_task, blur_task]);
                        }
                        Err(e) => tracing::error!("Next track failed: {e}"),
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
                            let mpd_task = self.dispatch_mpd_after_play();
                            let blur_task = self.maybe_update_blurred_cover();
                            return Task::batch([mpd_task, blur_task]);
                        }
                        Err(e) => tracing::error!("Previous track failed: {e}"),
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
                            // Dispatch async Seek to MPD.
                            if let Some(client) = self.mpd_client() {
                                return self.dispatch_mpd(async move {
                                    client
                                        .command(mpd_client::commands::Seek(
                                            mpd_client::commands::SeekMode::Absolute(target),
                                        ))
                                        .await
                                        .map_err(|e| format!("MPD seek: {e}"))
                                });
                            }
                        }
                        Err(e) => tracing::warn!("Seek failed: {e}"),
                    }
                }
            }

            Message::SetVolume(vol) => {
                if let Some(ref mut player) = self.player
                    && let Err(e) = player.set_volume(vol)
                {
                    tracing::error!("Set volume failed: {e}");
                }
                self.config.volume = vol;
                // Dispatch async SetVolume to MPD.
                if let Some(client) = self.mpd_client() {
                    let vol_u8 = (vol.clamp(0.0, 1.0) * 100.0) as u8;
                    return self.dispatch_mpd(async move {
                        client
                            .command(mpd_client::commands::SetVolume(vol_u8))
                            .await
                            .map_err(|e| format!("MPD set_volume: {e}"))
                    });
                }
            }

            Message::ToggleShuffle => {
                self.config.shuffle = !self.config.shuffle;
            }

            Message::CycleRepeat => {
                self.config.repeat_mode = self.config.repeat_mode.next();
            }

            Message::MpdStatusUpdate {
                position,
                duration,
                state,
                volume,
            } => {
                // Feed polled status into the MPD backend cache.
                if let Some(ref mut player) = self.player {
                    if let Some(mpd) = player.mpd_backend_mut() {
                        mpd.update_status(position, duration, state, volume);
                    }

                    // Update UI position (unless user is dragging seek slider).
                    if self.seeking_preview.is_none() {
                        self.playback_position = position;
                        if let Some(ref track) = self.current_track
                            && self.playback_position > track.duration
                        {
                            self.playback_position = track.duration;
                        }
                    }

                    // Check if track ended (MPD reports Stopped after playback).
                    if player.is_finished().unwrap_or(false)
                        && let Ok(Some(track)) = player.next()
                    {
                        self.current_track = Some(track.clone());
                        self.playback_position = Duration::ZERO;
                        self.lyrics_text = None;
                        self.scrobble_now_playing_sent = false;
                        self.scrobble_sent = false;
                        // Dispatch the actual async MPD play command.
                        return self.dispatch_mpd_after_play();
                    }
                }

                // Scrobble handling for MPD tracks.
                if let Some(track) = self.current_track.clone() {
                    self.handle_scrobble(track);
                }
            }

            Message::MpdCommandError(err) => {
                tracing::error!("Async MPD command failed: {err}");
                // The next status poll will self-correct the UI state.
            }

            Message::PlaybackTick => {
                // Local/Subsonic playback — read position from the active backend.
                if let Some(ref mut player) = self.player {
                    if self.seeking_preview.is_none() {
                        self.playback_position = player.position();

                        if let Some(ref track) = self.current_track
                            && self.playback_position > track.duration
                        {
                            self.playback_position = track.duration;
                        }
                    }

                    // Check if track ended
                    if player.is_finished().unwrap_or(false)
                        && let Ok(Some(track)) = player.next()
                    {
                        self.current_track = Some(track.clone());
                        self.playback_position = Duration::ZERO;
                        self.lyrics_text = None;
                        self.scrobble_now_playing_sent = false;
                        self.scrobble_sent = false;
                    }
                }

                if let Some(track) = self.current_track.clone() {
                    self.handle_scrobble(track);
                }
            }

            // -- Track selection --
            Message::PlayTrackIndex(index) => {
                return self.play_track_list(self.all_tracks.clone(), index);
            }

            Message::PlayAlbum(album_idx) => {
                if let Some(album) = self.all_albums.get(album_idx) {
                    return self.play_track_list(album.tracks.clone(), 0);
                }
            }

            Message::PlayAlbumTrack(album_idx, track_idx) => {
                if let Some(album) = self.all_albums.get(album_idx) {
                    return self.play_track_list(album.tracks.clone(), track_idx);
                }
            }

            Message::PlayArtistAlbum(artist_idx, album_idx) => {
                if let Some(artist) = self.all_artists.get(artist_idx)
                    && let Some(album) = artist.albums.get(album_idx)
                {
                    return self.play_track_list(album.tracks.clone(), 0);
                }
            }

            Message::PlayArtistTrack(artist_idx, album_idx, track_idx) => {
                if let Some(artist) = self.all_artists.get(artist_idx)
                    && let Some(album) = artist.albums.get(album_idx)
                {
                    return self.play_track_list(album.tracks.clone(), track_idx);
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
                tracing::info!("Add music directory requested");
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
                    tracing::info!("Switched to provider: {id}");

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
                    tracing::info!("MPD server config saved: {}", state.name);
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
                    tracing::info!("MPD server removed at index {i}");
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
                tracing::info!("MPD provider '{provider_id}' is now connected");

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
                tracing::error!("MPD provider '{provider_id}' failed to connect: {error}");

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
                tracing::debug!("MPD idle event from provider '{provider_id}'");
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
                    tracing::info!("Subsonic server config saved: {}", state.name);
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
                    tracing::info!("Subsonic server removed at index {i}");
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

            Message::ExpandNowPlaying => {
                self.expand_target = Some(1.0);
                self.expand_anim_start = Some(std::time::Instant::now());
                self.expand_anim_from = self.expand_progress;
            }

            Message::CollapseNowPlaying => {
                self.expand_target = Some(0.0);
                self.expand_anim_start = Some(std::time::Instant::now());
                self.expand_anim_from = self.expand_progress;
            }

            Message::ExpandAnimTick => {
                use crate::views::now_playing::animation;

                if let (Some(target), Some(start)) =
                    (self.expand_target, self.expand_anim_start)
                {
                    let elapsed = start.elapsed().as_secs_f32() * 1000.0;
                    let t = (elapsed / animation::ANIMATION_DURATION_MS).min(1.0);

                    // Apply easing based on direction
                    let eased = if target > self.expand_anim_from {
                        animation::ease_out(t)
                    } else {
                        animation::ease_in(t)
                    };

                    self.expand_progress =
                        animation::lerp(self.expand_anim_from, target, eased);

                    // Check if animation is complete
                    if t >= 1.0 {
                        self.expand_progress = target;
                        self.expand_target = None;
                        self.expand_anim_start = None;
                    }
                }
            }

            Message::BlurReady(key, handle) => {
                self.blurred_cover = Some(handle);
                self.blurred_cover_key = Some(key);
            }

            // -- Visualizer messages (cfg-gated) --
            #[cfg(feature = "visualizer")]
            Message::ToggleVisualizer => {
                self.visualizer_active = !self.visualizer_active;
            }

            #[cfg(feature = "visualizer")]
            Message::NextVisualizerPreset => {
                self.next_preset_signal
                    .store(true, std::sync::atomic::Ordering::Release);
            }

            #[cfg(feature = "visualizer")]
            Message::VisualizerFrameReady => {
                // The shared VizFrameBuffer already has the new pixels.
                // This message just triggers a view redraw so the Shader
                // widget picks them up in its next prepare() call.
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

        // Collapse expanded now-playing view when navigating
        if self.expand_progress > 0.0 || self.expand_target.is_some() {
            self.expand_target = Some(0.0);
            self.expand_anim_start = Some(std::time::Instant::now());
            self.expand_anim_from = self.expand_progress;
        }

        self.update_title()
    }
}

impl AppModel {
    /// Rebuild `provider_list` and `active_provider_index` from the registry.
    ///
    /// Call after any change to the set of registered providers (init,
    /// reinit_mpd_providers, reinit_subsonic_providers).
    fn rebuild_provider_list(&mut self) {
        let entries = self.registry.list();
        self.provider_list = entries
            .iter()
            .map(|(id, name, _)| (id.clone(), name.clone()))
            .collect();
        self.active_provider_index = self
            .provider_list
            .iter()
            .position(|(id, _)| id == self.registry.active_id());
    }

    /// Get the MPD `Client` if the active backend is MPD.
    fn mpd_client(&self) -> Option<mpd_client::Client> {
        let player = self.player.as_ref()?;
        if player.active_backend_type() != ActiveBackend::Mpd {
            return None;
        }
        Some(player.mpd_backend_ref()?.client())
    }

    /// Dispatch an async MPD command, mapping errors to `MpdCommandError`.
    fn dispatch_mpd<F>(&self, future: F) -> Task<cosmic::Action<Message>>
    where
        F: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        cosmic::task::future(async move {
            if let Err(e) = future.await {
                cosmic::Action::App(Message::MpdCommandError(e))
            } else {
                // No-op message — command succeeded, status poll will confirm.
                cosmic::Action::App(Message::PlaybackTick)
            }
        })
    }

    /// Dispatch an async MPD play command for a URI (ClearQueue + Add + Play).
    fn dispatch_mpd_play(&self, uri: String) -> Task<cosmic::Action<Message>> {
        if let Some(client) = self.mpd_client() {
            self.dispatch_mpd(async move {
                client
                    .command(mpd_client::commands::ClearQueue)
                    .await
                    .map_err(|e| format!("MPD clear: {e}"))?;
                client
                    .command(mpd_client::commands::Add::uri(&uri))
                    .await
                    .map_err(|e| format!("MPD add: {e}"))?;
                client
                    .command(mpd_client::commands::Play::current())
                    .await
                    .map_err(|e| format!("MPD play: {e}"))?;
                Ok(())
            })
        } else {
            Task::none()
        }
    }

    /// After `Player` sets optimistic state, take the pending URI and dispatch.
    fn dispatch_mpd_after_play(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(ref mut player) = self.player
            && let Some(mpd) = player.mpd_backend_mut()
            && let Some(uri) = mpd.take_play_uri()
        {
            return self.dispatch_mpd_play(uri);
        }
        Task::none()
    }

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
                    tracing::error!("browse_tracks failed: {e}");
                    Vec::new()
                });
                let albums = provider_clone.browse_albums().unwrap_or_else(|e| {
                    tracing::error!("browse_albums failed: {e}");
                    Vec::new()
                });
                let artists = provider_clone.browse_artists().unwrap_or_else(|e| {
                    tracing::error!("browse_artists failed: {e}");
                    Vec::new()
                });
                (tracks, albums, artists)
            })
            .await
            .unwrap_or_default();

            // Extract cover art
            let mut cover_images = HashMap::new();
            let mut cover_art_bytes = HashMap::new();
            for album in &albums {
                let key = crate::library::CoverArt::album_key(&album.artist, &album.name);
                if let Some(first_track) = album.tracks.first()
                    && let Some(bytes) =
                        crate::library::CoverArt::get_cover_art(&first_track.path)
                {
                    let handle = widget::icon::from_raster_bytes(bytes.clone());
                    cover_images.insert(key.clone(), handle);
                    cover_art_bytes.insert(key, bytes);
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
                cover_art_bytes,
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
                            tracing::error!("MPD list_album_names failed: {e}");
                            _ = emitter
                                .send(cosmic::Action::App(Message::LibraryLoadComplete))
                                .await;
                            return;
                        }
                    };

                    tracing::info!(
                        "MPD incremental load: {} albums in batches of {BATCH_SIZE}",
                        album_names.len()
                    );

                    // Step 2: Process in batches.
                    for chunk in album_names.chunks(BATCH_SIZE) {
                        let albums = match mpd.browse_albums_batch(chunk).await {
                            Ok(a) => a,
                            Err(e) => {
                                tracing::error!("MPD browse_albums_batch failed: {e}");
                                break;
                            }
                        };

                        // Fetch cover art for this batch.
                        let mut cover_images = HashMap::new();
                        let mut cover_art_bytes = HashMap::new();
                        let prov = Arc::clone(&provider);
                        for album in &albums {
                            let key = crate::library::CoverArt::album_key(
                                &album.artist,
                                &album.name,
                            );
                            let prov2 = Arc::clone(&prov);
                            let hint = album.cover_hint();
                            if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || {
                                prov2.get_cover_art(&hint)
                            })
                            .await
                            .unwrap_or(Ok(None))
                            {
                                let handle = widget::icon::from_raster_bytes(bytes.clone());
                                cover_images.insert(key.clone(), handle);
                                cover_art_bytes.insert(key, bytes);
                            }
                        }

                        _ = emitter
                            .send(cosmic::Action::App(Message::LibraryBatch {
                                albums,
                                cover_images,
                                cover_art_bytes,
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

                    tracing::info!("Subsonic incremental load: batches of {BATCH_SIZE}");

                    let mut offset: i32 = 0;
                    let page_size = BATCH_SIZE as i32;

                    loop {
                        let (albums, has_more) =
                            match subsonic.browse_albums_page(offset, page_size).await {
                                Ok(result) => result,
                                Err(e) => {
                                    tracing::error!("Subsonic browse_albums_page failed: {e}");
                                    break;
                                }
                            };

                        if albums.is_empty() {
                            break;
                        }

                        let batch_count = albums.len();

                        // Fetch cover art for this batch.
                        let mut cover_images = HashMap::new();
                        let mut cover_art_bytes = HashMap::new();
                        let prov = Arc::clone(&provider);
                        for album in &albums {
                            let key = crate::library::CoverArt::album_key(
                                &album.artist,
                                &album.name,
                            );
                            let prov2 = Arc::clone(&prov);
                            let hint = album.cover_hint();
                            if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || {
                                prov2.get_cover_art(&hint)
                            })
                            .await
                            .unwrap_or(Ok(None))
                            {
                                let handle = widget::icon::from_raster_bytes(bytes.clone());
                                cover_images.insert(key.clone(), handle);
                                cover_art_bytes.insert(key, bytes);
                            }
                        }

                        tracing::debug!(
                            "Subsonic batch: offset={offset}, albums={batch_count}"
                        );

                        _ = emitter
                            .send(cosmic::Action::App(Message::LibraryBatch {
                                albums,
                                cover_images,
                                cover_art_bytes,
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
            tracing::error!("Failed to save config: {e:?}");
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
                .register(Arc::clone(&provider) as Arc<dyn MusicProvider>);
        }

        self.rebuild_provider_list();

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
                        .register(Arc::clone(&provider) as Arc<dyn MusicProvider>);
                }
                Err(e) => {
                    tracing::error!("Failed to create Subsonic provider '{}': {e}", entry.name);
                }
            }
        }

        self.rebuild_provider_list();

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
                Some(MpdBackend::new(client))
            })
    }

    /// Recreate the Player with the appropriate backend for the current provider.
    fn recreate_player(&mut self) {
        let mpd_backend = self.make_mpd_backend();
        match Player::new(mpd_backend) {
            #[allow(unused_mut)]
            Ok(mut p) => {
                // Re-wire PCM buffer for visualizer
                #[cfg(feature = "visualizer")]
                if let Some(ref buf) = self.pcm_buffer {
                    p.set_pcm_buffer(Arc::clone(buf));
                }
                self.player = Some(p);
            }
            Err(e) => {
                tracing::error!("Failed to recreate player: {e}");
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
            .find(|p| p.id() == &*track.provider_id)
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

    /// Incrementally merge new albums into `all_artists`.
    ///
    /// Only processes the `new_albums` slice (the batch that just arrived),
    /// appending to existing artists or creating new ones.  Avatars are only
    /// generated for newly-seen artist names — existing entries in
    /// `artist_avatars` are reused.
    fn merge_artists_from_batch(&mut self, new_albums: &[Album]) {
        // Build an index over the current artists list for O(1) lookup.
        let mut index: HashMap<String, usize> = self
            .all_artists
            .iter()
            .enumerate()
            .map(|(i, a)| (a.name.clone(), i))
            .collect();

        for album in new_albums {
            if let Some(&idx) = index.get(&album.artist) {
                self.all_artists[idx].albums.push(album.clone());
            } else {
                let idx = self.all_artists.len();
                index.insert(album.artist.clone(), idx);
                self.all_artists.push(Artist {
                    name: album.artist.clone(),
                    albums: vec![album.clone()],
                });

                // Generate avatar only for new artists.
                if !self.artist_avatars.contains_key(&album.artist) {
                    let bytes =
                        crate::library::CoverArt::generate_artist_avatar(&album.artist, 64);
                    let handle = widget::icon::from_raster_bytes(bytes);
                    self.artist_avatars.insert(album.artist.clone(), handle);
                }
            }
        }
    }

    /// Start playback from the given queue at `start_index`.
    ///
    /// Takes ownership of the track list to avoid an extra clone — the
    /// caller is responsible for providing an owned `Vec<Track>`.
    fn play_track_list(
        &mut self,
        tracks: Vec<Track>,
        start_index: usize,
    ) -> Task<cosmic::Action<Message>> {
        if let Some(ref mut player) = self.player {
            let current = tracks.get(start_index).cloned();
            player.set_queue(tracks);
            if player.play_index(start_index).is_ok() {
                self.current_track = current;
                self.playback_position = Duration::ZERO;
                self.lyrics_text = None;
                self.scrobble_now_playing_sent = false;
                self.scrobble_sent = false;
                let mpd_task = self.dispatch_mpd_after_play();
                let blur_task = self.maybe_update_blurred_cover();
                return Task::batch([mpd_task, blur_task]);
            }
        }
        Task::none()
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

    /// Trigger blur computation for the current track if the album changed.
    ///
    /// Checks if the current track's album key differs from the cached blurred
    /// cover key. If so, looks up the raw bytes and spawns a background task
    /// to compute the blur. Returns a Task that sends `Message::BlurReady`.
    fn maybe_update_blurred_cover(&mut self) -> Task<cosmic::Action<Message>> {
        let track = match self.current_track.as_ref() {
            Some(t) => t,
            None => {
                // No track, clear blurred cover
                self.blurred_cover = None;
                self.blurred_cover_key = None;
                return Task::none();
            }
        };

        // Use album_artist to match how albums store cover art.
        // Falls back to track.artist when album_artist is empty.
        let artist = if track.album_artist.is_empty() {
            &track.artist
        } else {
            &track.album_artist
        };
        let key = crate::library::CoverArt::album_key(artist, &track.album);

        // Skip if already computed for this album
        if self.blurred_cover_key.as_ref() == Some(&key) {
            return Task::none();
        }

        // Look up raw bytes
        let bytes = match self.cover_art_bytes.get(&key) {
            Some(b) => b.clone(),
            None => {
                // No cover art bytes available, clear blurred cover
                self.blurred_cover = None;
                self.blurred_cover_key = None;
                return Task::none();
            }
        };

        let key_clone = key.clone();
        cosmic::task::future(async move {
            // Compute blur in blocking task to avoid blocking async runtime
            let blurred = tokio::task::spawn_blocking(move || {
                crate::views::now_playing::blur::compute_blurred_cover(&bytes)
            })
            .await
            .ok()
            .flatten();

            if let Some(blurred_bytes) = blurred {
                let handle = widget::icon::from_raster_bytes(blurred_bytes);
                cosmic::Action::App(Message::BlurReady(key_clone, handle))
            } else {
                // Blur computation failed, send a no-op
                cosmic::Action::None
            }
        })
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
