// SPDX-License-Identifier: GPL-3.0

use crate::config::Config;
use crate::fl;
use crate::library::{Album, Artist, LibraryDb, LibraryScanner, LyricsProvider, Track};
use crate::player::{PlaybackState, Player};
use crate::views::{albums, artists, equalizer, lyrics, now_playing, songs};
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use cosmic::{iced_futures, prelude::*};
use futures_util::SinkExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

/// Main application model.
pub struct AppModel {
    core: cosmic::Core,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    about: About,
    config: Config,
    context_page: ContextPage,

    // Library data
    db: Option<LibraryDb>,
    all_tracks: Vec<Track>,
    all_albums: Vec<Album>,
    all_artists: Vec<Artist>,
    library_scanning: bool,

    // Player
    player: Option<Player>,
    playback_position: Duration,
    current_track: Option<Track>,

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

    // Player transport
    TogglePlayback,
    NextTrack,
    PreviousTrack,
    Seek(f32),
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

        // Open library database
        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cosmic-music-player")
            .join("library.db");

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let db = LibraryDb::open(&db_path).ok();

        // Initialize player
        let player = match Player::new() {
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
            db,
            all_tracks: Vec::new(),
            all_albums: Vec::new(),
            all_artists: Vec::new(),
            library_scanning: false,
            player,
            playback_position: Duration::ZERO,
            current_track: None,
            selected_album: None,
            selected_artist: None,
            songs_sort: songs::SortField::Title,
            cover_images: HashMap::new(),
            artist_avatars: HashMap::new(),
            lyrics_text: None,
            lyrics_loading: false,
            eq_preset: None,
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
        )
        .map(|msg| match msg {
            now_playing::NowPlayingMessage::TogglePlayback => Message::TogglePlayback,
            now_playing::NowPlayingMessage::Next => Message::NextTrack,
            now_playing::NowPlayingMessage::Previous => Message::PreviousTrack,
            now_playing::NowPlayingMessage::Seek(v) => Message::Seek(v),
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
                self.library_scanning = true;
                let dirs = self.config.music_dirs.clone();
                let db_path = dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("cosmic-music-player")
                    .join("library.db");

                return cosmic::task::future(async move {
                    let count = if let Some(parent) = db_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                        if let Ok(db) = LibraryDb::open(&db_path) {
                            LibraryScanner::scan(&db, &dirs).unwrap_or(0)
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    cosmic::Action::App(Message::LibraryScanComplete(count))
                });
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
                self.all_tracks = tracks;
                self.all_albums = albums;
                self.all_artists = artists;
                self.cover_images = cover_images;
                self.artist_avatars = artist_avatars;
            }

            // -- Playback --
            Message::TogglePlayback => {
                if let Some(ref mut player) = self.player {
                    if player.state() == PlaybackState::Stopped && !self.all_tracks.is_empty() {
                        // If stopped, start playing first track
                        let paths: Vec<PathBuf> =
                            self.all_tracks.iter().map(|t| t.path.clone()).collect();
                        player.set_queue(paths);
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
                if let Some(ref mut player) = self.player
                    && player.next().is_ok() {
                        let idx = player.queue_index();
                        self.current_track = self.all_tracks.get(idx).cloned();
                        self.playback_position = Duration::ZERO;
                        self.lyrics_text = None;
                    }
            }

            Message::PreviousTrack => {
                if let Some(ref mut player) = self.player
                    && player.previous().is_ok() {
                        let idx = player.queue_index();
                        self.current_track = self.all_tracks.get(idx).cloned();
                        self.playback_position = Duration::ZERO;
                        self.lyrics_text = None;
                    }
            }

            Message::Seek(fraction) => {
                if let Some(ref mut player) = self.player
                    && let Some(ref track) = self.current_track {
                        let target =
                            Duration::from_secs_f32(fraction * track.duration.as_secs_f32());
                        player.seek(target).ok();
                        self.playback_position = target;
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
                // Advance position estimate
                self.playback_position += Duration::from_millis(500);

                // Check if track ended
                if let Some(ref mut player) = self.player
                    && player.is_finished().unwrap_or(false) {
                        // Auto-advance
                        if player.next().is_ok() {
                            let idx = player.queue_index();
                            self.current_track = self.all_tracks.get(idx).cloned();
                            self.playback_position = Duration::ZERO;
                            self.lyrics_text = None;
                        }
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
        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cosmic-music-player")
            .join("library.db");

        cosmic::task::future(async move {
            let (tracks, albums, artists) = if let Ok(db) = LibraryDb::open(&db_path) {
                let tracks = db.all_tracks().unwrap_or_default();
                let albums = db.all_albums().unwrap_or_default();
                let artists = db.all_artists().unwrap_or_default();
                (tracks, albums, artists)
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };

            // Extract cover art for each album
            let mut cover_images = HashMap::new();
            for album in &albums {
                if let Some(first_track) = album.tracks.first() {
                    let key = crate::library::CoverArt::album_key(&album.artist, &album.name);
                    if let Some(bytes) = crate::library::CoverArt::get_cover_art(&first_track.path)
                    {
                        let handle = widget::icon::from_raster_bytes(bytes);
                        cover_images.insert(key, handle);
                    }
                }
            }

            // Generate artist avatars
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

    fn play_track_list(&mut self, tracks: &[Track], start_index: usize) {
        let paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();

        if let Some(ref mut player) = self.player {
            player.set_queue(paths);
            if player.play_index(start_index).is_ok() {
                self.current_track = tracks.get(start_index).cloned();
                self.playback_position = Duration::ZERO;
                self.lyrics_text = None;
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
}

/// Menu bar actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Equalizer,
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
            MenuAction::ScanLibrary => Message::ScanLibrary,
            MenuAction::AddMusicDir => Message::AddMusicDir,
            MenuAction::Quit => Message::Quit,
        }
    }
}
