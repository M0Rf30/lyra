// SPDX-License-Identifier: GPL-3.0

use crate::config::{Config, ReplayGainMode};
use crate::convert::{ConvertJob, JobKind, JobState, OutputFormat, run_job};
use crate::fl;
use crate::library::{Album, Artist, LibraryDb, LibraryScanner, Lyrics, LyricsProvider, Track};
use crate::online::podcast::{self, PodcastSearchResult};
use crate::online::radio;
use crate::online::radio::StationSearchResult;
use crate::online::store::{Episode, OnlineStore, Podcast, RadioStation};
use crate::player::mpd_backend::MpdBackend;
use crate::player::{ActiveBackend, PlaybackState, Player};
use crate::provider::local::LocalProvider;
use crate::provider::mpd::{MpdConfig, MpdProvider};
use crate::provider::subsonic::{SubsonicConfig, SubsonicProvider};
use crate::provider::{MusicProvider, ProviderRegistry};
use crate::views::{
    albums, artists, convert, equalizer, genres, lyrics, now_playing, playlists, podcasts,
    providers, settings, songs,
};
use crate::views::radio as radio_view;
use cosmic::app::context_drawer;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::prelude::*;
use cosmic::widget::{self, about::About, icon, menu, nav_bar};
use futures_util::{SinkExt, Stream};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "visualizer")]
use std::sync::Mutex;
use std::time::Duration;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const APP_ICON: &[u8] =
    include_bytes!("../resources/icons/hicolor/scalable/apps/io.github.m0rf30.Lyra.svg");

/// Widget id for the header library-search input, used to programmatically
/// focus it when the search bar is activated.
const SEARCH_INPUT_ID: &str = "lyra-library-search";

/// Frames of no mouse movement (at the visualizer's ~30fps render cadence)
/// before the fullscreen HUD control card auto-hides. ~3 seconds.
#[cfg(feature = "visualizer")]
const VIZ_HUD_HOLD_FRAMES: u32 = 90;

/// Main application model.
pub struct AppModel {
    core: cosmic::Core,
    nav: nav_bar::Model,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    about: About,
    config: Config,
    /// Cached cosmic-config context to avoid repeated D-Bus watcher creation attempts.
    config_context: Option<cosmic_config::Config>,
    context_page: ContextPage,

    // Notifications
    /// Toast notifications (e.g. provider connection failures).
    toasts: widget::toaster::Toasts<Message>,

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
    /// Monotonically increasing generation counter for library
    /// reloads/scans. Bumped before every new reload/scan so in-flight
    /// async results tagged with an older generation (or a different
    /// provider id) can be detected and ignored as stale.
    reload_generation: u64,

    // Library search (header search bar)
    /// Current search query (case-insensitive substring match against the
    /// active page's fields).
    library_search: String,
    /// Whether the header search input is visible/active.
    search_active: bool,
    /// Albums matching `library_search`; cached in the model (not built
    /// fresh inside `view()`) because view functions borrow `&'a [T]`
    /// slices that must live as long as `&self` — a `Vec` built locally
    /// inside `view()` would not satisfy that lifetime.
    filtered_albums: Vec<Album>,
    /// Maps `filtered_albums[i]` back to its index in `all_albums`, so
    /// index-carrying view messages (select/play) resolve against the
    /// real, unfiltered data.
    filtered_album_map: Vec<usize>,
    filtered_artists: Vec<Artist>,
    filtered_artist_map: Vec<usize>,
    filtered_tracks: Vec<Track>,
    filtered_track_map: Vec<usize>,
    filtered_playlists: Vec<crate::library::Playlist>,
    filtered_playlist_map: Vec<usize>,
    filtered_genres: Vec<String>,
    filtered_genre_map: Vec<usize>,

    // Podcasts
    podcasts: Vec<Podcast>,
    selected_podcast: Option<usize>,
    podcast_episodes: Vec<Episode>,
    podcast_search_query: String,
    podcast_search_results: Vec<PodcastSearchResult>,
    podcast_search_loading: bool,
    podcast_add_url: String,
    /// The episode id currently playing, if the current track came from a
    /// podcast subscription — used to persist playback position.
    current_podcast_episode_id: Option<i64>,
    /// Last whole-second position persisted for `current_podcast_episode_id`,
    /// so the tick handler only writes to the DB roughly every 5 seconds.
    last_saved_podcast_position_secs: u64,

    // Radio
    radio_stations: Vec<RadioStation>,
    radio_search_query: String,
    radio_search_results: Vec<StationSearchResult>,
    radio_search_loading: bool,
    radio_add_name: String,
    radio_add_url: String,

    /// Podcast artwork / radio favicon bytes, keyed by their source URL.
    /// Shared between both views since the icons are the same kind of
    /// small, best-effort raster image loaded from a remote URL.
    online_icons: HashMap<String, widget::icon::Handle>,

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
    /// When true, the Songs list column headers show a descending arrow.
    songs_sort_descending: bool,
    /// When true, the Songs view shows only favorite tracks.
    favorites_filter: bool,
    /// When set, the Songs view shows only tracks matching this genre.
    genre_filter: Option<String>,
    /// Available playlists for the Playlists view.
    playlists: Vec<crate::library::Playlist>,
    /// Currently selected playlist index (for detail view).
    selected_playlist: Option<usize>,
    /// Text input for new playlist name.
    new_playlist_name: String,
    /// Live-edited text for the rename field in `playlist_detail_view`,
    /// seeded from the playlist's current name when its detail view opens.
    rename_playlist_input: String,
    /// All distinct genres from the active provider.
    all_genres: Vec<String>,
    /// Currently selected genre index (for detail view).
    selected_genre: Option<usize>,
    /// Tracks filtered by the currently selected genre.
    genre_tracks: Vec<Track>,
    cover_images: HashMap<String, widget::icon::Handle>,
    artist_avatars: HashMap<String, widget::icon::Handle>,

    // Keyboard input state
    /// Tracks whether any text input field currently has keyboard focus.
    /// When true, space bar should type a space character instead of toggling playback.
    text_input_focused: bool,

    // Lyrics
    lyrics_text: Option<Lyrics>,
    lyrics_loading: bool,
    /// When true and the expanded now-playing view is active, lyrics render
    /// as an in-view overlay (over the cover art / visualizer) instead of
    /// opening the generic context-drawer sidebar, keeping the immersive
    /// full view intact.
    lyrics_overlay_active: bool,

    // Equalizer
    eq_preset: Option<crate::player::equalizer::EqPreset>,
    preset_manager: crate::player::eq_presets::EqPresetManager,
    all_presets: Vec<crate::player::equalizer::EqPresetData>,
    active_preset_name: Option<String>,
    eq_dirty: bool,
    save_as_name: String,

    // AutoEQ
    /// AutoEQ profiles loaded from GitHub, available in the preset dropdown.
    autoeq_profiles: Vec<crate::autoeq::AutoEQProfileMetadata>,
    autoeq_loading: bool,
    /// Current search query for filtering AutoEQ profiles in the dropdown.
    autoeq_search: String,

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
    /// Whether the visualizer is currently fullscreen.
    #[cfg(feature = "visualizer")]
    viz_fullscreen: bool,
    /// Nav-bar active state saved when entering visualizer fullscreen, so it
    /// can be restored on exit (the user may have collapsed it beforehand).
    #[cfg(feature = "visualizer")]
    viz_prev_nav_active: bool,
    /// Shared frame buffer for the shader-based visualizer widget.
    /// The render subscription writes RGBA pixels here; the Shader widget
    /// reads them in its `prepare()` method via `queue.write_texture()`.
    #[cfg(feature = "visualizer")]
    viz_frame_buf: Arc<Mutex<crate::views::now_playing::viz_shader::VizFrameBuffer>>,
    #[cfg(feature = "visualizer")]
    pcm_buffer: Option<Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>>,
    /// Sender half of the command channel to the render thread (see
    /// `VizCommand`); the `Receiver` lives in `viz_cmd_rx_slot`.
    #[cfg(feature = "visualizer")]
    viz_cmd_tx: std::sync::mpsc::Sender<crate::views::now_playing::visualizer::VizCommand>,
    /// Holds the render thread's command `Receiver` between activations —
    /// see the comment where it's created in `AppModel::init`.
    #[cfg(feature = "visualizer")]
    viz_cmd_rx_slot: Arc<
        Mutex<Option<std::sync::mpsc::Receiver<crate::views::now_playing::visualizer::VizCommand>>>,
    >,
    /// Preset name the render thread most recently loaded/switched to.
    /// `None` after a playlist-driven `NextPreset` switch, since the
    /// playlist API exposes no way to read back which preset it landed
    /// on; set again on the next explicit `LoadPreset`.
    #[cfg(feature = "visualizer")]
    viz_current_preset_shared: Arc<Mutex<Option<String>>>,
    /// Opacity of the visualizer metadata overlay (0.0 = hidden, 1.0 = fully visible).
    /// Decays to 0 over ~4 seconds after a track change.
    #[cfg(feature = "visualizer")]
    viz_metadata_opacity: f32,
    /// Frames elapsed (~30fps) since the mouse last moved while the
    /// visualizer is fullscreen. Drives HUD control-card auto-hide; see
    /// `VIZ_HUD_HOLD_FRAMES`.
    #[cfg(feature = "visualizer")]
    viz_hud_idle_frames: u32,
    /// Whether the cursor is currently over the fullscreen HUD control
    /// card. While true, the card stays visible regardless of
    /// `viz_hud_idle_frames` (the user may be resting the pointer on a
    /// slider/button without moving it).
    #[cfg(feature = "visualizer")]
    viz_hud_pointer_over: bool,

    // Local file conversion / transcoding / CUE-ripping
    /// Queued/running/finished conversion jobs, oldest first.
    convert_jobs: Vec<ConvertJob>,
    /// Output directory for new conversion jobs.
    convert_out_dir: PathBuf,
    /// Selected index into `OutputFormat::ALL`.
    convert_format_index: usize,
    /// Selected index into `views::convert::SAMPLE_RATE_OPTIONS`.
    convert_rate_index: usize,
    /// Monotonic id source for new jobs.
    convert_next_id: u64,
    /// Caps concurrently-running conversion jobs at 2, shared across every
    /// in-flight job future.
    convert_semaphore: Arc<tokio::sync::Semaphore>,
    /// Whether the preset browser overlay is currently open.
    #[cfg(feature = "visualizer")]
    viz_browser_open: bool,
    /// Discovered `.milk` presets, populated lazily on first browser open.
    #[cfg(feature = "visualizer")]
    viz_preset_entries: Vec<crate::views::now_playing::visualizer::PresetEntry>,
    /// Set once the background preset scan has been kicked off, so
    /// reopening the browser doesn't rescan every time.
    #[cfg(feature = "visualizer")]
    viz_presets_scan_started: bool,
    /// Live search filter text for the preset browser.
    #[cfg(feature = "visualizer")]
    viz_preset_search: String,
    /// Whether automatic preset transitions are locked (see
    /// `VizCommand::SetLocked`).
    #[cfg(feature = "visualizer")]
    viz_locked: bool,
    /// Beat-reactivity sensitivity (see `VizCommand::SetBeatSensitivity`).
    #[cfg(feature = "visualizer")]
    viz_beat_sensitivity: f32,
    /// UI-local mirror of `viz_current_preset_shared`, updated
    /// optimistically on `LoadVizPreset` and resynced from the render
    /// thread on every `VisualizerFrameReady`.
    #[cfg(feature = "visualizer")]
    viz_current_preset_name: Option<String>,
}

/// All application messages.
#[derive(Debug, Clone)]
pub enum Message {
    // Navigation / chrome
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    /// Toggle albums view between grid and list.
    ToggleAlbumsViewMode,
    /// Toggle artists view between grid and list.
    ToggleArtistsViewMode,
    /// Toggle genres view between grid and list.
    ToggleGenresViewMode,
    /// Tracks text input focus state for keyboard shortcuts.
    /// When true, text input has focus; when false, input lost focus.
    TextInputFocused(bool),

    // Library search
    /// The library search query changed (live filter as you type).
    LibrarySearchChanged(String),
    /// Toggle the header search input on/off.
    ToggleLibrarySearch,
    /// Clear the search query and deactivate the search input (Esc / clear icon).
    ClearLibrarySearch,
    // Library
    ScanLibrary,
    /// `count` is the number of tracks the scan updated; `generation`
    /// and `provider_id` identify the reload/scan this result belongs
    /// to, so stale results (superseded by a later reload or a provider
    /// switch) can be ignored.
    LibraryScanComplete {
        generation: u64,
        provider_id: String,
        count: usize,
    },
    LibraryLoaded {
        generation: u64,
        provider_id: String,
        tracks: Vec<Track>,
        albums: Vec<Album>,
        artists: Vec<Artist>,
        cover_images: HashMap<String, widget::icon::Handle>,
        artist_avatars: HashMap<String, widget::icon::Handle>,
        /// Raw cover art bytes for blur processing.
        cover_art_bytes: HashMap<String, Vec<u8>>,
    },
    /// Filesystem watcher detected changes in music directories.
    /// Contains the deduplicated list of changed paths after debounce.
    FilesChanged(Vec<PathBuf>),

    /// Incremental batch of albums from a remote provider (e.g. Subsonic).
    /// Each batch appends albums, derives tracks/artists, and updates the UI.
    LibraryBatch {
        generation: u64,
        provider_id: String,
        albums: Vec<Album>,
        cover_images: HashMap<String, widget::icon::Handle>,
        /// Raw cover art bytes for blur processing.
        cover_art_bytes: HashMap<String, Vec<u8>>,
    },
    /// Signals that incremental loading is complete.
    LibraryLoadComplete {
        generation: u64,
        provider_id: String,
    },

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
    /// Toggle the favorites-only filter in the Songs view.
    ToggleFavoritesFilter,

    // Favorites & ratings
    /// Toggle favorite status for a track (by track ID string).
    ToggleFavorite(String),
    /// Set rating (1-5) for a track. Pass 0 to clear.
    SetRating(String, u8),

    // Playlist actions
    /// Add a track to a playlist. (track source_uri, playlist ID)
    AddToPlaylist(String, String),

    // Playlists view
    /// Select a playlist to show its detail view.
    SelectPlaylist(usize),
    /// Go back from playlist detail to the list.
    BackToPlaylistList,
    /// Create a new playlist with the given name.
    CreatePlaylist(String),
    /// Delete a playlist by index.
    DeletePlaylist(usize),
    /// Rename a playlist (index, new name).
    RenamePlaylist(usize, String),
    /// Play all tracks in a playlist.
    PlayPlaylist(usize),
    /// Play a specific track within a playlist (playlist idx, track idx).
    PlayPlaylistTrack(usize, usize),
    /// Remove a track from a playlist (playlist idx, track idx).
    RemovePlaylistTrack(usize, usize),
    /// The new-playlist name input changed.
    NewPlaylistNameChanged(String),
    /// The rename input changed (playlist index, new text).
    RenamePlaylistInput(usize, String),
    /// Playlists have been loaded from the provider.
    PlaylistsLoaded(Vec<crate::library::Playlist>),

    // Genre filtering
    /// Filter tracks by genre name.
    FilterByGenre(String),

    // Genres view
    /// Select a genre to show its tracks.
    SelectGenre(usize),
    /// Go back from genre detail to the grid.
    BackToGenreGrid,
    /// Play a track in the genre detail view (index within genre_tracks).
    PlayGenreTrack(usize),
    /// Genres have been loaded from the provider.
    GenresLoaded(Vec<String>),
    /// Genre tracks have been loaded.
    GenreTracksLoaded(Vec<Track>),

    // Lyrics
    ShowLyrics,
    LyricsLoaded(Option<Lyrics>),
    FetchLyricsOnline,

    // Equalizer
    EqSetBand(usize, f32),
    EqSetPreset(crate::player::equalizer::EqPreset),
    EqToggle(bool),
    EqSetPreamp(f32),
    /// Select a preset by name from the combined preset list.
    EqSelectPreset(String),
    /// Overwrite the currently loaded custom preset with current band/preamp values.
    EqSavePreset,
    /// Save current band/preamp values as a new named custom preset.
    EqSavePresetAs(String),
    /// Delete the currently active custom preset.
    EqDeletePreset,
    /// Reset to Flat (all bands 0, preamp 0, clear selection).
    EqResetPreset,
    /// Update the "Save As" name input.
    EqSaveAsNameChanged(String),

    // AutoEQ
    /// Update the AutoEQ search query (filters profiles in dropdown).
    AutoEQSearchChanged(String),
    /// Fetch AutoEQ index from GitHub (triggered by "Load AutoEQ..." button).
    FetchAutoEQIndex,
    /// AutoEQ index fetched successfully (or error).
    AutoEQIndexLoaded(Result<Vec<crate::autoeq::AutoEQProfileMetadata>, String>),
    /// Select an AutoEQ profile by path from the dropdown.
    EqSelectAutoEQ(String),
    /// AutoEQ profile fetched and ready to apply (or error).
    AutoEQProfileLoaded(Result<crate::autoeq::AutoEQProfile, String>),

    // Settings
    AddMusicDir,
    /// Result from the XDG Desktop Portal directory picker.
    DirPickerResult(Result<PathBuf, String>),
    RemoveMusicDir(usize),
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
    /// Subsonic transcoding bitrate changed (server index, bitrate or None).
    SubsonicTranscodingBitrate(usize, Option<u32>),
    /// Subsonic transcoding format changed (server index, format or None).
    SubsonicTranscodingFormat(usize, Option<String>),

    // Playback settings (Tasks 107, 108, 113, 114)
    /// Set crossfade duration (seconds, 0 = disabled).
    SetCrossfade(f32),
    /// Set replay gain mode.
    SetReplayGainMode(ReplayGainMode),

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
    /// Toggle fullscreen for the visualizer window (on → off, off → on).
    #[cfg(feature = "visualizer")]
    ToggleVisualizerFullscreen,
    /// Mouse moved while the visualizer is fullscreen — resets the HUD
    /// control-card auto-hide idle counter.
    #[cfg(feature = "visualizer")]
    VizHudActivity,
    /// Cursor entered/left the fullscreen HUD control card — see
    /// `NowPlayingMessage::VizHudPointerEnter`/`VizHudPointerExit`.
    #[cfg(feature = "visualizer")]
    VizHudPointerEnter,
    #[cfg(feature = "visualizer")]
    VizHudPointerExit,
    /// Toggle the preset browser overlay on/off.
    #[cfg(feature = "visualizer")]
    TogglePresetBrowser,
    /// Preset browser search query changed.
    #[cfg(feature = "visualizer")]
    PresetSearchInput(String),
    /// Load a specific preset file, bypassing the playlist, with a smooth
    /// transition.
    #[cfg(feature = "visualizer")]
    LoadVizPreset(PathBuf),
    /// Lock/unlock automatic preset transitions.
    #[cfg(feature = "visualizer")]
    SetVizLocked(bool),
    /// Adjust beat-reactivity sensitivity.
    #[cfg(feature = "visualizer")]
    SetVizBeatSensitivity(f32),
    /// Background preset scan finished.
    #[cfg(feature = "visualizer")]
    VizPresetsScanned(Vec<crate::views::now_playing::visualizer::PresetEntry>),

    // Notifications
    // Podcasts
    PodcastSearchChanged(String),
    PodcastSearchSubmit,
    PodcastSearchResults(Result<Vec<PodcastSearchResult>, String>),
    PodcastAddUrlChanged(String),
    SubscribePodcast(String),
    PodcastSubscribed(Result<(), String>),
    PodcastsLoaded(Vec<Podcast>),
    SelectPodcast(usize),
    BackToPodcastList,
    RemovePodcast(usize),
    RefreshPodcast(usize),
    RefreshAllPodcasts,
    PodcastRefreshed(i64, Result<(), String>),
    PodcastEpisodesLoaded(i64, Vec<Episode>),
    PlayPodcastEpisode(usize),
    TogglePodcastEpisodePlayed(usize),
    /// A podcast/radio icon (artwork or favicon) finished downloading.
    OnlineIconLoaded(String, Vec<u8>),

    // Radio
    RadioSearchChanged(String),
    RadioSearchSubmit,
    RadioSearchResults(Result<Vec<StationSearchResult>, String>),
    RadioAddNameChanged(String),
    RadioAddUrlChanged(String),
    AddRadioStation {
        name: String,
        stream_url: String,
        homepage: String,
        favicon_url: String,
        tags: String,
    },
    AddRadioFromSearch(usize),
    RadioStationsLoaded(Vec<RadioStation>),
    RemoveRadioStation(usize),
    PlayRadioStation(usize),
    PlayRadioSearchResult(usize),
    RadioStreamResolved {
        name: String,
        result: Result<String, String>,
    },

    /// A toast notification was dismissed (by timeout or user action).
    CloseToast(widget::toaster::ToastId),

    // Convert / transcode / rip local files
    ConvertAddFiles,
    ConvertFilesPicked(Result<Vec<PathBuf>, String>),
    ConvertPickOutputDir,
    ConvertOutputDirPicked(Result<PathBuf, String>),
    ConvertFormatSelected(usize),
    ConvertRateSelected(usize),
    ConvertStart,
    ConvertCancelJob(u64),
    ConvertClearFinished,
    ConvertJobFinished(u64, JobState),
    ConvertTick,

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
            albums::AlbumMessage::ToggleFavorite(id) => Message::ToggleFavorite(id),
            albums::AlbumMessage::SetRating(id, r) => Message::SetRating(id, r),
            albums::AlbumMessage::FilterByGenre(g) => Message::FilterByGenre(g),
            albums::AlbumMessage::AddToPlaylist(uri, pid) => Message::AddToPlaylist(uri, pid),
            albums::AlbumMessage::ToggleViewMode => Message::ToggleAlbumsViewMode,
        }
    }
}

impl From<artists::ArtistMessage> for Message {
    fn from(msg: artists::ArtistMessage) -> Self {
        match msg {
            artists::ArtistMessage::PlayArtistAlbum(ai, ali) => Message::PlayArtistAlbum(ai, ali),
            artists::ArtistMessage::PlayTrack(ai, ali, ti) => Message::PlayArtistTrack(ai, ali, ti),
            artists::ArtistMessage::SelectArtist(i) => Message::SelectArtist(i),
            artists::ArtistMessage::BackToList => Message::BackToArtistList,
            artists::ArtistMessage::ToggleFavorite(id) => Message::ToggleFavorite(id),
            artists::ArtistMessage::SetRating(id, r) => Message::SetRating(id, r),
            artists::ArtistMessage::FilterByGenre(g) => Message::FilterByGenre(g),
            artists::ArtistMessage::ToggleViewMode => Message::ToggleArtistsViewMode,
        }
    }
}

impl From<podcasts::PodcastMessage> for Message {
    fn from(msg: podcasts::PodcastMessage) -> Self {
        match msg {
            podcasts::PodcastMessage::SearchChanged(s) => Message::PodcastSearchChanged(s),
            podcasts::PodcastMessage::SearchSubmit => Message::PodcastSearchSubmit,
            podcasts::PodcastMessage::AddUrlChanged(s) => Message::PodcastAddUrlChanged(s),
            podcasts::PodcastMessage::AddByUrl(url) => Message::SubscribePodcast(url),
            podcasts::PodcastMessage::SubscribeFromSearch(url) => Message::SubscribePodcast(url),
            podcasts::PodcastMessage::SelectPodcast(i) => Message::SelectPodcast(i),
            podcasts::PodcastMessage::BackToList => Message::BackToPodcastList,
            podcasts::PodcastMessage::RemovePodcast(i) => Message::RemovePodcast(i),
            podcasts::PodcastMessage::RefreshPodcast(i) => Message::RefreshPodcast(i),
            podcasts::PodcastMessage::RefreshAll => Message::RefreshAllPodcasts,
            podcasts::PodcastMessage::PlayEpisode(i) => Message::PlayPodcastEpisode(i),
            podcasts::PodcastMessage::TogglePlayed(i) => Message::TogglePodcastEpisodePlayed(i),
        }
    }
}

impl From<radio_view::RadioMessage> for Message {
    fn from(msg: radio_view::RadioMessage) -> Self {
        match msg {
            radio_view::RadioMessage::SearchChanged(s) => Message::RadioSearchChanged(s),
            radio_view::RadioMessage::SearchSubmit => Message::RadioSearchSubmit,
            radio_view::RadioMessage::AddNameChanged(s) => Message::RadioAddNameChanged(s),
            radio_view::RadioMessage::AddUrlChanged(s) => Message::RadioAddUrlChanged(s),
            radio_view::RadioMessage::AddByUrl(name, url) => Message::AddRadioStation {
                name,
                stream_url: url,
                homepage: String::new(),
                favicon_url: String::new(),
                tags: String::new(),
            },
            radio_view::RadioMessage::AddFromSearch(i) => Message::AddRadioFromSearch(i),
            radio_view::RadioMessage::RemoveStation(i) => Message::RemoveRadioStation(i),
            radio_view::RadioMessage::PlayStation(i) => Message::PlayRadioStation(i),
            radio_view::RadioMessage::PlaySearchResult(i) => Message::PlayRadioSearchResult(i),
        }
    }
}

impl From<convert::ConvertMessage> for Message {
    fn from(msg: convert::ConvertMessage) -> Self {
        match msg {
            convert::ConvertMessage::AddFiles => Message::ConvertAddFiles,
            convert::ConvertMessage::PickOutputDir => Message::ConvertPickOutputDir,
            convert::ConvertMessage::FormatSelected(i) => Message::ConvertFormatSelected(i),
            convert::ConvertMessage::RateSelected(i) => Message::ConvertRateSelected(i),
            convert::ConvertMessage::StartQueue => Message::ConvertStart,
            convert::ConvertMessage::CancelJob(id) => Message::ConvertCancelJob(id),
            convert::ConvertMessage::ClearFinished => Message::ConvertClearFinished,
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
            .icon(icon::from_name("media-optical-symbolic"))
            .activate();

        nav.insert()
            .text(fl!("artists"))
            .data::<Page>(Page::Artists)
            .icon(icon::from_name("system-users-symbolic"));

        nav.insert()
            .text(fl!("songs"))
            .data::<Page>(Page::Songs)
            .icon(icon::from_name("audio-x-generic-symbolic"));

        nav.insert()
            .text(fl!("playlists"))
            .data::<Page>(Page::Playlists)
            .icon(icon::from_name("playlist-symbolic"));

        nav.insert()
            .text(fl!("genres"))
            .data::<Page>(Page::Genres)
            .icon(icon::from_name("folder-music-symbolic"));

        nav.insert()
            .text(fl!("podcasts"))
            .data::<Page>(Page::Podcasts)
            .icon(icon::from_name("application-rss+xml-symbolic"));

        nav.insert()
            .text(fl!("radio"))
            .data::<Page>(Page::Radio)
            .icon(icon::from_name("network-wireless-symbolic"));

        nav.insert()
            .text(fl!("convert"))
            .data::<Page>(Page::Convert)
            .icon(icon::from_name("media-import-audio-symbolic"));

        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license("GPL-3.0");

        // Load config and cache the context to avoid repeated D-Bus watcher creation
        let config_context = cosmic_config::Config::new(Self::APP_ID, Config::VERSION).ok();
        let mut config = config_context
            .as_ref()
            .map(|context| match Config::get_entry(context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        // Tasks 83-84: Migrate plaintext passwords to system keyring.
        // For each provider config entry that has a password but hasn't been
        // migrated yet, attempt to store it in the keyring. On failure, keep
        // the plaintext password and log a warning.
        {
            let keyring_ok = crate::credentials::is_keyring_available();
            let mut config_changed = false;

            if keyring_ok {
                for entry in &mut config.mpd_servers {
                    if entry.password_in_keyring {
                        // Verify the keyring entry still exists; reset if lost.
                        match crate::credentials::retrieve_password(&entry.id) {
                            Ok(None) => {
                                tracing::warn!(
                                    "MPD password for '{}' was marked as stored in keyring \
                                     but the entry is missing; resetting so user can re-enter.",
                                    entry.id
                                );
                                entry.password_in_keyring = false;
                                config_changed = true;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to verify keyring entry for MPD '{}': {e}",
                                    entry.id
                                );
                            }
                            Ok(Some(_)) => {}
                        }
                    } else if entry.password.is_some() {
                        let pw = entry.password.as_deref().unwrap_or_default();
                        match crate::credentials::store_password(&entry.id, pw) {
                            Ok(()) => {
                                tracing::info!(
                                    "Migrated MPD password for '{}' to system keyring",
                                    entry.id
                                );
                                entry.password_in_keyring = true;
                                entry.password = None;
                                config_changed = true;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to migrate MPD password for '{}' to keyring, \
                                     keeping plaintext in config: {e}",
                                    entry.id
                                );
                            }
                        }
                    }
                }

                for entry in &mut config.subsonic_servers {
                    if entry.password_in_keyring {
                        // Verify the keyring entry still exists; reset if lost.
                        match crate::credentials::retrieve_password(&entry.id) {
                            Ok(None) => {
                                tracing::warn!(
                                    "Subsonic password for '{}' was marked as stored in keyring \
                                     but the entry is missing; resetting so user can re-enter.",
                                    entry.id
                                );
                                entry.password_in_keyring = false;
                                config_changed = true;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to verify keyring entry for Subsonic '{}': {e}",
                                    entry.id
                                );
                            }
                            Ok(Some(_)) => {}
                        }
                    } else if entry.password.is_some() {
                        let pw = entry.password.as_deref().unwrap_or_default();
                        match crate::credentials::store_password(&entry.id, pw) {
                            Ok(()) => {
                                tracing::info!(
                                    "Migrated Subsonic password for '{}' to system keyring",
                                    entry.id
                                );
                                entry.password_in_keyring = true;
                                entry.password = None;
                                config_changed = true;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to migrate Subsonic password for '{}' to keyring, \
                                     keeping plaintext in config: {e}",
                                    entry.id
                                );
                            }
                        }
                    }
                }
            } else {
                tracing::warn!(
                    "System keyring is not available; passwords will remain in plaintext config"
                );
            }

            // Persist config if any passwords were migrated.
            if config_changed
                && let Some(ref context) = config_context
                && let Err(e) = config.write_entry(context)
            {
                tracing::error!("Failed to save config after keyring migration: {e:?}");
            }
        }

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
        let mut subsonic_init_errors: Vec<(String, String)> = Vec::new();
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
                    subsonic_init_errors.push((entry.name.clone(), e.to_string()));
                }
            }
        }

        // Build editing state for MPD servers
        let mpd_edit_states: Vec<providers::MpdEditState> = config
            .mpd_servers
            .iter()
            .map(providers::MpdEditState::from_config)
            .collect();
        let mpd_connection_status: Vec<Option<String>> = vec![None; mpd_edit_states.len()];

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

        // Command channel to the projectM render thread — replaces the
        // old `next_preset_signal: AtomicBool` flag so the UI can also
        // request specific-preset loads, lock state, and beat sensitivity.
        // The `Sender` lives in `AppModel` for the whole app lifetime; the
        // `Receiver` is checked out of this `Mutex<Option<_>>` slot by
        // whichever render thread is currently running and handed back
        // when it stops (visualizer deactivated), so a later reactivation
        // can check it out again — the channel itself is only ever
        // created once, here.
        #[cfg(feature = "visualizer")]
        let (viz_cmd_tx, viz_cmd_rx) =
            std::sync::mpsc::channel::<crate::views::now_playing::visualizer::VizCommand>();
        #[cfg(feature = "visualizer")]
        let viz_cmd_rx_slot = Arc::new(Mutex::new(Some(viz_cmd_rx)));
        // Preset name the render thread most recently loaded/switched to;
        // written there, mirrored into `AppModel::viz_current_preset_name`
        // on every `VisualizerFrameReady`.
        #[cfg(feature = "visualizer")]
        let viz_current_preset_shared: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let mut app = AppModel {
            core,
            nav,
            key_binds: key_binds(),
            about,
            config,
            config_context: config_context.clone(),
            context_page: ContextPage::default(),
            toasts: widget::toaster::Toasts::new(Message::CloseToast),
            registry,
            mpd_providers,
            provider_list: Vec::new(),
            active_provider_index: None,
            all_tracks: Vec::new(),
            all_albums: Vec::new(),
            all_artists: Vec::new(),
            library_scanning: false,
            reload_generation: 0,

            library_search: String::new(),
            search_active: false,
            filtered_albums: Vec::new(),
            filtered_album_map: Vec::new(),
            filtered_artists: Vec::new(),
            filtered_artist_map: Vec::new(),
            filtered_tracks: Vec::new(),
            filtered_track_map: Vec::new(),
            filtered_playlists: Vec::new(),
            filtered_playlist_map: Vec::new(),
            filtered_genres: Vec::new(),
            filtered_genre_map: Vec::new(),
            podcasts: Vec::new(),
            selected_podcast: None,
            podcast_episodes: Vec::new(),
            podcast_search_query: String::new(),
            podcast_search_results: Vec::new(),
            podcast_search_loading: false,
            podcast_add_url: String::new(),
            current_podcast_episode_id: None,
            last_saved_podcast_position_secs: 0,
            radio_stations: Vec::new(),
            radio_search_query: String::new(),
            radio_search_results: Vec::new(),
            radio_search_loading: false,
            radio_add_name: String::new(),
            radio_add_url: String::new(),
            online_icons: HashMap::new(),
            player,
            playback_position: Duration::ZERO,
            current_track: None,
            seeking_preview: None,
            scrobble_now_playing_sent: false,
            scrobble_sent: false,
            selected_album: None,
            selected_artist: None,
            songs_sort: songs::SortField::Title,
            songs_sort_descending: false,
            favorites_filter: false,
            genre_filter: None,
            playlists: Vec::new(),
            selected_playlist: None,
            new_playlist_name: String::new(),
            rename_playlist_input: String::new(),
            all_genres: Vec::new(),
            selected_genre: None,
            genre_tracks: Vec::new(),
            cover_images: HashMap::new(),
            artist_avatars: HashMap::new(),
            text_input_focused: false,
            lyrics_text: None,
            lyrics_loading: false,
            lyrics_overlay_active: false,
            eq_preset: None,
            preset_manager: {
                let presets_dir = dirs::config_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("lyra")
                    .join("eq_presets");
                crate::player::eq_presets::EqPresetManager::new(presets_dir)
                    .expect("Failed to create EQ presets directory")
            },
            all_presets: Vec::new(), // loaded below after construction
            active_preset_name: None,
            eq_dirty: false,
            save_as_name: String::new(),
            autoeq_profiles: Vec::new(),
            autoeq_loading: false,
            autoeq_search: String::new(),
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
            viz_fullscreen: false,
            #[cfg(feature = "visualizer")]
            viz_prev_nav_active: true,
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
            viz_cmd_tx,
            #[cfg(feature = "visualizer")]
            viz_cmd_rx_slot,
            #[cfg(feature = "visualizer")]
            viz_current_preset_shared,
            #[cfg(feature = "visualizer")]
            viz_metadata_opacity: 0.0,
            #[cfg(feature = "visualizer")]
            viz_hud_idle_frames: 0,
            #[cfg(feature = "visualizer")]
            viz_hud_pointer_over: false,
            convert_jobs: Vec::new(),
            convert_out_dir: dirs::audio_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
                .join("Converted"),
            convert_format_index: 0,
            convert_rate_index: 0,
            convert_next_id: 0,
            convert_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
            #[cfg(feature = "visualizer")]
            viz_browser_open: false,
            #[cfg(feature = "visualizer")]
            viz_preset_entries: Vec::new(),
            #[cfg(feature = "visualizer")]
            viz_presets_scan_started: false,
            #[cfg(feature = "visualizer")]
            viz_preset_search: String::new(),
            #[cfg(feature = "visualizer")]
            viz_locked: false,
            #[cfg(feature = "visualizer")]
            viz_beat_sensitivity: 1.0,
            #[cfg(feature = "visualizer")]
            viz_current_preset_name: None,
        };

        app.rebuild_provider_list();
        app.all_presets = app.preset_manager.load_all();
        // Restore active preset name from config
        if !app.config.active_eq_preset_name.is_empty() {
            app.active_preset_name = Some(app.config.active_eq_preset_name.clone());
        }
        let title_cmd = app.update_title();

        // Trigger initial library scan
        let scan_cmd = cosmic::task::message(cosmic::Action::App(Message::ScanLibrary));

        // Surface any provider construction failures collected above as toasts.
        let mut init_tasks = vec![title_cmd, scan_cmd];
        for (name, reason) in subsonic_init_errors {
            init_tasks.push(app.push_toast(widget::toaster::Toast::new(fl!(
                "toast-provider-connect-failed",
                provider = name,
                reason = reason
            ))));
        }

        (app, Task::batch(init_tasks))
    }

    /// Header bar start: menu bar.
    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![
            menu::Tree::with_children(
                menu::root(fl!("file")).apply(Element::from),
                menu::items(
                    &self.key_binds,
                    vec![
                        // `menu::Item::Divider` renders as a solid filled
                        // block (not a thin line) in this pinned libcosmic
                        // revision -- omitted rather than shipping a
                        // visibly-broken separator.
                        menu::Item::Button(fl!("add-music-folder"), None, MenuAction::AddMusicDir),
                        menu::Item::Button(fl!("scan-library"), None, MenuAction::ScanLibrary),
                        menu::Item::Button(fl!("quit"), None, MenuAction::Quit),
                    ],
                ),
            ),
            menu::Tree::with_children(
                menu::root(fl!("view")).apply(Element::from),
                menu::items(
                    &self.key_binds,
                    vec![
                        menu::Item::Button(fl!("search"), None, MenuAction::Search),
                        menu::Item::Button(fl!("equalizer"), None, MenuAction::Equalizer),
                        menu::Item::Button(fl!("providers"), None, MenuAction::Providers),
                        menu::Item::Button(fl!("settings"), None, MenuAction::Settings),
                        menu::Item::Button(fl!("about"), None, MenuAction::About),
                    ],
                ),
            ),
        ]);

        vec![menu_bar.into()]
    }

    /// Header bar center: library search input, shown when search is
    /// active (playback controls are in the bottom bar).
    fn header_center(&self) -> Vec<Element<'_, Self::Message>> {
        if !self.search_active {
            return vec![];
        }

        let input = widget::search_input(fl!("search-library"), &self.library_search)
            .id(widget::Id::new(SEARCH_INPUT_ID))
            .on_input(Message::LibrarySearchChanged)
            .on_clear(Message::ClearLibrarySearch)
            .width(Length::Fixed(320.0));

        vec![input.into()]
    }

    /// Header bar end: library search toggle, plus the provider selector
    /// (shown when multiple providers are configured).
    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        let mut elements: Vec<Element<'_, Self::Message>> = vec![
            widget::button::icon(icon::from_name("edit-find-symbolic"))
                .selected(self.search_active)
                .tooltip(fl!("search"))
                .on_press(Message::ToggleLibrarySearch)
                .into(),
        ];

        if self.provider_list.len() > 1 {
            let provider_names: Vec<String> = self
                .provider_list
                .iter()
                .map(|(_, name)| name.clone())
                .collect();

            let dropdown = widget::dropdown(
                provider_names,
                self.active_provider_index,
                Message::SwitchProvider,
            );

            elements.push(dropdown.into());
        }

        elements
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
                let save_as = self.save_as_name.clone();

                let eq_content = equalizer::equalizer_view(
                    &self.config.equalizer_bands,
                    self.config.equalizer_enabled,
                    self.config.equalizer_preamp,
                    &self.all_presets,
                    self.active_preset_name.as_deref(),
                    self.eq_dirty,
                    &self.save_as_name,
                    &self.autoeq_profiles,
                    self.autoeq_loading,
                    &self.autoeq_search,
                )
                .map(move |msg| match msg {
                    equalizer::EqualizerMessage::SetBand(i, v) => Message::EqSetBand(i, v),
                    equalizer::EqualizerMessage::ToggleEnabled(e) => Message::EqToggle(e),
                    equalizer::EqualizerMessage::SetPreamp(v) => Message::EqSetPreamp(v),
                    equalizer::EqualizerMessage::SelectPreset(name) => {
                        Message::EqSelectPreset(name)
                    }
                    equalizer::EqualizerMessage::SelectAutoEQ(path) => {
                        Message::EqSelectAutoEQ(path)
                    }
                    equalizer::EqualizerMessage::SavePreset => Message::EqSavePreset,
                    equalizer::EqualizerMessage::SaveAsNameChanged(name) => {
                        Message::EqSaveAsNameChanged(name)
                    }
                    equalizer::EqualizerMessage::SavePresetAs => {
                        Message::EqSavePresetAs(save_as.clone())
                    }
                    equalizer::EqualizerMessage::DeletePreset => Message::EqDeletePreset,
                    equalizer::EqualizerMessage::ResetPreset => Message::EqResetPreset,
                    equalizer::EqualizerMessage::FetchAutoEQ => Message::FetchAutoEQIndex,
                    equalizer::EqualizerMessage::AutoEQSearchChanged(query) => {
                        Message::AutoEQSearchChanged(query)
                    }
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
                    providers::ProvidersMessage::TestConnection(i) => Message::MpdTestConnection(i),
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
                    providers::ProvidersMessage::SubsonicSave(i) => Message::SubsonicSaveServer(i),
                    providers::ProvidersMessage::SubsonicRemove(i) => {
                        Message::SubsonicRemoveServer(i)
                    }
                    providers::ProvidersMessage::SubsonicTestConnection(i) => {
                        Message::SubsonicTestConnection(i)
                    }
                    // Transcoding (Task 109)
                    providers::ProvidersMessage::SubsonicTranscodingBitrate(i, br) => {
                        Message::SubsonicTranscodingBitrate(i, br)
                    }
                    providers::ProvidersMessage::SubsonicTranscodingFormat(i, f) => {
                        Message::SubsonicTranscodingFormat(i, f)
                    }
                });

                context_drawer::context_drawer(
                    providers_content,
                    Message::ToggleContextPage(ContextPage::Providers),
                )
                .title(fl!("providers"))
            }
            ContextPage::Settings => {
                let volume = self
                    .player
                    .as_ref()
                    .map(|p| p.volume())
                    .unwrap_or(self.config.volume);

                let settings_content = settings::view(
                    &self.config.music_dirs,
                    self.config.crossfade_duration_secs,
                    self.config.replay_gain_mode,
                    volume,
                )
                .map(|msg| match msg {
                    settings::SettingsMessage::AddMusicDir => Message::AddMusicDir,
                    settings::SettingsMessage::RemoveMusicDir(i) => Message::RemoveMusicDir(i),
                    settings::SettingsMessage::SetCrossfade(v) => Message::SetCrossfade(v),
                    settings::SettingsMessage::SetReplayGainMode(m) => {
                        Message::SetReplayGainMode(m)
                    }
                    settings::SettingsMessage::SetVolume(v) => Message::SetVolume(v),
                    settings::SettingsMessage::OpenEqualizer => {
                        Message::ToggleContextPage(ContextPage::Equalizer)
                    }
                    settings::SettingsMessage::OpenProviders => {
                        Message::ToggleContextPage(ContextPage::Providers)
                    }
                    settings::SettingsMessage::OpenAbout => {
                        Message::ToggleContextPage(ContextPage::About)
                    }
                });

                context_drawer::context_drawer(
                    settings_content,
                    Message::ToggleContextPage(ContextPage::Settings),
                )
                .title(fl!("settings"))
            }
            ContextPage::Lyrics => {
                let (title, artist) = self
                    .current_track
                    .as_ref()
                    .map(|t| (t.title.as_str(), t.artist.as_str()))
                    .unwrap_or(("", ""));

                let lyrics_content = lyrics::lyrics_view(
                    self.lyrics_text.as_ref(),
                    title,
                    artist,
                    self.lyrics_loading,
                    self.playback_position,
                )
                .map(|msg| match msg {
                    lyrics::LyricsMessage::FetchLyrics => Message::FetchLyricsOnline,
                    lyrics::LyricsMessage::Close => Message::ToggleContextPage(ContextPage::Lyrics),
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
        let page = self
            .nav
            .active_data::<Page>()
            .cloned()
            .unwrap_or(Page::Albums);

        let search_query_active = self.search_active && !self.library_search.trim().is_empty();

        let content: Element<'_, Self::Message> = match page {
            Page::Albums => {
                if let Some(album_idx) = self.selected_album {
                    if let Some(album) = self.all_albums.get(album_idx) {
                        albums::album_detail_view(
                            album,
                            album_idx,
                            &self.cover_images,
                            &self.playlists,
                            self.current_track.as_ref().map(|t| t.id),
                        )
                        .map(Message::from)
                    } else {
                        widget::text("Album not found").into()
                    }
                } else {
                    let (albums_data, album_map): (&[Album], Option<&[usize]>) =
                        if search_query_active {
                            (
                                &self.filtered_albums,
                                Some(self.filtered_album_map.as_slice()),
                            )
                        } else {
                            (&self.all_albums, None)
                        };
                    albums::albums_view(
                        albums_data,
                        &self.cover_images,
                        self.config.albums_view_mode,
                    )
                    .map(move |msg| {
                        Message::from(match msg {
                            albums::AlbumMessage::SelectAlbum(i) => {
                                albums::AlbumMessage::SelectAlbum(unfilter_index(album_map, i))
                            }
                            albums::AlbumMessage::PlayAlbum(i) => {
                                albums::AlbumMessage::PlayAlbum(unfilter_index(album_map, i))
                            }
                            other => other,
                        })
                    })
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
                            self.current_track.as_ref().map(|t| t.id),
                        )
                        .map(Message::from)
                    } else {
                        widget::text("Artist not found").into()
                    }
                } else {
                    let (artists_data, artist_map): (&[Artist], Option<&[usize]>) =
                        if search_query_active {
                            (
                                &self.filtered_artists,
                                Some(self.filtered_artist_map.as_slice()),
                            )
                        } else {
                            (&self.all_artists, None)
                        };
                    artists::artists_view(
                        artists_data,
                        &self.artist_avatars,
                        self.config.artists_view_mode,
                    )
                    .map(move |msg| {
                        Message::from(match msg {
                            artists::ArtistMessage::SelectArtist(i) => {
                                artists::ArtistMessage::SelectArtist(unfilter_index(artist_map, i))
                            }
                            artists::ArtistMessage::PlayArtistAlbum(ai, ali) => {
                                artists::ArtistMessage::PlayArtistAlbum(
                                    unfilter_index(artist_map, ai),
                                    ali,
                                )
                            }
                            artists::ArtistMessage::PlayTrack(ai, ali, ti) => {
                                artists::ArtistMessage::PlayTrack(
                                    unfilter_index(artist_map, ai),
                                    ali,
                                    ti,
                                )
                            }
                            other => other,
                        })
                    })
                }
            }

            Page::Songs => {
                let (tracks_data, track_map): (&[Track], Option<&[usize]>) = if search_query_active
                {
                    (
                        &self.filtered_tracks,
                        Some(self.filtered_track_map.as_slice()),
                    )
                } else {
                    (&self.all_tracks, None)
                };
                songs::songs_list_view(
                    tracks_data,
                    self.songs_sort,
                    self.songs_sort_descending,
                    self.favorites_filter,
                    self.genre_filter.as_deref(),
                    &self.playlists,
                    self.current_track.as_ref().map(|t| t.id),
                )
                .map(move |msg| match msg {
                    songs::SongMessage::PlayTrack(i) => {
                        Message::PlayTrackIndex(unfilter_index(track_map, i))
                    }
                    songs::SongMessage::SortBy(f) => Message::SortSongs(f),
                    songs::SongMessage::ToggleFavorite(id) => Message::ToggleFavorite(id),
                    songs::SongMessage::SetRating(id, r) => Message::SetRating(id, r),
                    songs::SongMessage::AddToPlaylist(uri, pid) => Message::AddToPlaylist(uri, pid),
                    songs::SongMessage::ToggleFavoritesFilter => Message::ToggleFavoritesFilter,
                    songs::SongMessage::FilterByGenre(g) => Message::FilterByGenre(g),
                    songs::SongMessage::ClearGenreFilter => Message::FilterByGenre(String::new()),
                })
            }

            Page::Playlists => {
                if let Some(pl_idx) = self.selected_playlist {
                    if let Some(playlist) = self.playlists.get(pl_idx) {
                        playlists::playlist_detail_view(
                            playlist,
                            pl_idx,
                            &self.rename_playlist_input,
                        )
                        .map(|msg| match msg {
                            playlists::PlaylistMessage::BackToList => Message::BackToPlaylistList,
                            playlists::PlaylistMessage::PlayPlaylist(i) => Message::PlayPlaylist(i),
                            playlists::PlaylistMessage::PlayTrack(pi, ti) => {
                                Message::PlayPlaylistTrack(pi, ti)
                            }
                            playlists::PlaylistMessage::RemoveTrack(pi, ti) => {
                                Message::RemovePlaylistTrack(pi, ti)
                            }
                            playlists::PlaylistMessage::SelectPlaylist(i) => {
                                Message::SelectPlaylist(i)
                            }
                            playlists::PlaylistMessage::CreatePlaylist(n) => {
                                Message::CreatePlaylist(n)
                            }
                            playlists::PlaylistMessage::DeletePlaylist(i) => {
                                Message::DeletePlaylist(i)
                            }
                            playlists::PlaylistMessage::RenamePlaylist(i, n) => {
                                Message::RenamePlaylist(i, n)
                            }
                            playlists::PlaylistMessage::NewPlaylistNameChanged(n) => {
                                Message::NewPlaylistNameChanged(n)
                            }
                            playlists::PlaylistMessage::RenameInputChanged(i, n) => {
                                Message::RenamePlaylistInput(i, n)
                            }
                        })
                    } else {
                        widget::text("Playlist not found").into()
                    }
                } else {
                    let (playlists_data, playlist_map): (
                        &[crate::library::Playlist],
                        Option<&[usize]>,
                    ) = if search_query_active {
                        (
                            &self.filtered_playlists,
                            Some(self.filtered_playlist_map.as_slice()),
                        )
                    } else {
                        (&self.playlists, None)
                    };
                    playlists::playlist_list_view(playlists_data, &self.new_playlist_name).map(
                        move |msg| match msg {
                            playlists::PlaylistMessage::SelectPlaylist(i) => {
                                Message::SelectPlaylist(unfilter_index(playlist_map, i))
                            }
                            playlists::PlaylistMessage::CreatePlaylist(n) => {
                                Message::CreatePlaylist(n)
                            }
                            playlists::PlaylistMessage::DeletePlaylist(i) => {
                                Message::DeletePlaylist(unfilter_index(playlist_map, i))
                            }
                            playlists::PlaylistMessage::RenamePlaylist(i, n) => {
                                Message::RenamePlaylist(unfilter_index(playlist_map, i), n)
                            }
                            playlists::PlaylistMessage::NewPlaylistNameChanged(n) => {
                                Message::NewPlaylistNameChanged(n)
                            }
                            playlists::PlaylistMessage::RenameInputChanged(i, n) => {
                                Message::RenamePlaylistInput(unfilter_index(playlist_map, i), n)
                            }
                            playlists::PlaylistMessage::BackToList => Message::BackToPlaylistList,
                            playlists::PlaylistMessage::PlayPlaylist(i) => {
                                Message::PlayPlaylist(unfilter_index(playlist_map, i))
                            }
                            playlists::PlaylistMessage::PlayTrack(pi, ti) => {
                                Message::PlayPlaylistTrack(unfilter_index(playlist_map, pi), ti)
                            }
                            playlists::PlaylistMessage::RemoveTrack(pi, ti) => {
                                Message::RemovePlaylistTrack(unfilter_index(playlist_map, pi), ti)
                            }
                        },
                    )
                }
            }

            Page::Genres => {
                if let Some(genre_idx) = self.selected_genre {
                    if let Some(genre_name) = self.all_genres.get(genre_idx) {
                        genres::genre_detail_view(genre_name, &self.genre_tracks).map(|msg| {
                            match msg {
                                genres::GenreMessage::BackToGrid => Message::BackToGenreGrid,
                                genres::GenreMessage::PlayTrack(i) => Message::PlayGenreTrack(i),
                                genres::GenreMessage::SelectGenre(i) => Message::SelectGenre(i),
                                genres::GenreMessage::ToggleViewMode => {
                                    Message::ToggleGenresViewMode
                                }
                            }
                        })
                    } else {
                        widget::text("Genre not found").into()
                    }
                } else {
                    let (genres_data, genre_map): (&[String], Option<&[usize]>) =
                        if search_query_active {
                            (
                                &self.filtered_genres,
                                Some(self.filtered_genre_map.as_slice()),
                            )
                        } else {
                            (&self.all_genres, None)
                        };
                    genres::genres_view(genres_data, self.config.genres_view_mode).map(
                        move |msg| match msg {
                        genres::GenreMessage::SelectGenre(i) => {
                            Message::SelectGenre(unfilter_index(genre_map, i))
                        }
                        genres::GenreMessage::BackToGrid => Message::BackToGenreGrid,
                        genres::GenreMessage::PlayTrack(i) => Message::PlayGenreTrack(i),
                        genres::GenreMessage::ToggleViewMode => Message::ToggleGenresViewMode,
                        },
                    )
                }
            }

            Page::Podcasts => match self
                .selected_podcast
                .and_then(|idx| self.podcasts.get(idx).map(|podcast| (idx, podcast)))
            {
                Some((_, podcast)) => podcasts::podcast_detail_view(
                    podcast,
                    &self.podcast_episodes,
                    self.current_podcast_episode_id,
                    &self.online_icons,
                )
                .map(Message::from),
                None => podcasts::podcast_list_view(
                    &self.podcasts,
                    &self.podcast_search_query,
                    &self.podcast_search_results,
                    self.podcast_search_loading,
                    &self.podcast_add_url,
                    &self.online_icons,
                )
                .map(Message::from),
            },

            Page::Radio => {
                let current_radio_url = self
                    .current_track
                    .as_ref()
                    .filter(|t| &*t.provider_id == "radio")
                    .map(|t| t.source_uri.as_str());
                radio_view::radio_view(
                    &self.radio_stations,
                    &self.radio_search_query,
                    &self.radio_search_results,
                    self.radio_search_loading,
                    &self.radio_add_name,
                    &self.radio_add_url,
                    &self.online_icons,
                    current_radio_url,
                )
                .map(Message::from)
            }

            Page::Convert => convert::convert_view(
                &self.convert_jobs,
                &self.convert_out_dir,
                self.convert_format_index,
                self.convert_rate_index,
            )
            .map(Message::from),
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
            now_playing::NowPlayingMessage::ToggleFavorite(id) => Message::ToggleFavorite(id),
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::ToggleVisualizer => Message::ToggleVisualizer,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::NextPreset => Message::NextVisualizerPreset,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::ToggleVizFullscreen => {
                Message::ToggleVisualizerFullscreen
            }
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::VizHudPointerEnter => Message::VizHudPointerEnter,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::VizHudPointerExit => Message::VizHudPointerExit,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::TogglePresetBrowser => Message::TogglePresetBrowser,
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::PresetSearchInput(query) => {
                Message::PresetSearchInput(query)
            }
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::LoadVizPreset(path) => Message::LoadVizPreset(path),
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::SetVizLocked(locked) => Message::SetVizLocked(locked),
            #[cfg(feature = "visualizer")]
            now_playing::NowPlayingMessage::SetVizBeatSensitivity(v) => {
                Message::SetVizBeatSensitivity(v)
            }
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
            #[cfg(feature = "visualizer")]
            let viz_hud_visible = self.viz_hud_pointer_over
                || self.viz_hud_idle_frames < VIZ_HUD_HOLD_FRAMES
                || self.viz_browser_open;
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
                self.lyrics_overlay_active,
                self.lyrics_text.as_ref(),
                self.lyrics_loading,
                #[cfg(feature = "visualizer")]
                self.visualizer_active,
                #[cfg(feature = "visualizer")]
                Arc::clone(&self.viz_frame_buf),
                #[cfg(feature = "visualizer")]
                self.viz_metadata_opacity,
                #[cfg(feature = "visualizer")]
                self.viz_fullscreen,
                #[cfg(feature = "visualizer")]
                viz_hud_visible,
                #[cfg(feature = "visualizer")]
                self.viz_browser_open,
                #[cfg(feature = "visualizer")]
                &self.viz_preset_entries,
                #[cfg(feature = "visualizer")]
                &self.viz_preset_search,
                #[cfg(feature = "visualizer")]
                self.viz_locked,
                #[cfg(feature = "visualizer")]
                self.viz_beat_sensitivity,
                #[cfg(feature = "visualizer")]
                self.viz_current_preset_name.as_deref(),
            )
            .map(map_now_playing_msg);

            widget::container(expanded).width(Length::Fill).into()
        } else {
            // Collapsed state: normal layout
            let mut layout_col = widget::Column::new().push(
                widget::container(content)
                    .width(Length::Fill)
                    .height(Length::Fill),
            );

            if self.library_scanning {
                layout_col = layout_col.push(
                    widget::container(
                        widget::Row::new()
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

        // WindowBackground pins the app surface to background.base color and
        // sets icon_color/text_color to background.on so all child widgets
        // inherit the correct foreground regardless of maximize state or
        // compositor behavior (which may otherwise paint a transparent/white
        // surface behind the content area).
        let background = widget::container(layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(cosmic::theme::Container::WindowBackground);

        widget::toaster(&self.toasts, background)
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
                    subs.push(Subscription::run_with(
                        MpdPollKey(client),
                        mpd_status_stream,
                    ));
                }
            } else {
                // Local/Subsonic: simple tick for UI updates.
                subs.push(Subscription::run(playback_tick_stream));
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
            subs.push(Subscription::run_with(
                MpdIdleKey { idx, provider },
                mpd_idle_stream,
            ));
        }

        // Convert queue progress ticker (every 500ms while jobs are running)
        if self.convert_jobs.iter().any(|j| j.state == JobState::Running) {
            subs.push(Subscription::run(convert_tick_stream));
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
            let cmd_rx = Arc::clone(&self.viz_cmd_rx_slot);
            let frame_buf = Arc::clone(&self.viz_frame_buf);
            let current_preset = Arc::clone(&self.viz_current_preset_shared);
            subs.push(Subscription::run_with(
                VizRenderKey {
                    pcm,
                    cmd_rx,
                    frame_buf,
                    current_preset,
                },
                projectm_render_stream,
            ));
        }

        // Filesystem watcher subscription — only when the Local provider is active.
        // Uses notify::RecommendedWatcher to watch music_dirs recursively.
        // Debounces events with a 2-second quiet timer before emitting
        // Message::FilesChanged with the collected paths.
        {
            let is_local_active = self
                .registry
                .active()
                .is_some_and(|p| p.provider_type() == crate::provider::ProviderType::Local);

            if is_local_active && !self.config.music_dirs.is_empty() {
                let music_dirs = self.config.music_dirs.clone();
                subs.push(Subscription::run_with(music_dirs, fs_watcher_stream));
            }
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

        // Mouse movement while the visualizer is fullscreen resets the HUD
        // control-card auto-hide idle counter (see `viz_hud_idle_frames`).
        // Scoped to `viz_fullscreen` so ordinary app usage never emits this.
        #[cfg(feature = "visualizer")]
        if self.viz_fullscreen {
            subs.push(cosmic::iced::event::listen_with(|event, _status, _id| {
                if let cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::CursorMoved {
                    ..
                }) = event
                {
                    Some(Message::VizHudActivity)
                } else {
                    None
                }
            }));
        }

        // Space bar to toggle playback (unless captured by a text input widget)
        subs.push(cosmic::iced::event::listen_with(|event, status, _id| {
            if let cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key: cosmic::iced::keyboard::Key::Character(s),
                modifiers,
                ..
            }) = &event
            {
                // Only toggle playback if:
                // 1. Space key pressed
                // 2. No modifier keys (Ctrl, Shift, Alt, etc.)
                // 3. Event not captured by a widget (e.g., text input)
                if s.as_str() == " "
                    && modifiers.is_empty()
                    && status != cosmic::iced::event::Status::Captured
                {
                    return Some(Message::TogglePlayback);
                }
            }
            None
        }));

        // Global key bindings (e.g. Ctrl+F to toggle library search).
        // `listen_with` requires a non-capturing `fn` pointer, so this
        // rebuilds the (tiny) key_binds map on each event rather than
        // capturing `self.key_binds` — both are sourced from the same
        // `key_binds()` function, so the menu's displayed shortcut and
        // this runtime check never drift apart.
        subs.push(cosmic::iced::event::listen_with(|event, status, _id| {
            if status == cosmic::iced::event::Status::Captured {
                return None;
            }
            if let cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) = &event
            {
                for (bind, action) in &key_binds() {
                    if bind.matches(*modifiers, key, Some(physical_key)) {
                        return Some(menu::action::MenuAction::message(action));
                    }
                }
            }
            None
        }));

        // Escape closes the library search (in addition to collapsing the
        // expanded now-playing view, handled above).
        if self.search_active {
            subs.push(cosmic::iced::event::listen_with(|event, _status, _id| {
                if let cosmic::iced::Event::Keyboard(
                    cosmic::iced::keyboard::Event::KeyPressed {
                        key: cosmic::iced::keyboard::Key::Named(
                            cosmic::iced::keyboard::key::Named::Escape,
                        ),
                        ..
                    },
                ) = event
                {
                    Some(Message::ClearLibrarySearch)
                } else {
                    None
                }
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

            Message::TextInputFocused(focused) => {
                self.text_input_focused = focused;
            }

            // -- Library search --
            Message::LibrarySearchChanged(query) => {
                self.library_search = query;
                self.refresh_search_filter();
            }

            Message::ToggleLibrarySearch => {
                self.search_active = !self.search_active;
                if self.search_active {
                    return cosmic::widget::text_input::focus(widget::Id::new(SEARCH_INPUT_ID))
                        .map(cosmic::Action::App);
                }
                self.library_search.clear();
                self.refresh_search_filter();
            }

            Message::ClearLibrarySearch => {
                self.library_search.clear();
                self.search_active = false;
                self.refresh_search_filter();
            }

            Message::CloseToast(id) => {
                self.toasts.remove(id);
            }

            // -- Library --
            Message::ScanLibrary => {
                if let Some(provider) = self.registry.active_shared() {
                    match provider.provider_type() {
                        crate::provider::ProviderType::Local => {
                            self.library_scanning = true;
                            let generation = self.begin_reload_generation();
                            let provider_id = provider.id().to_string();
                            return cosmic::task::future(async move {
                                let count = tokio::task::spawn_blocking(move || {
                                    provider.sync_library().unwrap_or_else(|e| {
                                        tracing::error!("sync_library failed: {e}");
                                        0
                                    })
                                })
                                .await
                                .unwrap_or(0);
                                cosmic::Action::App(Message::LibraryScanComplete {
                                    generation,
                                    provider_id,
                                    count,
                                })
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

            Message::LibraryScanComplete {
                generation,
                provider_id,
                count,
            } => {
                if self.is_stale_reload(generation, &provider_id) {
                    return Task::none();
                }
                self.library_scanning = false;
                tracing::info!("Library scan complete: {count} tracks updated");
                // Only reload if tracks actually changed to avoid unnecessary
                // view rebuilds (which reset scroll position).
                if count > 0 || self.all_tracks.is_empty() {
                    return self.reload_library();
                }
            }

            Message::LibraryLoaded {
                generation,
                provider_id,
                tracks,
                albums,
                artists,
                cover_images,
                artist_avatars,
                cover_art_bytes,
            } => {
                if self.is_stale_reload(generation, &provider_id) {
                    return Task::none();
                }
                self.library_scanning = false;
                self.all_tracks = tracks;
                self.all_albums = albums;
                self.all_artists = artists;
                self.cover_images = cover_images;
                self.artist_avatars = artist_avatars;
                self.cover_art_bytes = cover_art_bytes;
                self.refresh_search_filter();
                // Re-trigger blur now that cover art bytes are available
                let blur_task = self.maybe_update_blurred_cover();
                return blur_task;
            }

            Message::LibraryBatch {
                generation,
                provider_id,
                albums,
                cover_images,
                cover_art_bytes,
            } => {
                if self.is_stale_reload(generation, &provider_id) {
                    return Task::none();
                }
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
                self.refresh_search_filter();

                // Re-trigger blur in case the current track's cover just arrived
                let blur_task = self.maybe_update_blurred_cover();
                return blur_task;
            }

            Message::LibraryLoadComplete {
                generation,
                provider_id,
            } => {
                if self.is_stale_reload(generation, &provider_id) {
                    return Task::none();
                }
                self.library_scanning = false;
                // Final sort
                self.all_tracks.sort_by(|a, b| a.title.cmp(&b.title));
                self.all_albums.sort_by(|a, b| a.name.cmp(&b.name));
                self.all_artists.sort_by(|a, b| a.name.cmp(&b.name));
                self.refresh_search_filter();
                tracing::info!(
                    "Library load complete: {} albums, {} tracks, {} artists",
                    self.all_albums.len(),
                    self.all_tracks.len(),
                    self.all_artists.len()
                );
            }

            // -- Filesystem watcher --
            Message::FilesChanged(paths) => {
                // Filter out directories and non-existent-but-not-deleted paths.
                let paths: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| p.is_file() || !p.exists())
                    .collect();

                if paths.is_empty() {
                    return Task::none();
                }

                tracing::info!("Filesystem watcher detected {} changed paths", paths.len());

                // Run incremental scan on the changed paths in a background task.
                if let Some(provider) = self.registry.active_shared()
                    && provider.provider_type() == crate::provider::ProviderType::Local
                {
                    self.library_scanning = true;
                    let generation = self.begin_reload_generation();
                    let provider_id = provider.id().to_string();
                    return cosmic::task::future(async move {
                        let count = tokio::task::spawn_blocking(move || {
                            // The LocalProvider wraps LibraryDb in a Mutex.
                            // For incremental scan, we access the DB through the
                            // same path used by sync_library — open a temporary DB
                            // connection for the scan, or re-use the provider's scan.
                            // Since LocalProvider::sync_library calls LibraryScanner::scan,
                            // we open the DB directly for scan_paths.
                            let db_path = dirs::data_dir()
                                .unwrap_or_else(|| PathBuf::from("."))
                                .join("lyra")
                                .join("library.db");

                            match crate::library::LibraryDb::open(&db_path) {
                                Ok(db) => {
                                    LibraryScanner::scan_paths(&db, &paths).unwrap_or_else(|e| {
                                        tracing::error!("scan_paths failed: {e}");
                                        0
                                    })
                                }
                                Err(e) => {
                                    tracing::error!("Failed to open DB for incremental scan: {e}");
                                    0
                                }
                            }
                        })
                        .await
                        .unwrap_or(0);
                        cosmic::Action::App(Message::LibraryScanComplete {
                            generation,
                            provider_id,
                            count,
                        })
                    });
                }
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
                            #[cfg(feature = "visualizer")]
                            {
                                self.viz_metadata_opacity = 1.0;
                            }
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
                            #[cfg(feature = "visualizer")]
                            {
                                self.viz_metadata_opacity = 1.0;
                            }
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
                            #[cfg(feature = "visualizer")]
                            {
                                self.viz_metadata_opacity = 1.0;
                            }
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
                    let target = Duration::from_secs_f32(fraction * track.duration.as_secs_f32());
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

            // Task 111: Wire shuffle toggle for MPD
            Message::ToggleShuffle => {
                self.config.shuffle = !self.config.shuffle;
                if let Some(mpd) = self.active_mpd_provider() {
                    let enabled = self.config.shuffle;
                    if let Err(e) = mpd.send_random(enabled) {
                        tracing::error!("MPD send_random: {e}");
                    }
                }
            }

            // Task 112: Wire repeat mode for MPD
            Message::CycleRepeat => {
                self.config.repeat_mode = self.config.repeat_mode.next();
                if let Some(mpd) = self.active_mpd_provider() {
                    let (repeat, single) = match self.config.repeat_mode {
                        crate::config::RepeatMode::None => {
                            (false, mpd_client::commands::SingleMode::Disabled)
                        }
                        crate::config::RepeatMode::All => {
                            (true, mpd_client::commands::SingleMode::Disabled)
                        }
                        crate::config::RepeatMode::One => {
                            (true, mpd_client::commands::SingleMode::Enabled)
                        }
                    };
                    if let Err(e) = mpd.send_repeat(repeat) {
                        tracing::error!("MPD send_repeat: {e}");
                    }
                    if let Err(e) = mpd.send_single(single) {
                        tracing::error!("MPD send_single: {e}");
                    }
                }
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
                        #[cfg(feature = "visualizer")]
                        {
                            self.viz_metadata_opacity = 1.0;
                        }
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
                        #[cfg(feature = "visualizer")]
                        {
                            self.viz_metadata_opacity = 1.0;
                        }
                    }
                }

                let mut position_save_task = Task::none();
                if let Some(track) = self.current_track.clone() {
                    match &*track.provider_id {
                        "radio" => {
                            if let Some(player) = &self.player
                                && let Some(title) = player.icy_title()
                                && !title.is_empty()
                                && let Some(current) = &mut self.current_track
                                && current.title != title
                            {
                                current.title = title;
                            }
                        }
                        "podcast" => {
                            if let Some(episode_id) = self.current_podcast_episode_id {
                                let secs = self.playback_position.as_secs();
                                if secs != self.last_saved_podcast_position_secs && secs % 5 == 0 {
                                    self.last_saved_podcast_position_secs = secs;
                                    let position_ms = self.playback_position.as_millis() as i64;
                                    position_save_task =
                                        self.save_podcast_position(episode_id, position_ms, false);
                                }
                            }
                        }
                        _ => {}
                    }
                    self.handle_scrobble(track);
                }
                return position_save_task;
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
                self.refresh_search_filter();
            }

            Message::ToggleFavoritesFilter => {
                self.favorites_filter = !self.favorites_filter;
                // Clear genre filter when toggling favorites
                if self.favorites_filter {
                    self.genre_filter = None;
                }
            }

            Message::ToggleFavorite(track_id) => {
                if let Some(provider) = self.registry.active_shared() {
                    match provider.toggle_favorite(&track_id) {
                        Ok(new_state) => {
                            // Update the track's is_favorite in our local data.
                            for track in &mut self.all_tracks {
                                if track.id.to_string() == track_id {
                                    track.is_favorite = new_state;
                                }
                            }
                            for album in &mut self.all_albums {
                                for track in &mut album.tracks {
                                    if track.id.to_string() == track_id {
                                        track.is_favorite = new_state;
                                    }
                                }
                            }
                            for artist in &mut self.all_artists {
                                for album in &mut artist.albums {
                                    for track in &mut album.tracks {
                                        if track.id.to_string() == track_id {
                                            track.is_favorite = new_state;
                                        }
                                    }
                                }
                            }
                            // Also update the current playing track if it matches.
                            if let Some(ref mut ct) = self.current_track
                                && ct.id.to_string() == track_id
                            {
                                ct.is_favorite = new_state;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("toggle_favorite failed: {e}");
                        }
                    }
                }
            }

            Message::SetRating(track_id, rating) => {
                if let Some(provider) = self.registry.active_shared() {
                    match provider.set_rating(&track_id, rating) {
                        Ok(()) => {
                            let new_rating = if rating == 0 { None } else { Some(rating) };
                            for track in &mut self.all_tracks {
                                if track.id.to_string() == track_id {
                                    track.rating = new_rating;
                                }
                            }
                            for album in &mut self.all_albums {
                                for track in &mut album.tracks {
                                    if track.id.to_string() == track_id {
                                        track.rating = new_rating;
                                    }
                                }
                            }
                            for artist in &mut self.all_artists {
                                for album in &mut artist.albums {
                                    for track in &mut album.tracks {
                                        if track.id.to_string() == track_id {
                                            track.rating = new_rating;
                                        }
                                    }
                                }
                            }
                            if let Some(ref mut ct) = self.current_track
                                && ct.id.to_string() == track_id
                            {
                                ct.rating = new_rating;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("set_rating failed: {e}");
                        }
                    }
                }
            }

            Message::AddToPlaylist(track_source_uri, playlist_id) => {
                if let Some(provider) = self.registry.active_shared()
                    && let Err(e) = provider.add_to_playlist(&playlist_id, &[track_source_uri])
                {
                    tracing::warn!("add_to_playlist failed: {e}");
                }
            }

            Message::FilterByGenre(genre) => {
                if genre.is_empty() {
                    self.genre_filter = None;
                } else {
                    self.genre_filter = Some(genre);
                    self.favorites_filter = false;
                    // Navigate to Songs view.
                    // Collect entity IDs first to avoid borrow conflict.
                    let entities: Vec<_> = self.nav.iter().collect();
                    for entity in entities {
                        if self
                            .nav
                            .data::<Page>(entity)
                            .is_some_and(|p| *p == Page::Songs)
                        {
                            self.nav.activate(entity);
                            break;
                        }
                    }
                }
            }

            // -- Lyrics --
            Message::ShowLyrics => {
                // Try to load embedded lyrics, regardless of which
                // presentation (overlay vs sidebar) ends up showing them.
                if let Some(track) = &self.current_track {
                    self.lyrics_text = LyricsProvider::from_tags(&track.path)
                        .or_else(|| LyricsProvider::from_lrc_file(&track.path));
                }

                if self.expand_progress > 0.0 {
                    // Expanded now-playing is active: toggle the in-view
                    // overlay (drawn over the cover art / visualizer)
                    // instead of opening the generic sidebar, which would
                    // break the immersive full view.
                    self.lyrics_overlay_active = !self.lyrics_overlay_active;
                } else {
                    self.context_page = ContextPage::Lyrics;
                    self.core.window.show_context = true;
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
                let clamped = value.clamp(-12.0, 12.0);
                if index < self.config.equalizer_bands.len() {
                    self.config.equalizer_bands[index] = clamped;
                }
                // Update the live DSP filter.
                if let Some(ref player) = self.player {
                    player.eq_controller().set_band(index, clamped);
                }
                self.eq_preset = None;
                self.eq_dirty = true;
            }

            Message::EqSetPreset(preset) => {
                let gains = preset.gains();
                self.config.equalizer_bands = gains.to_vec();
                // Update all 10 bands in the live DSP.
                if let Some(ref player) = self.player {
                    player.eq_controller().set_all(&gains);
                }
                self.eq_preset = Some(preset);
            }

            Message::EqToggle(enabled) => {
                self.config.equalizer_enabled = enabled;
                // Enable/bypass the live DSP.
                if let Some(ref player) = self.player {
                    player.eq_controller().set_enabled(enabled);
                }
            }

            Message::EqSetPreamp(value) => {
                let clamped = value.clamp(-20.0, 10.0);
                self.config.equalizer_preamp = clamped;
                // TODO: Apply preamp to audio backend if supported
                tracing::debug!("Preamp set to {:+.1} dB", clamped);
                self.eq_preset = None;
                self.eq_dirty = true;
            }

            Message::EqSelectPreset(name) => {
                if let Some(preset) = self.all_presets.iter().find(|p| p.name == name) {
                    self.config.equalizer_bands = preset.bands.to_vec();
                    self.config.equalizer_preamp = preset.preamp;
                    if let Some(ref player) = self.player {
                        player.eq_controller().set_all(&preset.bands);
                    }
                    self.active_preset_name = Some(name.clone());
                    self.config.active_eq_preset_name = name;
                    self.eq_dirty = false;
                    self.eq_preset = None; // clear legacy preset tracking
                }
            }

            Message::EqSavePreset => {
                if let Some(ref name) = self.active_preset_name {
                    if self.preset_manager.is_builtin_name(name) {
                        tracing::warn!("Cannot overwrite built-in preset '{}'", name);
                    } else {
                        let preset = crate::player::equalizer::EqPresetData {
                            name: name.clone(),
                            bands: {
                                let mut b = [0.0_f32; 10];
                                for (i, v) in
                                    self.config.equalizer_bands.iter().enumerate().take(10)
                                {
                                    b[i] = *v;
                                }
                                b
                            },
                            preamp: self.config.equalizer_preamp,
                            source: crate::player::equalizer::PresetSource::Custom,
                        };
                        if let Err(e) = self.preset_manager.save_preset(&preset) {
                            tracing::error!("Failed to save preset: {}", e);
                        } else {
                            self.all_presets = self.preset_manager.load_all();
                            self.eq_dirty = false;
                        }
                    }
                }
            }

            Message::EqSavePresetAs(name) => {
                if name.trim().is_empty() {
                    tracing::warn!("Cannot save preset with empty name");
                } else if self.preset_manager.is_builtin_name(&name) {
                    tracing::warn!("Cannot use the name of a built-in preset: '{}'", name);
                } else {
                    let preset = crate::player::equalizer::EqPresetData {
                        name: name.clone(),
                        bands: {
                            let mut b = [0.0_f32; 10];
                            for (i, v) in self.config.equalizer_bands.iter().enumerate().take(10) {
                                b[i] = *v;
                            }
                            b
                        },
                        preamp: self.config.equalizer_preamp,
                        source: crate::player::equalizer::PresetSource::Custom,
                    };
                    if let Err(e) = self.preset_manager.save_preset(&preset) {
                        tracing::error!("Failed to save preset: {}", e);
                    } else {
                        self.all_presets = self.preset_manager.load_all();
                        self.active_preset_name = Some(name.clone());
                        self.config.active_eq_preset_name = name;
                        self.eq_dirty = false;
                    }
                }
            }

            Message::EqDeletePreset => {
                if let Some(ref name) = self.active_preset_name {
                    if self.preset_manager.is_builtin_name(name) {
                        tracing::warn!("Cannot delete built-in preset '{}'", name);
                    } else if let Err(e) = self.preset_manager.delete_preset(name) {
                        tracing::error!("Failed to delete preset: {}", e);
                    } else {
                        self.all_presets = self.preset_manager.load_all();
                        self.active_preset_name = None;
                        self.config.active_eq_preset_name = String::new();
                        self.eq_dirty = false;
                    }
                }
            }

            Message::EqSaveAsNameChanged(name) => {
                self.save_as_name = name;
            }

            Message::EqResetPreset => {
                self.config.equalizer_bands = vec![0.0; 10];
                self.config.equalizer_preamp = 0.0;
                if let Some(ref player) = self.player {
                    player.eq_controller().set_all(&[0.0; 10]);
                }
                self.active_preset_name = None;
                self.config.active_eq_preset_name = String::new();
                self.eq_preset = None;
                self.eq_dirty = false;
            }

            // -- AutoEQ --
            Message::AutoEQSearchChanged(query) => {
                self.autoeq_search = query;
            }

            Message::FetchAutoEQIndex => {
                if self.autoeq_loading {
                    return Task::none(); // already fetching
                }
                self.autoeq_loading = true;

                let cache_dir = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("lyra")
                    .join("autoeq");
                let timeout = std::time::Duration::from_secs(30);

                return cosmic::task::future(async move {
                    let result = match crate::autoeq::AutoEQManager::new(cache_dir, timeout) {
                        Ok(mut manager) => manager.fetch_index().await.map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    cosmic::Action::App(Message::AutoEQIndexLoaded(result))
                });
            }

            Message::AutoEQIndexLoaded(result) => {
                self.autoeq_loading = false;
                match result {
                    Ok(profiles) => {
                        tracing::info!("Loaded {} AutoEQ profiles", profiles.len());
                        self.autoeq_profiles = profiles;
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch AutoEQ index: {}", e);
                    }
                }
            }

            Message::EqSelectAutoEQ(path) => {
                // Find the profile metadata to get the name
                let name = self
                    .autoeq_profiles
                    .iter()
                    .find(|p| p.path == path)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();

                tracing::info!("Fetching AutoEQ profile: {} ({})", name, path);

                let cache_dir = dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("lyra")
                    .join("autoeq");
                let timeout = std::time::Duration::from_secs(30);

                return cosmic::task::future(async move {
                    let result = match crate::autoeq::AutoEQManager::new(cache_dir, timeout) {
                        Ok(mut manager) => manager
                            .fetch_profile(&path)
                            .await
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    cosmic::Action::App(Message::AutoEQProfileLoaded(result))
                });
            }

            Message::AutoEQProfileLoaded(result) => {
                match result {
                    Ok(profile) => {
                        // Apply bands + preamp
                        self.config.equalizer_bands = profile.bands.to_vec();
                        self.config.equalizer_preamp = profile.preamp;
                        if let Some(ref player) = self.player {
                            player.eq_controller().set_all(&profile.bands);
                        }
                        // Clear preset selection (user can "Save As" to keep)
                        self.active_preset_name = None;
                        self.config.active_eq_preset_name = String::new();
                        self.eq_dirty = false;
                        self.eq_preset = None;

                        tracing::info!(
                            "Applied AutoEQ profile: {} (preamp: {:+.1} dB)",
                            profile.name,
                            profile.preamp
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to load AutoEQ profile: {}", e);
                    }
                }
            }

            Message::ToggleAlbumsViewMode => {
                self.config.albums_view_mode = self.config.albums_view_mode.toggled();
                self.save_config();
            }
            Message::ToggleArtistsViewMode => {
                self.config.artists_view_mode = self.config.artists_view_mode.toggled();
                self.save_config();
            }
            Message::ToggleGenresViewMode => {
                self.config.genres_view_mode = self.config.genres_view_mode.toggled();
                self.save_config();
            }

            // -- Settings --
            Message::AddMusicDir => {
                // Launch the XDG Desktop Portal directory picker asynchronously.
                return cosmic::task::future(async {
                    let result = async {
                        use ashpd::desktop::file_chooser::SelectedFiles;

                        let selected = SelectedFiles::open_file()
                            .title("Select Music Directory")
                            .directory(true)
                            .modal(true)
                            .send()
                            .await
                            .map_err(|e| format!("Portal request failed: {e}"))?
                            .response()
                            .map_err(|e| format!("Portal response failed: {e}"))?;

                        let uris = selected.uris();
                        if let Some(uri) = uris.first() {
                            let uri_str = uri.as_str();
                            uri_str
                                .strip_prefix("file://")
                                .ok_or_else(|| format!("Not a local file URI: {uri_str}"))
                                .and_then(|encoded_path| {
                                    urlencoding::decode(encoded_path)
                                        .map(|decoded| PathBuf::from(decoded.as_ref()))
                                        .map_err(|e| format!("Could not decode URI path: {e}"))
                                })
                        } else {
                            Err("No directory selected".to_string())
                        }
                    }
                    .await;
                    cosmic::Action::App(Message::DirPickerResult(result))
                });
            }

            Message::DirPickerResult(result) => {
                match result {
                    Ok(path) => {
                        // Deduplicate: check if the path is already in music_dirs.
                        if self.config.music_dirs.contains(&path) {
                            tracing::info!("Directory already in music_dirs: {}", path.display());
                        } else {
                            tracing::info!("Adding music directory: {}", path.display());
                            self.config.music_dirs.push(path);
                            self.save_config();
                            // Re-register the Local provider with updated scan dirs.
                            self.reinit_local_provider();
                            // Trigger a library rescan.
                            return cosmic::task::message(cosmic::Action::App(
                                Message::ScanLibrary,
                            ));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Directory picker failed: {e}");
                    }
                }
            }

            Message::RemoveMusicDir(index) => {
                if index < self.config.music_dirs.len() {
                    let removed = self.config.music_dirs.remove(index);
                    tracing::info!("Removed music directory: {}", removed.display());
                    self.save_config();
                    // Re-register the Local provider with updated scan dirs.
                    self.reinit_local_provider();
                    // Trigger a library rescan.
                    return cosmic::task::message(cosmic::Action::App(Message::ScanLibrary));
                }
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
                    let mut entry = state.to_config();
                    // Store password to keyring immediately on save so it is
                    // never left as plaintext in the config file.
                    if let Some(pw) = entry.password.as_deref().filter(|p| !p.is_empty()) {
                        match crate::credentials::store_password(&entry.id, pw) {
                            Ok(()) => {
                                entry.password_in_keyring = true;
                                entry.password = None;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to store MPD password for '{}' in keyring, \
                                     keeping plaintext in config: {e}",
                                    entry.id
                                );
                            }
                        }
                    }
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

                // Update connection status for the matching provider card, and
                // surface a toast only on the transition into this failure
                // state — the idle/command reconnect loop retries every 5s
                // and would otherwise spam a toast on every retry.
                let new_status = format!("{}: {error}", fl!("connection-failed"));
                if let Some(idx) = self
                    .mpd_edit_states
                    .iter()
                    .position(|s| s.id == provider_id)
                {
                    let already_failed = self
                        .mpd_connection_status
                        .get(idx)
                        .is_some_and(|s| s.as_deref() == Some(new_status.as_str()));

                    if let Some(s) = self.mpd_connection_status.get_mut(idx) {
                        *s = Some(new_status);
                    }

                    if !already_failed {
                        let provider_name = self
                            .mpd_edit_states
                            .get(idx)
                            .map(|s| s.name.clone())
                            .filter(|n| !n.is_empty())
                            .unwrap_or(provider_id);
                        return self.push_toast(widget::toaster::Toast::new(fl!(
                            "toast-provider-connect-failed",
                            provider = provider_name,
                            reason = error
                        )));
                    }
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
                    let mut entry = state.to_config();
                    // Store password to keyring immediately on save so it is
                    // never left as plaintext in the config file.
                    if let Some(pw) = entry.password.as_deref().filter(|p| !p.is_empty()) {
                        match crate::credentials::store_password(&entry.id, pw) {
                            Ok(()) => {
                                entry.password_in_keyring = true;
                                entry.password = None;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to store Subsonic password for '{}' in keyring, \
                                     keeping plaintext in config: {e}",
                                    entry.id
                                );
                            }
                        }
                    }
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
                            let auth = opensubsonic::Auth::token(&username, &password);
                            let mut client = opensubsonic::Client::new(&url, auth)
                                .map_err(|e| format!("Client: {e}"))?;
                            if accept_invalid_certs {
                                client = client
                                    .with_danger_accept_invalid_certs()
                                    .map_err(|e| format!("TLS: {e}"))?;
                            }
                            client.ping().await.map_err(|e| format!("Ping: {e}"))?;
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

            // Task 109: Subsonic transcoding bitrate/format changes
            Message::SubsonicTranscodingBitrate(i, bitrate) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.transcoding_max_bitrate = bitrate;
                }
            }
            Message::SubsonicTranscodingFormat(i, format) => {
                if let Some(state) = self.subsonic_edit_states.get_mut(i) {
                    state.transcoding_format = format;
                }
            }

            // Task 113: Wire crossfade config change to player and MPD
            Message::SetCrossfade(secs) => {
                self.config.crossfade_duration_secs = secs;

                // Apply to local backend
                if let Some(player) = &mut self.player {
                    player.set_crossfade(secs);
                }

                // Forward to MPD if active
                if let Some(mpd) = self.active_mpd_provider() {
                    let seconds = secs as u64;
                    return self.dispatch_mpd(async move {
                        mpd.send_crossfade(seconds)
                            .map_err(|e| format!("crossfade: {e}"))
                    });
                }
            }

            // Task 114: Wire replay gain mode change to player and MPD
            Message::SetReplayGainMode(mode) => {
                self.config.replay_gain_mode = mode;

                // Apply to local backend
                if let Some(player) = &mut self.player {
                    player.set_replay_gain_mode(mode);
                }

                // Forward to MPD if active — convert config type to mpd_client type
                if let Some(mpd) = self.active_mpd_provider() {
                    use mpd_client::commands::ReplayGainMode as MpdReplayGainMode;
                    let mpd_mode = match mode {
                        ReplayGainMode::Off => MpdReplayGainMode::Off,
                        ReplayGainMode::Track => MpdReplayGainMode::Track,
                        ReplayGainMode::Album => MpdReplayGainMode::Album,
                        ReplayGainMode::Auto => MpdReplayGainMode::Auto,
                    };
                    return self.dispatch_mpd(async move {
                        mpd.send_replay_gain_mode(mpd_mode)
                            .map_err(|e| format!("replay_gain_mode: {e}"))
                    });
                }
            }

            Message::ExpandNowPlaying => {
                self.expand_target = Some(1.0);
                self.expand_anim_start = Some(std::time::Instant::now());
                self.expand_anim_from = self.expand_progress;
            }

            Message::CollapseNowPlaying => {
                // If the preset browser is open, Escape (or the collapse
                // button) closes it first instead of collapsing the whole
                // view. Handled here rather than branching in the Escape
                // subscription itself, since `listen_with` only accepts a
                // non-capturing `fn` pointer — it can't read
                // `viz_browser_open`.
                #[cfg(feature = "visualizer")]
                if self.viz_browser_open {
                    self.viz_browser_open = false;
                    return Task::none();
                }
                self.expand_target = Some(0.0);
                self.expand_anim_start = Some(std::time::Instant::now());
                self.expand_anim_from = self.expand_progress;
                self.lyrics_overlay_active = false;
                // Leaving the expanded view must also leave fullscreen, else the
                // header bar / nav sidebar would stay hidden with no visualizer.
                #[cfg(feature = "visualizer")]
                self.exit_viz_fullscreen();
            }

            Message::ExpandAnimTick => {
                use crate::views::now_playing::animation;

                if let (Some(target), Some(start)) = (self.expand_target, self.expand_anim_start) {
                    let elapsed = start.elapsed().as_secs_f32() * 1000.0;
                    let t = (elapsed / animation::ANIMATION_DURATION_MS).min(1.0);

                    // Apply easing based on direction
                    let eased = if target > self.expand_anim_from {
                        animation::ease_out(t)
                    } else {
                        animation::ease_in(t)
                    };

                    self.expand_progress = animation::lerp(self.expand_anim_from, target, eased);

                    // Check if animation is complete
                    if t >= 1.0 {
                        self.expand_progress = target;
                        self.expand_target = None;
                        self.expand_anim_start = None;
                    }
                }
            }

            Message::BlurReady(key, handle) => {
                // Guard against stale results: only apply if this blur is still
                // for the current track's album. A slow computation may finish
                // after the user has already moved to a different track.
                let current_key = self.current_track.as_ref().map(|t| {
                    let artist = if t.album_artist.is_empty() {
                        &t.artist
                    } else {
                        &t.album_artist
                    };
                    crate::library::CoverArt::album_key(artist, &t.album)
                });
                if current_key.as_ref() == Some(&key) {
                    self.blurred_cover = Some(handle);
                    self.blurred_cover_key = Some(key);
                }
                // If stale, discard silently — the correct blur is either already
                // cached or will be requested by the next maybe_update_blurred_cover call.
            }

            // -- Visualizer messages (cfg-gated) --
            #[cfg(feature = "visualizer")]
            Message::ToggleVisualizer => {
                self.visualizer_active = !self.visualizer_active;
                // When turning the visualizer off, the blurred cover background
                // takes over. If it was never computed (e.g. track changed while
                // viz was active and bytes weren't cached yet), trigger it now.
                if !self.visualizer_active {
                    self.exit_viz_fullscreen();
                    self.viz_browser_open = false;
                    // Force retry even if key matches — blurred_cover may be None.
                    if self.blurred_cover.is_none() {
                        self.blurred_cover_key = None;
                    }
                    return self.maybe_update_blurred_cover();
                }
            }

            #[cfg(feature = "visualizer")]
            Message::NextVisualizerPreset => {
                let _ = self
                    .viz_cmd_tx
                    .send(crate::views::now_playing::visualizer::VizCommand::NextPreset);
            }

            #[cfg(feature = "visualizer")]
            Message::VisualizerFrameReady => {
                // The shared VizFrameBuffer already has the new pixels.
                // This message just triggers a view redraw so the Shader
                // widget picks them up in its next prepare() call.

                // Resync the UI-local current-preset name from whatever
                // the render thread most recently set (see
                // `viz_current_preset_shared`'s doc comment). Overwrites
                // any optimistic value `LoadVizPreset` already set with
                // the render thread's own value — they converge within a
                // frame either way.
                if let Ok(name) = self.viz_current_preset_shared.lock() {
                    self.viz_current_preset_name = name.clone();
                }

                // Decay metadata overlay (~4 seconds at 30 fps = 120 frames).
                // No inner cfg needed — this arm is only reachable with the visualizer feature.
                if self.viz_metadata_opacity > 0.0 {
                    self.viz_metadata_opacity =
                        (self.viz_metadata_opacity - (1.0 / 120.0)).max(0.0);
                }

                // Tick the HUD auto-hide idle counter (only while
                // fullscreen, and not while the preset browser forces the
                // HUD visible — no point counting otherwise, and it avoids
                // a stale huge count instantly hiding the HUD the moment
                // the browser closes).
                if self.viz_fullscreen && !self.viz_hud_pointer_over && !self.viz_browser_open {
                    self.viz_hud_idle_frames = self.viz_hud_idle_frames.saturating_add(1);
                }
            }

            #[cfg(feature = "visualizer")]
            Message::VizHudActivity => {
                self.viz_hud_idle_frames = 0;
            }

            #[cfg(feature = "visualizer")]
            Message::VizHudPointerEnter => {
                self.viz_hud_pointer_over = true;
                self.viz_hud_idle_frames = 0;
            }

            #[cfg(feature = "visualizer")]
            Message::VizHudPointerExit => {
                self.viz_hud_pointer_over = false;
                self.viz_hud_idle_frames = 0;
            }

            #[cfg(feature = "visualizer")]
            Message::ToggleVisualizerFullscreen => {
                // COSMIC uses client-side decorations, so the header bar *is* the
                // titlebar. "Fullscreen" here means hiding the header bar and the
                // nav sidebar so the visualizer fills the whole window. (The old
                // `toggle_decorations` call targeted server-side decorations,
                // which COSMIC never draws, so it was a silent no-op.)
                self.viz_fullscreen = !self.viz_fullscreen;
                if self.viz_fullscreen {
                    self.viz_hud_idle_frames = 0;
                    self.viz_prev_nav_active = self.core.nav_bar_active();
                    self.core.window.show_headerbar = false;
                    self.core.nav_bar_set_toggled(false);
                } else {
                    self.core.window.show_headerbar = true;
                    self.core.nav_bar_set_toggled(self.viz_prev_nav_active);
                }
            }

            #[cfg(feature = "visualizer")]
            Message::TogglePresetBrowser => {
                self.viz_browser_open = !self.viz_browser_open;
                if self.viz_browser_open {
                    self.viz_hud_idle_frames = 0;
                    if !self.viz_presets_scan_started {
                        self.viz_presets_scan_started = true;
                        return cosmic::task::future(async move {
                            let dirs = crate::views::now_playing::visualizer::preset_search_dirs(
                                dirs::data_dir().map(|d| d.join("projectm").join("presets")),
                            );
                            let entries = tokio::task::spawn_blocking(move || {
                                crate::views::now_playing::visualizer::scan_presets(&dirs)
                            })
                            .await
                            .unwrap_or_default();
                            cosmic::Action::App(Message::VizPresetsScanned(entries))
                        });
                    }
                }
            }

            #[cfg(feature = "visualizer")]
            Message::PresetSearchInput(query) => {
                self.viz_preset_search = query;
            }

            #[cfg(feature = "visualizer")]
            Message::LoadVizPreset(path) => {
                // Optimistic: reflect the load immediately so the browser
                // highlights the new row without waiting on the render
                // thread's next frame — see `VisualizerFrameReady`.
                self.viz_current_preset_name =
                    path.file_stem().map(|s| s.to_string_lossy().into_owned());
                let _ = self
                    .viz_cmd_tx
                    .send(crate::views::now_playing::visualizer::VizCommand::LoadPreset(path));
            }

            #[cfg(feature = "visualizer")]
            Message::SetVizLocked(locked) => {
                self.viz_locked = locked;
                let _ = self
                    .viz_cmd_tx
                    .send(crate::views::now_playing::visualizer::VizCommand::SetLocked(locked));
            }

            #[cfg(feature = "visualizer")]
            Message::SetVizBeatSensitivity(sensitivity) => {
                self.viz_beat_sensitivity = sensitivity;
                let _ = self.viz_cmd_tx.send(
                    crate::views::now_playing::visualizer::VizCommand::SetBeatSensitivity(
                        sensitivity,
                    ),
                );
            }

            #[cfg(feature = "visualizer")]
            Message::VizPresetsScanned(entries) => {
                self.viz_preset_entries = entries;
            }

            // -- Playlists view --
            Message::SelectPlaylist(idx) => {
                self.selected_playlist = Some(idx);
                self.rename_playlist_input = self
                    .playlists
                    .get(idx)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
            }

            Message::BackToPlaylistList => {
                self.selected_playlist = None;
                self.rename_playlist_input.clear();
            }

            Message::CreatePlaylist(name) => {
                if let Some(provider) = self.registry.active_shared() {
                    match provider.create_playlist(&name) {
                        Ok(_) => {
                            self.new_playlist_name.clear();
                            return self.load_playlists();
                        }
                        Err(e) => {
                            tracing::error!("Failed to create playlist: {e}");
                        }
                    }
                }
            }

            Message::DeletePlaylist(idx) => {
                if let Some(playlist) = self.playlists.get(idx) {
                    let id = playlist.id.clone();
                    if let Some(provider) = self.registry.active_shared() {
                        match provider.delete_playlist(&id) {
                            Ok(()) => {
                                self.selected_playlist = None;
                                return self.load_playlists();
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete playlist: {e}");
                            }
                        }
                    }
                }
            }

            Message::RenamePlaylist(idx, new_name) => {
                if let Some(playlist) = self.playlists.get(idx) {
                    let id = playlist.id.clone();
                    if let Some(provider) = self.registry.active_shared() {
                        match provider.rename_playlist(&id, &new_name) {
                            Ok(()) => {
                                return self.load_playlists();
                            }
                            Err(e) => {
                                tracing::error!("Failed to rename playlist: {e}");
                            }
                        }
                    }
                }
            }

            Message::PlayPlaylist(idx) => {
                if let Some(playlist) = self.playlists.get(idx)
                    && !playlist.tracks.is_empty()
                {
                    return self.play_track_list(playlist.tracks.clone(), 0);
                }
            }

            Message::PlayPlaylistTrack(pl_idx, track_idx) => {
                if let Some(playlist) = self.playlists.get(pl_idx) {
                    return self.play_track_list(playlist.tracks.clone(), track_idx);
                }
            }

            Message::RemovePlaylistTrack(_pl_idx, _track_idx) => {
                // Track removal from playlists requires provider-specific
                // support that isn't uniformly available yet. Log and reload.
                tracing::info!("Track removal from playlist not yet implemented");
                return self.load_playlists();
            }

            Message::NewPlaylistNameChanged(name) => {
                self.new_playlist_name = name;
            }

            Message::RenamePlaylistInput(_idx, text) => {
                self.rename_playlist_input = text;
            }

            Message::PlaylistsLoaded(playlists) => {
                self.playlists = playlists;
                self.refresh_search_filter();
            }

            // -- Genres view --
            Message::SelectGenre(idx) => {
                self.selected_genre = Some(idx);
                // Load tracks for the selected genre
                if let Some(genre_name) = self.all_genres.get(idx) {
                    let genre = genre_name.clone();
                    if let Some(provider) = self.registry.active_shared() {
                        let tracks = provider.get_tracks_by_genre(&genre).unwrap_or_default();
                        self.genre_tracks = tracks;
                    } else {
                        // Fall back to filtering local tracks
                        self.genre_tracks = self
                            .all_tracks
                            .iter()
                            .filter(|t| t.genre.eq_ignore_ascii_case(&genre))
                            .cloned()
                            .collect();
                    }
                }
            }

            Message::BackToGenreGrid => {
                self.selected_genre = None;
                self.genre_tracks.clear();
            }

            Message::PlayGenreTrack(idx) => {
                if !self.genre_tracks.is_empty() {
                    return self.play_track_list(self.genre_tracks.clone(), idx);
                }
            }

            Message::GenresLoaded(genres) => {
                self.all_genres = genres;
                self.refresh_search_filter();
            }

            Message::GenreTracksLoaded(tracks) => {
                self.genre_tracks = tracks;
            }

            // -- Podcasts --
            Message::PodcastSearchChanged(query) => {
                self.podcast_search_query = query;
            }

            Message::PodcastSearchSubmit => {
                let query = self.podcast_search_query.trim().to_string();
                if query.is_empty() {
                    return Task::none();
                }
                self.podcast_search_loading = true;
                return cosmic::task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let client = reqwest::blocking::Client::new();
                        podcast::search_itunes(&client, &query)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    cosmic::Action::App(Message::PodcastSearchResults(result))
                });
            }

            Message::PodcastSearchResults(result) => {
                self.podcast_search_loading = false;
                match result {
                    Ok(results) => {
                        let icon_urls: Vec<String> =
                            results.iter().map(|r| r.image.clone()).collect();
                        self.podcast_search_results = results;
                        return self.load_online_icons(icon_urls);
                    }
                    Err(e) => {
                        return self.push_toast(widget::toaster::Toast::new(fl!(
                            "toast-podcast-search-failed",
                            reason = e
                        )));
                    }
                }
            }

            Message::PodcastAddUrlChanged(url) => {
                self.podcast_add_url = url;
            }

            Message::SubscribePodcast(feed_url) => {
                let feed_url = feed_url.trim().to_string();
                if feed_url.is_empty() {
                    return Task::none();
                }
                self.podcast_add_url.clear();
                return cosmic::task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let client = reqwest::blocking::Client::new();
                        let (meta, episodes) = podcast::fetch_feed(&client, &feed_url)?;
                        let store = open_online_store()?;
                        let id = store.add_podcast(&feed_url, &meta)?;
                        store.upsert_episodes(id, &episodes)?;
                        store.touch_podcast_refresh(id, &meta, now_epoch())?;
                        Ok(())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    cosmic::Action::App(Message::PodcastSubscribed(result))
                });
            }

            Message::PodcastSubscribed(result) => match result {
                Ok(()) => {
                    self.podcast_search_results.clear();
                    self.podcast_search_query.clear();
                    return self.load_podcasts();
                }
                Err(e) => {
                    return self.push_toast(widget::toaster::Toast::new(fl!(
                        "toast-podcast-subscribe-failed",
                        reason = e
                    )));
                }
            },

            Message::PodcastsLoaded(podcasts) => {
                let icon_urls: Vec<String> = podcasts.iter().map(|p| p.image_url.clone()).collect();
                self.podcasts = podcasts;
                return self.load_online_icons(icon_urls);
            }

            Message::SelectPodcast(idx) => {
                self.selected_podcast = Some(idx);
                if let Some(podcast) = self.podcasts.get(idx) {
                    return self.load_podcast_episodes(podcast.id);
                }
            }

            Message::BackToPodcastList => {
                self.selected_podcast = None;
                self.podcast_episodes.clear();
            }

            Message::RemovePodcast(idx) => {
                if let Some(podcast) = self.podcasts.get(idx) {
                    let id = podcast.id;
                    match open_online_store().and_then(|store| store.remove_podcast(id)) {
                        Ok(()) => {
                            if self.selected_podcast == Some(idx) {
                                self.selected_podcast = None;
                                self.podcast_episodes.clear();
                            }
                            return self.load_podcasts();
                        }
                        Err(e) => tracing::error!("Failed to remove podcast: {e}"),
                    }
                }
            }

            Message::RefreshPodcast(idx) => {
                if let Some(podcast) = self.podcasts.get(idx) {
                    return refresh_podcast_task(podcast.id, podcast.feed_url.clone());
                }
            }

            Message::RefreshAllPodcasts => {
                let tasks: Vec<_> = self
                    .podcasts
                    .iter()
                    .map(|p| refresh_podcast_task(p.id, p.feed_url.clone()))
                    .collect();
                return Task::batch(tasks);
            }

            Message::PodcastRefreshed(id, result) => match result {
                Ok(()) => {
                    let reload_task = self.load_podcasts();
                    let is_selected = self
                        .selected_podcast
                        .and_then(|i| self.podcasts.get(i))
                        .is_some_and(|p| p.id == id);
                    let episodes_task = if is_selected {
                        self.load_podcast_episodes(id)
                    } else {
                        Task::none()
                    };
                    return Task::batch([reload_task, episodes_task]);
                }
                Err(e) => {
                    return self.push_toast(widget::toaster::Toast::new(fl!(
                        "toast-podcast-refresh-failed",
                        reason = e
                    )));
                }
            },

            Message::PodcastEpisodesLoaded(podcast_id, episodes) => {
                let is_selected = self
                    .selected_podcast
                    .and_then(|i| self.podcasts.get(i))
                    .is_some_and(|p| p.id == podcast_id);
                if is_selected {
                    self.podcast_episodes = episodes;
                }
            }

            Message::PlayPodcastEpisode(idx) => {
                let Some(podcast_idx) = self.selected_podcast else {
                    return Task::none();
                };
                let Some(episode) = self.podcast_episodes.get(idx).cloned() else {
                    return Task::none();
                };
                let Some(podcast) = self.podcasts.get(podcast_idx) else {
                    return Task::none();
                };
                let track = Track {
                    id: -1,
                    path: PathBuf::new(),
                    title: episode.title.clone(),
                    artist: podcast.title.clone(),
                    album_artist: podcast.title.clone(),
                    album: podcast.title.clone(),
                    genre: String::new(),
                    track_number: 0,
                    disc_number: 0,
                    year: 0,
                    duration: Duration::from_secs(episode.duration_secs.max(0) as u64),
                    bitrate: 0,
                    sample_rate: 0,
                    provider_id: Arc::from("podcast"),
                    source_uri: episode.enclosure_url.clone(),
                    is_favorite: false,
                    rating: None,
                    rg_track_gain: None,
                    rg_album_gain: None,
                };
                let play_task = self.play_track_list(vec![track], 0);
                self.current_podcast_episode_id = Some(episode.id);
                self.last_saved_podcast_position_secs = 0;
                if episode.position_ms > 0 {
                    let resume_at = Duration::from_millis(episode.position_ms as u64);
                    if let Some(player) = &mut self.player {
                        let _ = player.seek(resume_at);
                    }
                    self.playback_position = resume_at;
                }
                return play_task;
            }

            Message::TogglePodcastEpisodePlayed(idx) => {
                if let Some(episode) = self.podcast_episodes.get(idx).cloned() {
                    let new_played = !episode.played;
                    let result = open_online_store().and_then(|store| {
                        store.save_episode_position(episode.id, episode.position_ms, new_played)
                    });
                    match result {
                        Ok(()) => {
                            if let Some(ep) = self.podcast_episodes.get_mut(idx) {
                                ep.played = new_played;
                            }
                        }
                        Err(e) => tracing::error!("Failed to toggle played: {e}"),
                    }
                }
            }

            Message::OnlineIconLoaded(url, bytes) => {
                if !bytes.is_empty() {
                    self.online_icons.insert(url, widget::icon::from_raster_bytes(bytes));
                }
            }

            // -- Radio --
            Message::RadioSearchChanged(query) => {
                self.radio_search_query = query;
            }

            Message::RadioSearchSubmit => {
                let query = self.radio_search_query.trim().to_string();
                if query.is_empty() {
                    return Task::none();
                }
                self.radio_search_loading = true;
                return cosmic::task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let client = reqwest::blocking::Client::new();
                        radio::search_stations(&client, &query)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    cosmic::Action::App(Message::RadioSearchResults(result))
                });
            }

            Message::RadioSearchResults(result) => {
                self.radio_search_loading = false;
                match result {
                    Ok(results) => {
                        let icon_urls: Vec<String> =
                            results.iter().map(|r| r.favicon.clone()).collect();
                        self.radio_search_results = results;
                        return self.load_online_icons(icon_urls);
                    }
                    Err(e) => {
                        return self.push_toast(widget::toaster::Toast::new(fl!(
                            "toast-radio-search-failed",
                            reason = e
                        )));
                    }
                }
            }

            Message::RadioAddNameChanged(name) => {
                self.radio_add_name = name;
            }

            Message::RadioAddUrlChanged(url) => {
                self.radio_add_url = url;
            }

            Message::AddRadioStation {
                name,
                stream_url,
                homepage,
                favicon_url,
                tags,
            } => {
                let result = open_online_store().and_then(|store| {
                    store.add_radio_station(&name, &stream_url, &homepage, &favicon_url, &tags)
                });
                match result {
                    Ok(_) => {
                        self.radio_add_name.clear();
                        self.radio_add_url.clear();
                        return self.load_radio_stations();
                    }
                    Err(e) => tracing::error!("Failed to add radio station: {e}"),
                }
            }

            Message::AddRadioFromSearch(idx) => {
                if let Some(result) = self.radio_search_results.get(idx).cloned() {
                    return cosmic::task::message(cosmic::Action::App(Message::AddRadioStation {
                        name: result.name,
                        stream_url: result.url,
                        homepage: result.homepage,
                        favicon_url: result.favicon,
                        tags: result.tags,
                    }));
                }
            }

            Message::RadioStationsLoaded(stations) => {
                let icon_urls: Vec<String> =
                    stations.iter().map(|s| s.favicon_url.clone()).collect();
                self.radio_stations = stations;
                return self.load_online_icons(icon_urls);
            }

            Message::RemoveRadioStation(idx) => {
                if let Some(station) = self.radio_stations.get(idx) {
                    let id = station.id;
                    match open_online_store().and_then(|store| store.remove_radio_station(id)) {
                        Ok(()) => return self.load_radio_stations(),
                        Err(e) => tracing::error!("Failed to remove radio station: {e}"),
                    }
                }
            }

            Message::PlayRadioStation(idx) => {
                if let Some(station) = self.radio_stations.get(idx) {
                    return resolve_and_play_radio(station.name.clone(), station.stream_url.clone());
                }
            }

            Message::PlayRadioSearchResult(idx) => {
                if let Some(result) = self.radio_search_results.get(idx) {
                    return resolve_and_play_radio(result.name.clone(), result.url.clone());
                }
            }

            Message::RadioStreamResolved { name, result } => match result {
                Ok(resolved_url) => {
                    let track = Track {
                        id: -1,
                        path: PathBuf::new(),
                        title: name,
                        artist: String::new(),
                        album_artist: String::new(),
                        album: String::new(),
                        genre: String::new(),
                        track_number: 0,
                        disc_number: 0,
                        year: 0,
                        duration: Duration::ZERO,
                        bitrate: 0,
                        sample_rate: 0,
                        provider_id: Arc::from("radio"),
                        source_uri: resolved_url,
                        is_favorite: false,
                        rating: None,
                        rg_track_gain: None,
                        rg_album_gain: None,
                    };
                    return self.play_track_list(vec![track], 0);
                }
                Err(e) => {
                    return self.push_toast(widget::toaster::Toast::new(fl!(
                        "toast-radio-play-failed",
                        reason = e
                    )));
                }
            },

            // -- Convert / transcode / rip --
            Message::ConvertAddFiles => {
                return cosmic::task::future(async {
                    let result = async {
                        use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};

                        let mut filter = FileFilter::new("Audio, Video & CUE Files");
                        for ext in crate::player::engine::decoder::SUPPORTED_EXTENSIONS
                            .iter()
                            .chain(["cue", "mkv", "mov", "avi"].iter())
                        {
                            filter = filter.glob(&format!("*.{ext}"));
                        }

                        let selected = SelectedFiles::open_file()
                            .title("Select Audio/Video Files or a CUE Sheet")
                            .multiple(true)
                            .modal(true)
                            .filter(filter)
                            .send()
                            .await
                            .map_err(|e| format!("Portal request failed: {e}"))?
                            .response()
                            .map_err(|e| format!("Portal response failed: {e}"))?;

                        let mut paths = Vec::new();
                        for uri in selected.uris() {
                            let uri_str = uri.as_str();
                            let path = uri_str
                                .strip_prefix("file://")
                                .ok_or_else(|| format!("Not a local file URI: {uri_str}"))
                                .and_then(|encoded| {
                                    urlencoding::decode(encoded)
                                        .map(|d| PathBuf::from(d.as_ref()))
                                        .map_err(|e| format!("Could not decode URI path: {e}"))
                                })?;
                            paths.push(path);
                        }
                        if paths.is_empty() {
                            Err("No files selected".to_string())
                        } else {
                            Ok(paths)
                        }
                    }
                    .await;
                    cosmic::Action::App(Message::ConvertFilesPicked(result))
                });
            }

            Message::ConvertFilesPicked(result) => match result {
                Ok(paths) => {
                    let format = OutputFormat::ALL[self.convert_format_index];
                    let target_rate = convert::SAMPLE_RATE_OPTIONS[self.convert_rate_index];
                    for path in paths {
                        let kind = if path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
                        {
                            JobKind::CueSplit
                        } else {
                            JobKind::Convert
                        };
                        let id = self.convert_next_id;
                        self.convert_next_id += 1;
                        self.convert_jobs.push(ConvertJob::new(
                            id,
                            path,
                            kind,
                            format,
                            target_rate,
                            self.convert_out_dir.clone(),
                        ));
                    }
                }
                Err(e) => tracing::warn!("convert: file picker failed: {e}"),
            },

            Message::ConvertPickOutputDir => {
                return cosmic::task::future(async {
                    let result = async {
                        use ashpd::desktop::file_chooser::SelectedFiles;

                        let selected = SelectedFiles::open_file()
                            .title("Select Output Directory")
                            .directory(true)
                            .modal(true)
                            .send()
                            .await
                            .map_err(|e| format!("Portal request failed: {e}"))?
                            .response()
                            .map_err(|e| format!("Portal response failed: {e}"))?;

                        let uris = selected.uris();
                        if let Some(uri) = uris.first() {
                            let uri_str = uri.as_str();
                            uri_str
                                .strip_prefix("file://")
                                .ok_or_else(|| format!("Not a local file URI: {uri_str}"))
                                .and_then(|encoded| {
                                    urlencoding::decode(encoded)
                                        .map(|d| PathBuf::from(d.as_ref()))
                                        .map_err(|e| format!("Could not decode URI path: {e}"))
                                })
                        } else {
                            Err("No directory selected".to_string())
                        }
                    }
                    .await;
                    cosmic::Action::App(Message::ConvertOutputDirPicked(result))
                });
            }

            Message::ConvertOutputDirPicked(result) => match result {
                Ok(path) => self.convert_out_dir = path,
                Err(e) => tracing::warn!("convert: output directory picker failed: {e}"),
            },

            Message::ConvertFormatSelected(index) => self.convert_format_index = index,

            Message::ConvertRateSelected(index) => self.convert_rate_index = index,

            Message::ConvertStart => {
                let mut tasks = Vec::new();
                for job in &mut self.convert_jobs {
                    if job.state == JobState::Queued {
                        job.state = JobState::Running;
                        let job_clone = job.clone();
                        let semaphore = Arc::clone(&self.convert_semaphore);
                        tasks.push(cosmic::task::future(async move {
                            let (id, state) = run_job(job_clone, semaphore).await;
                            cosmic::Action::App(Message::ConvertJobFinished(id, state))
                        }));
                    }
                }
                if !tasks.is_empty() {
                    return Task::batch(tasks);
                }
            }

            Message::ConvertJobFinished(id, state) => {
                if let Some(job) = self.convert_jobs.iter_mut().find(|j| j.id == id) {
                    job.state = state;
                }
            }

            Message::ConvertCancelJob(id) => {
                if let Some(job) = self.convert_jobs.iter().find(|j| j.id == id) {
                    job.request_cancel();
                }
            }

            Message::ConvertClearFinished => {
                self.convert_jobs
                    .retain(|j| matches!(j.state, JobState::Queued | JobState::Running));
            }

            Message::ConvertTick => {}
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
        self.selected_playlist = None;
        self.selected_genre = None;

        // Collapse expanded now-playing view when navigating
        if self.expand_progress > 0.0 || self.expand_target.is_some() {
            self.lyrics_overlay_active = false;
            self.expand_target = Some(0.0);
            self.expand_anim_start = Some(std::time::Instant::now());
            self.expand_anim_from = self.expand_progress;
        }

        // Lazy-load data for Playlists and Genres pages
        let page = self.nav.active_data::<Page>().cloned();
        let page_task = match page {
            Some(Page::Playlists) => self.load_playlists(),
            Some(Page::Genres) => self.load_genres(),
            Some(Page::Podcasts) => self.load_podcasts(),
            Some(Page::Radio) => self.load_radio_stations(),
            _ => Task::none(),
        };

        let title_task = self.update_title();
        Task::batch([title_task, page_task])
    }
}

/// Identity key for the MPD status-poll subscription. Hashing only the
/// fixed string (not the client) preserves the old fixed-string-id
/// behavior: a single persistent subscription while active.
struct MpdPollKey(mpd_client::Client);

impl Hash for MpdPollKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "mpd-status-poll".hash(state);
    }
}

/// MPD: poll status from the server every 300ms.
///
/// This replaces the generic tick for MPD playback — we get
/// position/duration/state/volume from the real MPD status.
fn mpd_status_stream(key: &MpdPollKey) -> impl Stream<Item = Message> + use<> {
    let client = key.0.clone();
    cosmic::iced::stream::channel(
        1,
        |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            let mut interval = tokio::time::interval(Duration::from_millis(300));
            loop {
                interval.tick().await;
                match client.command(mpd_client::commands::Status).await {
                    Ok(status) => {
                        let state = match status.state {
                            mpd_client::responses::PlayState::Playing => PlaybackState::Playing,
                            mpd_client::responses::PlayState::Paused => PlaybackState::Paused,
                            mpd_client::responses::PlayState::Stopped => PlaybackState::Stopped,
                        };
                        _ = emitter
                            .send(Message::MpdStatusUpdate {
                                position: status.elapsed.unwrap_or(Duration::ZERO),
                                duration: status.duration.unwrap_or(Duration::ZERO),
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
        },
    )
}

/// Local/Subsonic: simple tick for UI updates (no captured state).
fn playback_tick_stream() -> impl Stream<Item = Message> {
    cosmic::iced::stream::channel(
        1,
        |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                _ = emitter.send(Message::PlaybackTick).await;
            }
        },
    )
}

/// Convert-queue progress ticker: while jobs are running, periodically
/// triggers a re-render so the UI picks up the latest atomic progress
/// values written by the background job tasks.
fn convert_tick_stream() -> impl Stream<Item = Message> {
    cosmic::iced::stream::channel(
        1,
        |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                _ = emitter.send(Message::ConvertTick).await;
            }
        },
    )
}

/// Identity key for an MPD idle-event subscription. Hashing only `idx`
/// (not the provider) preserves the old `("mpd-idle", idx)` identity
/// semantics.
struct MpdIdleKey {
    idx: usize,
    provider: Arc<MpdProvider>,
}

impl Hash for MpdIdleKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
    }
}

/// MPD idle event subscription — one per configured MPD provider.
///
/// Each subscription opens TWO separate TCP connections to the MPD server:
/// 1. Idle connection — stays in idle mode, streams events via ConnectionEvents
/// 2. Command connection — stored in MpdProvider for browse/search/playback
///
/// This avoids protocol conflicts between idle mode and command execution.
fn mpd_idle_stream(key: &MpdIdleKey) -> impl Stream<Item = Message> + use<> {
    let provider = Arc::clone(&key.provider);
    cosmic::iced::stream::channel(
        4,
        move |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
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
                    tracing::error!("MPD provider '{pid}' command connection failed: {e}");
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
        },
    )
}

/// Filesystem watcher subscription — only when the Local provider is active.
///
/// Uses notify::RecommendedWatcher to watch music_dirs recursively.
/// Debounces events with a 2-second quiet timer before emitting
/// Message::FilesChanged with the collected paths.
#[allow(clippy::ptr_arg)] // must match `fn(&D) -> S` where D = Vec<PathBuf> (Subscription::run_with)
fn fs_watcher_stream(music_dirs: &Vec<PathBuf>) -> impl Stream<Item = Message> + use<> {
    let music_dirs = music_dirs.clone();
    cosmic::iced::stream::channel(
        4,
        move |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            use notify::{RecursiveMode, Watcher};

            let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(256);

            // Create watcher that sends changed paths through the channel.
            // IMPORTANT: Only react to content changes (Create/Modify/Remove),
            // NOT Access events. Reading files during scanning triggers Access
            // events which would create an infinite scan loop.
            let _watcher = {
                let tx = tx.clone();
                let mut watcher = match notify::RecommendedWatcher::new(
                    move |result: Result<notify::Event, notify::Error>| {
                        if let Ok(event) = result {
                            use notify::EventKind;
                            match event.kind {
                                EventKind::Create(_)
                                | EventKind::Modify(_)
                                | EventKind::Remove(_) => {
                                    for path in event.paths {
                                        let _ = tx.blocking_send(path);
                                    }
                                }
                                _ => {}
                            }
                        }
                    },
                    notify::Config::default(),
                ) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!("Failed to create filesystem watcher: {e}");
                        // Keep the future alive so the subscription ID is stable.
                        std::future::pending::<()>().await;
                        return;
                    }
                };

                for dir in &music_dirs {
                    if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                        tracing::warn!("Failed to watch directory {}: {e}", dir.display());
                    }
                }

                watcher // keep alive
            };

            // Debounce loop: collect paths, wait for 2s of quiet, then emit.
            let debounce_duration = Duration::from_secs(2);
            loop {
                // Wait for the first event.
                let first = match rx.recv().await {
                    Some(path) => path,
                    None => break, // channel closed
                };

                let mut changed_paths = vec![first];

                // Collect more events until 2 seconds of silence.
                loop {
                    match tokio::time::timeout(debounce_duration, rx.recv()).await {
                        Ok(Some(path)) => {
                            changed_paths.push(path);
                        }
                        Ok(None) => break, // channel closed
                        Err(_) => break,   // timeout — debounce complete
                    }
                }

                // Deduplicate paths.
                changed_paths.sort();
                changed_paths.dedup();

                tracing::debug!(
                    "Filesystem watcher: {} changed paths after debounce",
                    changed_paths.len()
                );

                _ = emitter.send(Message::FilesChanged(changed_paths)).await;
            }
        },
    )
}

/// Identity key for the projectM visualizer render subscription. Hashes to
/// a constant so there's only ever one active instance regardless of the
/// captured Arc contents (mirrors `MpdPollKey`/`MpdIdleKey`).
#[cfg(feature = "visualizer")]
struct VizRenderKey {
    pcm: Arc<Mutex<crate::views::now_playing::visualizer::PcmBuffer>>,
    cmd_rx: Arc<
        Mutex<Option<std::sync::mpsc::Receiver<crate::views::now_playing::visualizer::VizCommand>>>,
    >,
    frame_buf: Arc<Mutex<crate::views::now_playing::viz_shader::VizFrameBuffer>>,
    current_preset: Arc<Mutex<Option<String>>>,
}

#[cfg(feature = "visualizer")]
impl Hash for VizRenderKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "projectm-render".hash(state);
    }
}

/// Runs the projectM render loop on a dedicated OS thread and relays
/// "frame ready" notifications back to the UI through an iced subscription
/// stream. See the call site for why the render loop needs its own thread
/// (thread-local EGL/GL context).
#[cfg(feature = "visualizer")]
fn projectm_render_stream(key: &VizRenderKey) -> impl Stream<Item = Message> + use<> {
    let pcm = Arc::clone(&key.pcm);
    let cmd_rx_slot = Arc::clone(&key.cmd_rx);
    let frame_buf = Arc::clone(&key.frame_buf);
    let current_preset = Arc::clone(&key.current_preset);
    cosmic::iced::stream::channel(
        2,
        move |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            // Use a one-shot channel to know when the render thread
            // has produced a new frame so we can notify the UI.
            let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<()>(2);

            // Spawn a dedicated OS thread for the GL render loop.
            // The EGL context created inside `ProjectMRenderer::new`
            // stays current for the lifetime of this thread.
            std::thread::Builder::new()
                .name("projectm-render".into())
                .spawn(move || {
                    let preset_dir = dirs::data_dir().map(|d| d.join("projectm").join("presets"));
                    let mut renderer =
                        match crate::views::now_playing::visualizer::ProjectMRenderer::new(
                            preset_dir,
                        ) {
                            Ok(r) => r,
                            Err(e) => {
                                tracing::error!("Failed to create projectM renderer: {e}");
                                return;
                            }
                        };

                    // Check out the command receiver for this thread's
                    // lifetime; handed back below so a future activation
                    // (visualizer toggled off then on again) can check it
                    // out in turn — the channel itself is only ever
                    // created once, in `AppModel::init`.
                    let mut cmd_rx = cmd_rx_slot.lock().ok().and_then(|mut slot| slot.take());

                    loop {
                        // ~30 fps
                        std::thread::sleep(Duration::from_millis(33));

                        // Drain every pending command before rendering
                        // this frame.
                        if let Some(rx) = cmd_rx.as_ref() {
                            use crate::views::now_playing::visualizer::VizCommand;
                            while let Ok(cmd) = rx.try_recv() {
                                match cmd {
                                    VizCommand::NextPreset => {
                                        renderer.next_preset();
                                        // The playlist API exposes no way to
                                        // read back which preset it landed
                                        // on — clear the tracked name.
                                        if let Ok(mut name) = current_preset.lock() {
                                            *name = None;
                                        }
                                    }
                                    VizCommand::LoadPreset(path) => {
                                        renderer.load_preset(&path);
                                        let display_name = path
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().into_owned());
                                        if let Ok(mut name) = current_preset.lock() {
                                            *name = display_name;
                                        }
                                    }
                                    VizCommand::SetLocked(locked) => renderer.set_locked(locked),
                                    VizCommand::SetBeatSensitivity(sensitivity) => {
                                        renderer.set_beat_sensitivity(sensitivity);
                                    }
                                }
                            }
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

                    // Hand the receiver back so a future render-thread
                    // activation can check it out in turn.
                    if let Ok(mut slot) = cmd_rx_slot.lock() {
                        *slot = cmd_rx.take();
                    }
                })
                .expect("failed to spawn projectm-render thread");

            // Relay frame-ready signals from the render thread to iced.
            while frame_rx.recv().await.is_some() {
                _ = emitter.send(Message::VisualizerFrameReady).await;
            }
        },
    )
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

    /// Get the active MPD provider (if the active provider is MPD).
    ///
    /// Used by Tasks 111-112 to wire shuffle/repeat toggles to MPD.
    fn active_mpd_provider(&self) -> Option<Arc<MpdProvider>> {
        let active_id = self.registry.active_id();
        self.mpd_providers
            .iter()
            .find(|p| p.id() == active_id)
            .cloned()
    }

    /// Load playlists from the active provider asynchronously.
    ///
    /// Used by Task 119 to refresh playlists after CRUD operations.
    fn load_playlists(&self) -> Task<cosmic::Action<Message>> {
        if let Some(provider) = self.registry.active_shared() {
            cosmic::task::future(async move {
                let playlists = tokio::task::spawn_blocking(move || {
                    provider.list_playlists().unwrap_or_else(|e| {
                        tracing::warn!("list_playlists failed: {e}");
                        Vec::new()
                    })
                })
                .await
                .unwrap_or_default();
                cosmic::Action::App(Message::PlaylistsLoaded(playlists))
            })
        } else {
            Task::none()
        }
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

    /// Bumps the library reload generation counter, invalidating any
    /// in-flight async result tagged with an older generation. Call this
    /// once at the start of every new reload/scan that should supersede
    /// earlier work.
    fn begin_reload_generation(&mut self) -> u64 {
        self.reload_generation += 1;
        self.reload_generation
    }

    /// True when an async library result tagged with `generation`/
    /// `provider_id` is stale — superseded by a later reload/scan or a
    /// provider switch — and must be ignored without mutating state.
    fn is_stale_reload(&self, generation: u64, provider_id: &str) -> bool {
        reload_result_is_stale(
            self.reload_generation,
            self.registry.active_id(),
            generation,
            provider_id,
        )
    }

    fn reload_library(&mut self) -> Task<cosmic::Action<Message>> {
        let provider = match self.registry.active_shared() {
            Some(p) => p,
            None => return Task::none(),
        };
        let provider_type = provider.provider_type();
        let generation = self.begin_reload_generation();

        match provider_type {
            crate::provider::ProviderType::Local => self.reload_library_local(provider, generation),
            crate::provider::ProviderType::Mpd | crate::provider::ProviderType::Subsonic => {
                // Clear existing data before incremental loading begins.
                self.all_tracks.clear();
                self.all_albums.clear();
                self.all_artists.clear();
                self.cover_images.clear();
                self.artist_avatars.clear();
                self.library_scanning = true;
                self.reload_library_incremental(provider, provider_type, generation)
            }
        }
    }

    /// Single-shot library reload for the local provider (reads from local DB).
    fn reload_library_local(
        &self,
        provider: Arc<dyn MusicProvider + Send + Sync>,
        generation: u64,
    ) -> Task<cosmic::Action<Message>> {
        let provider_id = provider.id().to_string();
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

            // Extract cover art in parallel
            let cover_tasks: Vec<_> = albums
                .iter()
                .filter_map(|album| {
                    let key = crate::library::CoverArt::album_key(&album.artist, &album.name);
                    album.tracks.first().map(|track| (key, track.path.clone()))
                })
                .map(|(key, path)| {
                    tokio::task::spawn_blocking(move || {
                        crate::library::CoverArt::get_cover_art(&path).map(|bytes| (key, bytes))
                    })
                })
                .collect();

            let mut cover_images = HashMap::new();
            let mut cover_art_bytes = HashMap::new();
            for task in cover_tasks {
                if let Ok(Some((key, bytes))) = task.await {
                    let handle = widget::icon::from_raster_bytes(bytes.clone());
                    cover_images.insert(key.clone(), handle);
                    cover_art_bytes.insert(key, bytes);
                }
            }

            // Generate artist avatars (fast, keep sequential)
            let mut artist_avatars = HashMap::new();
            for artist in &artists {
                let bytes = crate::library::CoverArt::generate_artist_avatar(&artist.name, 64);
                let handle = widget::icon::from_raster_bytes(bytes);
                artist_avatars.insert(artist.name.clone(), handle);
            }

            cosmic::Action::App(Message::LibraryLoaded {
                generation,
                provider_id,
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
        generation: u64,
    ) -> Task<cosmic::Action<Message>> {
        // Downcast to concrete provider types for paged access.
        // We clone the Arc'd provider references from self.
        let mpd_providers = self.mpd_providers.clone();
        let subsonic_providers = self.subsonic_providers.clone();
        let active_id = self.registry.active_id().to_string();

        let stream = cosmic::iced::stream::channel(
            8,
            move |mut emitter: cosmic::iced::futures::channel::mpsc::Sender<
                cosmic::Action<Message>,
            >| async move {
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
                                    .send(cosmic::Action::App(Message::LibraryLoadComplete {
                                        generation,
                                        provider_id: active_id.clone(),
                                    }))
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

                            // Fetch cover art for this batch in parallel.
                            let prov = Arc::clone(&provider);
                            let cover_tasks: Vec<_> = albums
                                .iter()
                                .map(|album| {
                                    let key = crate::library::CoverArt::album_key(
                                        &album.artist,
                                        &album.name,
                                    );
                                    let prov2 = Arc::clone(&prov);
                                    let hint = album.cover_hint();
                                    tokio::task::spawn_blocking(move || {
                                        let result = prov2.get_cover_art(&hint);
                                        (key, result)
                                    })
                                })
                                .collect();

                            let mut cover_images = HashMap::new();
                            let mut cover_art_bytes = HashMap::new();
                            for task in cover_tasks {
                                if let Ok((key, Ok(Some(bytes)))) = task.await {
                                    let handle = widget::icon::from_raster_bytes(bytes.clone());
                                    cover_images.insert(key.clone(), handle);
                                    cover_art_bytes.insert(key, bytes);
                                }
                            }

                            _ = emitter
                                .send(cosmic::Action::App(Message::LibraryBatch {
                                    generation,
                                    provider_id: active_id.clone(),
                                    albums,
                                    cover_images,
                                    cover_art_bytes,
                                }))
                                .await;
                        }
                    }

                    crate::provider::ProviderType::Subsonic => {
                        // Find the matching SubsonicProvider by id.
                        let subsonic = match subsonic_providers.iter().find(|p| p.id() == active_id)
                        {
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

                            // Fetch cover art for this batch in parallel.
                            let prov = Arc::clone(&provider);
                            let cover_tasks: Vec<_> = albums
                                .iter()
                                .map(|album| {
                                    let key = crate::library::CoverArt::album_key(
                                        &album.artist,
                                        &album.name,
                                    );
                                    let prov2 = Arc::clone(&prov);
                                    let hint = album.cover_hint();
                                    tokio::task::spawn_blocking(move || {
                                        let result = prov2.get_cover_art(&hint);
                                        (key, result)
                                    })
                                })
                                .collect();

                            let mut cover_images = HashMap::new();
                            let mut cover_art_bytes = HashMap::new();
                            for task in cover_tasks {
                                if let Ok((key, Ok(Some(bytes)))) = task.await {
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
                                    generation,
                                    provider_id: active_id.clone(),
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
                    .send(cosmic::Action::App(Message::LibraryLoadComplete {
                        generation,
                        provider_id: active_id.clone(),
                    }))
                    .await;
            },
        );

        cosmic::task::stream(stream)
    }

    /// Persist the current config via cosmic-config.
    fn save_config(&self) {
        if let Some(ref context) = self.config_context
            && let Err(e) = self.config.write_entry(context)
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
        self.registry
            .remove_by_type(crate::provider::ProviderType::Mpd);
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

        // Re-create from config (clone first: iterating `&self.config...`
        // while calling `self.push_toast()` inside the loop would otherwise
        // hold an immutable borrow of `self` across a mutable one).
        let rt_handle = tokio::runtime::Handle::current();
        let mut toast_tasks = Vec::new();
        let subsonic_servers = self.config.subsonic_servers.clone();
        for entry in &subsonic_servers {
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
                    toast_tasks.push(self.push_toast(widget::toaster::Toast::new(fl!(
                        "toast-provider-connect-failed",
                        provider = entry.name.clone(),
                        reason = e.to_string()
                    ))));
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
        let reload_task = if self
            .registry
            .active()
            .is_some_and(|p| p.provider_type() == crate::provider::ProviderType::Subsonic)
        {
            self.reload_library()
        } else {
            Task::none()
        };
        toast_tasks.push(reload_task);

        Task::batch(toast_tasks)
    }

    /// Re-initialize the Local provider with the current `config.music_dirs`.
    ///
    /// Removes the old Local provider from the registry, creates a new one
    /// with the updated scan directories, and rebuilds the provider list.
    fn reinit_local_provider(&mut self) {
        self.registry
            .remove_by_type(crate::provider::ProviderType::Local);

        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lyra")
            .join("library.db");

        if let Ok(db) = LibraryDb::open(&db_path) {
            let local = LocalProvider::new(db, self.config.music_dirs.clone());
            self.registry.register(Arc::new(local));
        } else {
            tracing::error!("Failed to open library database for reinit");
        }

        self.rebuild_provider_list();
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
                    tracing::debug!("Reconnecting PCM buffer to new player instance");
                    p.set_pcm_buffer(Arc::clone(buf));
                } else {
                    tracing::warn!("PCM buffer is None - visualizer will not receive audio");
                }

                // Apply saved EQ state to the new player's DSP.
                let eq = p.eq_controller();
                eq.set_enabled(self.config.equalizer_enabled);
                if self.config.equalizer_bands.len() == 10 {
                    let mut gains = [0.0_f32; 10];
                    gains.copy_from_slice(&self.config.equalizer_bands);
                    eq.set_all(&gains);
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
                    let bytes = crate::library::CoverArt::generate_artist_avatar(&album.artist, 64);
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
        // Switching away from a podcast episode (to a different episode or
        // any other track) — persist its last known position first.
        let podcast_save_task = match self.current_podcast_episode_id.take() {
            Some(episode_id) => {
                let position_ms = self.playback_position.as_millis() as i64;
                self.save_podcast_position(episode_id, position_ms, false)
            }
            None => Task::none(),
        };

        if let Some(ref mut player) = self.player {
            let current = tracks.get(start_index).cloned();
            player.set_queue(tracks);
            if player.play_index(start_index).is_ok() {
                self.current_track = current;
                self.playback_position = Duration::ZERO;
                self.lyrics_text = None;
                self.scrobble_now_playing_sent = false;
                self.scrobble_sent = false;
                #[cfg(feature = "visualizer")]
                {
                    self.viz_metadata_opacity = 1.0;
                }
                let mpd_task = self.dispatch_mpd_after_play();
                let blur_task = self.maybe_update_blurred_cover();
                return Task::batch([podcast_save_task, mpd_task, blur_task]);
            }
        }
        podcast_save_task
    }

    fn sort_tracks(&mut self, field: songs::SortField) {
        match field {
            songs::SortField::Title => self.all_tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            songs::SortField::Artist => self.all_tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            songs::SortField::Album => self.all_tracks.sort_by(|a, b| a.album.cmp(&b.album)),
            songs::SortField::Duration => self.all_tracks.sort_by_key(|a| a.duration),
        }
    }

    /// Recomputes the search-filtered library caches from `library_search`.
    ///
    /// Called whenever the query changes and whenever the underlying library
    /// data is reloaded or re-sorted, so the filtered caches — and the index
    /// maps that translate a filtered position back to its index in the
    /// corresponding unfiltered vector — never go stale. When the query is
    /// empty the caches are cleared; `view()` then reads directly from the
    /// unfiltered vectors.
    fn refresh_search_filter(&mut self) {
        let query = self.library_search.trim().to_lowercase();

        self.filtered_albums.clear();
        self.filtered_album_map.clear();
        self.filtered_artists.clear();
        self.filtered_artist_map.clear();
        self.filtered_tracks.clear();
        self.filtered_track_map.clear();
        self.filtered_playlists.clear();
        self.filtered_playlist_map.clear();
        self.filtered_genres.clear();
        self.filtered_genre_map.clear();

        if query.is_empty() {
            return;
        }

        for (i, album) in self.all_albums.iter().enumerate() {
            if album.name.to_lowercase().contains(&query)
                || album.artist.to_lowercase().contains(&query)
            {
                self.filtered_albums.push(album.clone());
                self.filtered_album_map.push(i);
            }
        }

        for (i, artist) in self.all_artists.iter().enumerate() {
            if artist.name.to_lowercase().contains(&query) {
                self.filtered_artists.push(artist.clone());
                self.filtered_artist_map.push(i);
            }
        }

        for (i, track) in self.all_tracks.iter().enumerate() {
            if track.title.to_lowercase().contains(&query)
                || track.artist.to_lowercase().contains(&query)
                || track.album.to_lowercase().contains(&query)
            {
                self.filtered_tracks.push(track.clone());
                self.filtered_track_map.push(i);
            }
        }

        for (i, playlist) in self.playlists.iter().enumerate() {
            if playlist.name.to_lowercase().contains(&query) {
                self.filtered_playlists.push(playlist.clone());
                self.filtered_playlist_map.push(i);
            }
        }

        for (i, genre) in self.all_genres.iter().enumerate() {
            if genre.to_lowercase().contains(&query) {
                self.filtered_genres.push(genre.clone());
                self.filtered_genre_map.push(i);
            }
        }
    }

    /// Pushes a toast notification, mapping its auto-dismiss timer task into
    /// the `cosmic::Action`-wrapped message type `update()` returns.
    fn push_toast(
        &mut self,
        toast: widget::toaster::Toast<Message>,
    ) -> Task<cosmic::Action<Message>> {
        self.toasts.push(toast).map(cosmic::Action::App)
    }

    /// Exit visualizer fullscreen if active: restore the COSMIC header bar and
    /// nav sidebar. No-op when not in fullscreen. Called whenever the expanded
    /// now-playing view is left or the visualizer is turned off.
    #[cfg(feature = "visualizer")]
    fn exit_viz_fullscreen(&mut self) {
        if self.viz_fullscreen {
            self.viz_fullscreen = false;
            self.core.window.show_headerbar = true;
            self.core.nav_bar_set_toggled(self.viz_prev_nav_active);
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
                // No track — clear everything.
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

        // Already computed and cached for this album — nothing to do.
        if self.blurred_cover_key.as_ref() == Some(&key) {
            return Task::none();
        }

        // Look up raw bytes. If they are not available yet (still loading),
        // reset the key so we retry when bytes arrive, but keep the current
        // blurred_cover showing (previous track's blur) rather than blanking
        // the background immediately. The blur will update as soon as bytes
        // are ready and maybe_update_blurred_cover is called again.
        let bytes = match self.cover_art_bytes.get(&key) {
            Some(b) => b.clone(),
            None => {
                self.blurred_cover_key = None; // ensure retry on next bytes-ready event
                // Do NOT clear blurred_cover — keep the old blur visible.
                return Task::none();
            }
        };

        // Bytes are available — start the async blur computation.
        // Clear the key now so a concurrent track change will not skip
        // the next blur computation (BlurReady carries the key and will
        // only apply if it still matches the current track).
        self.blurred_cover_key = None;

        let key_clone = key.clone();
        cosmic::task::future(async move {
            // Compute blur in a blocking task to avoid stalling the async runtime.
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
                // Blur computation failed — no-op; keep old blur or black base.
                cosmic::Action::None
            }
        })
    }

    /// Load genres from the active provider and dispatch a GenresLoaded message.
    fn load_genres(&self) -> Task<cosmic::Action<Message>> {
        if let Some(provider) = self.registry.active_shared() {
            cosmic::task::future(async move {
                let genres = tokio::task::spawn_blocking(move || {
                    provider.list_genres().unwrap_or_else(|e| {
                        tracing::debug!("list_genres: {e}");
                        Vec::new()
                    })
                })
                .await
                .unwrap_or_default();
                cosmic::Action::App(Message::GenresLoaded(genres))
            })
        } else {
            Task::none()
        }
    }

    /// Load subscribed podcasts from the online store.
    fn load_podcasts(&self) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let podcasts = tokio::task::spawn_blocking(|| {
                open_online_store()
                    .and_then(|store| store.list_podcasts())
                    .unwrap_or_else(|e| {
                        tracing::warn!("list_podcasts failed: {e}");
                        Vec::new()
                    })
            })
            .await
            .unwrap_or_default();
            cosmic::Action::App(Message::PodcastsLoaded(podcasts))
        })
    }

    /// Load a podcast's episodes from the online store.
    fn load_podcast_episodes(&self, podcast_id: i64) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let episodes = tokio::task::spawn_blocking(move || {
                open_online_store()
                    .and_then(|store| store.list_episodes(podcast_id))
                    .unwrap_or_else(|e| {
                        tracing::warn!("list_episodes failed: {e}");
                        Vec::new()
                    })
            })
            .await
            .unwrap_or_default();
            cosmic::Action::App(Message::PodcastEpisodesLoaded(podcast_id, episodes))
        })
    }

    /// Load saved radio stations from the online store.
    fn load_radio_stations(&self) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let stations = tokio::task::spawn_blocking(|| {
                open_online_store()
                    .and_then(|store| store.list_radio_stations())
                    .unwrap_or_else(|e| {
                        tracing::warn!("list_radio_stations failed: {e}");
                        Vec::new()
                    })
            })
            .await
            .unwrap_or_default();
            cosmic::Action::App(Message::RadioStationsLoaded(stations))
        })
    }

    /// Fetch each not-yet-cached icon URL and dispatch `OnlineIconLoaded` for
    /// it, used for podcast artwork and radio station favicons alike.
    fn load_online_icons(&self, urls: Vec<String>) -> Task<cosmic::Action<Message>> {
        let tasks: Vec<_> = urls
            .into_iter()
            .filter(|url| !url.is_empty() && !self.online_icons.contains_key(url))
            .map(|url| {
                let fetch_url = url.clone();
                cosmic::task::future(async move {
                    let bytes = tokio::task::spawn_blocking(move || {
                        reqwest::blocking::Client::new()
                            .get(&fetch_url)
                            .send()
                            .ok()
                            .and_then(|r| r.bytes().ok())
                            .map(|b| b.to_vec())
                            .unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default();
                    cosmic::Action::App(Message::OnlineIconLoaded(url, bytes))
                })
            })
            .collect();
        Task::batch(tasks)
    }

    /// Persist podcast episode playback progress. Fire-and-forget: a
    /// transient DB error is logged, not surfaced as a toast, since it
    /// shouldn't interrupt playback.
    fn save_podcast_position(
        &self,
        episode_id: i64,
        position_ms: i64,
        played: bool,
    ) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let result = tokio::task::spawn_blocking(move || {
                open_online_store()
                    .and_then(|store| store.save_episode_position(episode_id, position_ms, played))
            })
            .await;
            if let Ok(Err(e)) = result {
                tracing::warn!("Failed to save podcast position: {e}");
            }
            // No-op message — this write is fire-and-forget, matching
            // `dispatch_mpd`'s convention for tasks nothing depends on.
            cosmic::Action::App(Message::PlaybackTick)
        })
    }
}

/// Path to the shared library database (also used by `OnlineStore` for
/// podcasts/radio — same file, same schema, opened via its own connection).
fn online_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lyra")
        .join("library.db")
}

/// Open the online store at the shared library database path.
fn open_online_store() -> Result<OnlineStore, String> {
    OnlineStore::open(&online_db_path())
}

/// Current Unix time in whole seconds.
fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Re-fetch a podcast's feed and update its metadata/episodes in the
/// online store, dispatching `PodcastRefreshed` with the outcome.
fn refresh_podcast_task(id: i64, feed_url: String) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(move || {
            let client = reqwest::blocking::Client::new();
            let (meta, episodes) = podcast::fetch_feed(&client, &feed_url)?;
            let store = open_online_store()?;
            store.touch_podcast_refresh(id, &meta, now_epoch())?;
            store.upsert_episodes(id, &episodes)?;
            Ok(())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        cosmic::Action::App(Message::PodcastRefreshed(id, result))
    })
}

/// Resolve a station URL (following a `.pls`/`.m3u`/`.m3u8` playlist if
/// needed) and dispatch `RadioStreamResolved` with the outcome.
fn resolve_and_play_radio(name: String, url: String) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(move || {
            let client = reqwest::blocking::Client::new();
            radio::resolve_stream_url(&client, &url)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        cosmic::Action::App(Message::RadioStreamResolved { name, result })
    })
}

/// Navigation pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Albums,
    Artists,
    Songs,
    Playlists,
    Genres,
    Podcasts,
    Radio,
    Convert,
}

/// Context drawer pages.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
    Equalizer,
    Lyrics,
    Providers,
    Settings,
}

/// Menu bar actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    About,
    Equalizer,
    Providers,
    Settings,
    ScanLibrary,
    AddMusicDir,
    Search,
    Quit,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Equalizer => Message::ToggleContextPage(ContextPage::Equalizer),
            MenuAction::Providers => Message::ToggleContextPage(ContextPage::Providers),
            MenuAction::Settings => Message::ToggleContextPage(ContextPage::Settings),
            MenuAction::ScanLibrary => Message::ScanLibrary,
            MenuAction::AddMusicDir => Message::AddMusicDir,
            MenuAction::Search => Message::ToggleLibrarySearch,
            MenuAction::Quit => Message::Quit,
        }
    }
}

/// Builds the map of global keyboard shortcuts to menu actions.
///
/// Consumed both by the menu bar (to display the shortcut label next to
/// each item) and by the keyboard subscription in `subscription()` (to
/// actually trigger the action when the shortcut is pressed).
fn key_binds() -> HashMap<menu::KeyBind, MenuAction> {
    let mut key_binds = HashMap::new();
    key_binds.insert(
        menu::KeyBind {
            modifiers: vec![menu::key_bind::Modifier::Ctrl],
            key: cosmic::iced::keyboard::Key::Character("f".into()),
        },
        MenuAction::Search,
    );
    key_binds
}

/// Translates a position in a search-filtered list back to its index in
/// the corresponding unfiltered library vector. Passthrough when `map` is
/// `None` (search inactive or the query is empty).
fn unfilter_index(map: Option<&[usize]>, i: usize) -> usize {
    map.map_or(i, |m| m[i])
}

/// Pure staleness check for an async library reload result.
///
/// `true` when the result's `(generation, provider_id)` no longer matches
/// the currently active reload — i.e. it was superseded by a later
/// reload/scan or a provider switch — and must be discarded without
/// mutating library data or `library_scanning`.
fn reload_result_is_stale(
    current_generation: u64,
    current_provider_id: &str,
    result_generation: u64,
    result_provider_id: &str,
) -> bool {
    result_generation != current_generation || result_provider_id != current_provider_id
}

#[cfg(test)]
mod reload_generation_tests {
    use super::reload_result_is_stale;

    #[test]
    fn matching_generation_and_provider_is_not_stale() {
        assert!(!reload_result_is_stale(3, "mpd-home", 3, "mpd-home"));
    }

    #[test]
    fn result_from_a_superseded_reload_is_stale() {
        // A second reload bumped the generation before this result arrived.
        assert!(reload_result_is_stale(3, "mpd-home", 2, "mpd-home"));
    }

    #[test]
    fn result_from_a_different_provider_is_stale_even_at_current_generation() {
        // The generation counter alone can't catch a switch back to the
        // same numeric generation on a different provider, so identity
        // must be checked independently.
        assert!(reload_result_is_stale(3, "mpd-home", 3, "subsonic-server"));
    }
}
