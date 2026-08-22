// SPDX-License-Identifier: GPL-3.0

use super::tasks::resolve_mpris_art_task;
use super::{AppModel, HTTP_CLIENT, Message, open_online_store, reload_result_is_stale};
use crate::fl;
use crate::library::{Album, Artist, LibraryDb, LibraryScanner, Track};
use crate::player::mpd_backend::MpdBackend;
use crate::player::{ActiveBackend, PlaybackState, Player};
use crate::provider::MusicProvider;
use crate::provider::local::LocalProvider;
use crate::provider::mpd::{MpdConfig, MpdProvider};
use crate::provider::subsonic::{SubsonicConfig, SubsonicProvider};
use crate::views::{providers, songs};
use cosmic::cosmic_config::CosmicConfigEntry;
use cosmic::prelude::*;
use cosmic::widget;
use futures_util::SinkExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

impl AppModel {
    /// Rebuild `provider_list` and `active_provider_index` from the registry.
    ///
    /// Call after any change to the set of registered providers (init,
    /// reinit_mpd_providers, reinit_subsonic_providers).
    pub(super) fn rebuild_provider_list(&mut self) {
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
    pub(super) fn mpd_client(&self) -> Option<mpd_client::Client> {
        let player = self.player.as_ref()?;
        if player.active_backend_type() != ActiveBackend::Mpd {
            return None;
        }
        Some(player.mpd_backend_ref()?.client())
    }

    /// Get the active MPD provider (if the active provider is MPD).
    ///
    /// Used by Tasks 111-112 to wire shuffle/repeat toggles to MPD.
    pub(super) fn active_mpd_provider(&self) -> Option<Arc<MpdProvider>> {
        let active_id = self.registry.active_id();
        self.mpd_providers
            .iter()
            .find(|p| p.id() == active_id)
            .cloned()
    }

    /// Load playlists from the active provider asynchronously.
    ///
    /// Used by Task 119 to refresh playlists after CRUD operations.
    pub(super) fn load_playlists(&self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn dispatch_mpd<F>(&self, future: F) -> Task<cosmic::Action<Message>>
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
    pub(super) fn dispatch_mpd_play(&self, uri: String) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn dispatch_mpd_after_play(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(ref mut player) = self.player
            && let Some(mpd) = player.mpd_backend_mut()
            && let Some(uri) = mpd.take_play_uri()
        {
            return self.dispatch_mpd_play(uri);
        }
        Task::none()
    }

    pub(super) fn update_title(&mut self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn begin_reload_generation(&mut self) -> u64 {
        self.reload_generation += 1;
        self.reload_generation
    }

    /// True when an async library result tagged with `generation`/
    /// `provider_id` is stale — superseded by a later reload/scan or a
    /// provider switch — and must be ignored without mutating state.
    pub(super) fn is_stale_reload(&self, generation: u64, provider_id: &str) -> bool {
        reload_result_is_stale(
            self.reload_generation,
            self.registry.active_id(),
            generation,
            provider_id,
        )
    }

    pub(super) fn reload_library(&mut self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn reload_library_local(
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
    pub(super) fn reload_library_incremental(
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
    pub(super) fn save_config(&self) {
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
    pub(super) fn reinit_mpd_providers(&mut self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn reinit_subsonic_providers(&mut self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn reinit_local_provider(&mut self) {
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
    pub(super) fn make_mpd_backend(&self) -> Option<MpdBackend> {
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
    pub(super) fn recreate_player(&mut self) {
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

                // Apply the saved master volume; a fresh `Player` hardcodes
                // 0.8, so without this a provider switch would reset the
                // level instead of preserving it.
                if let Err(e) = p.set_volume(self.config.volume) {
                    tracing::warn!("Failed to apply saved volume: {e}");
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
    pub(super) fn handle_scrobble(&mut self, track: Track) {
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

    /// Incrementally merge new albums into `all_artists`, splitting each
    /// album's primary-artist tag into individual collaborators when
    /// `split_artist_tags` is enabled — one album can then contribute to
    /// several `Artist` entries so browsing by any collaborator finds it.
    /// Albums keep a single primary attribution (`album.artist`, set by
    /// the provider) for the Albums view; only the artist index widens.
    /// Only processes the `new_albums` slice (the batch that just arrived),
    /// appending to existing artists or creating new ones. Avatars are only
    /// generated for newly-seen artist names — existing entries in
    /// `artist_avatars` are reused.
    pub(super) fn merge_artists_from_batch(&mut self, new_albums: &[Album]) {
        // Build an index over the current artists list for O(1) lookup.
        let mut index: HashMap<String, usize> = self
            .all_artists
            .iter()
            .enumerate()
            .map(|(i, a)| (a.name.clone(), i))
            .collect();

        let split_enabled = self.config.split_artist_tags;
        let delimiters = self.config.artist_tag_delimiters.clone();

        for album in new_albums {
            let names: Vec<String> = if split_enabled {
                crate::library::artist_tags::split(&album.artist, &delimiters)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            } else {
                vec![album.artist.clone()]
            };

            for name in names {
                if let Some(&idx) = index.get(&name) {
                    self.all_artists[idx].albums.push(album.clone());
                } else {
                    let idx = self.all_artists.len();
                    index.insert(name.clone(), idx);
                    self.all_artists.push(Artist {
                        name: name.clone(),
                        albums: vec![album.clone()],
                    });

                    // Generate an avatar only for artists we've never seen,
                    // in a single hash lookup.
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        self.artist_avatars.entry(name)
                    {
                        let bytes =
                            crate::library::CoverArt::generate_artist_avatar(slot.key(), 64);
                        slot.insert(widget::icon::from_raster_bytes(bytes));
                    }
                }
            }
        }
    }

    /// Rebuild `all_artists` from scratch by re-running the batch-merge
    /// aggregation (`merge_artists_from_batch`) over every album currently
    /// in `all_albums`. This is the single aggregation path — used
    /// whenever `split_artist_tags` or `artist_tag_delimiters` changes, so
    /// the artist index picks up the new split rules immediately, without
    /// a rescan.
    pub(super) fn rebuild_all_artists(&mut self) {
        self.all_artists.clear();
        let albums = std::mem::take(&mut self.all_albums);
        self.merge_artists_from_batch(&albums);
        self.all_artists.sort_by(|a, b| a.name.cmp(&b.name));
        self.all_albums = albums;
    }

    /// Start playback from the given queue at `start_index`.
    ///
    /// Takes ownership of the track list to avoid an extra clone — the
    /// caller is responsible for providing an owned `Vec<Track>`.
    pub(super) fn play_track_list(
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
                let mpris_task = self.publish_mpris();
                return Task::batch([podcast_save_task, mpd_task, blur_task, mpris_task]);
            }
        }
        podcast_save_task
    }

    /// Builds an `MprisSnapshot` from current player/config state and
    /// forwards it to the MPRIS D-Bus handle, if the session-bus server is
    /// up. `MprisHandle::publish` diffs internally, so calling this
    /// unconditionally on every tick is cheap.
    ///
    /// Cover art resolution involves blocking file I/O (tag parsing, an
    /// on-disk cache write) the first time a track's art is requested, so
    /// it is never done synchronously here: if the current track's art
    /// isn't already cached on `handle`, this publishes without `art_url`
    /// for now and returns a background task that resolves it and
    /// republishes via `Message::MprisArtResolved` once ready.
    pub(super) fn publish_mpris(&self) -> Task<cosmic::Action<Message>> {
        let Some(handle) = self.mpris.as_ref() else {
            return Task::none();
        };

        let status = match self.player.as_ref().map(|p| p.state()) {
            Some(PlaybackState::Playing) => crate::mpris::MprisStatus::Playing,
            Some(PlaybackState::Paused) => crate::mpris::MprisStatus::Paused,
            Some(PlaybackState::Stopped) | None => crate::mpris::MprisStatus::Stopped,
        };

        let loop_mode = match self.config.repeat_mode {
            crate::config::RepeatMode::None => crate::mpris::LoopMode::None,
            crate::config::RepeatMode::All => crate::mpris::LoopMode::Playlist,
            crate::config::RepeatMode::One => crate::mpris::LoopMode::Track,
        };

        let track = self.current_track.as_ref();
        let (art_url, art_task) = match track {
            Some(t) => match handle.cached_art_url(t.id) {
                Some(cached) => (cached, Task::none()),
                None => (None, resolve_mpris_art_task(t.id, t.path.clone())),
            },
            None => (None, Task::none()),
        };
        let has_queue = !self.all_tracks.is_empty();

        handle.publish(crate::mpris::MprisSnapshot {
            status,
            title: track.map(|t| t.title.clone()).unwrap_or_default(),
            artist: track.map(|t| t.artist.clone()).unwrap_or_default(),
            album: track.map(|t| t.album.clone()).unwrap_or_default(),
            album_artist: track.map(|t| t.album_artist.clone()).unwrap_or_default(),
            genre: track.map(|t| t.genre.clone()).unwrap_or_default(),
            track_id: track.map(|t| t.id).unwrap_or(0),
            length_us: track.map(|t| t.duration.as_micros() as i64).unwrap_or(0),
            position_us: self.playback_position.as_micros() as i64,
            art_url,
            volume: self
                .player
                .as_ref()
                .map(|p| p.volume() as f64)
                .unwrap_or(self.config.volume as f64),
            shuffle: self.config.shuffle,
            loop_mode,
            can_go_next: track.is_some() && has_queue,
            can_go_previous: track.is_some() && has_queue,
            can_seek: track.is_some_and(|t| &*t.provider_id != "radio"),
            can_play: track.is_some(),
        });
        art_task
    }

    pub(super) fn sort_tracks(&mut self, field: songs::SortField) {
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
    pub(super) fn refresh_search_filter(&mut self) {
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

    /// Every track under the folder view's current directory, recursively,
    /// in path order. Shared by "play folder" and "add folder to queue".
    pub(super) fn current_folder_tracks(&self) -> Vec<Track> {
        let dir = self.folder_state.current().to_path_buf();
        self.folder_state
            .tree()
            .tracks_in(&dir, true)
            .into_iter()
            .filter_map(|i| self.all_tracks.get(i).cloned())
            .collect()
    }

    /// Pushes a toast notification, mapping its auto-dismiss timer task into
    /// the `cosmic::Action`-wrapped message type `update()` returns.
    pub(super) fn push_toast(
        &mut self,
        toast: widget::toaster::Toast<Message>,
    ) -> Task<cosmic::Action<Message>> {
        self.toasts.push(toast).map(cosmic::Action::App)
    }

    /// Exit visualizer fullscreen if active: restore the COSMIC header bar and
    /// nav sidebar. No-op when not in fullscreen. Called whenever the expanded
    /// now-playing view is left or the visualizer is turned off.
    #[cfg(feature = "visualizer")]
    pub(super) fn exit_viz_fullscreen(&mut self) {
        if self.viz_fullscreen {
            self.viz_fullscreen = false;
            self.core.window.show_headerbar = true;
            self.core.nav_bar_set_toggled(self.viz_prev_nav_active);
        }
    }

    /// Trigger blur + accent-colour computation for the current track if
    /// the album changed.
    ///
    /// Checks if the current track's album key differs from the cached blurred
    /// cover key. If so, looks up the raw bytes and spawns a background task
    /// that computes both the blur and the cover-art accent colour from the
    /// same bytes (see `library::palette::extract`). Returns a Task that
    /// sends `Message::BlurReady`.
    pub(super) fn maybe_update_blurred_cover(&mut self) -> Task<cosmic::Action<Message>> {
        let track = match self.current_track.as_ref() {
            Some(t) => t,
            None => {
                // No track — clear everything.
                self.blurred_cover = None;
                self.blurred_cover_key = None;
                self.accent = None;
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

        // Bytes are available — start the async blur+accent computation.
        // Clear the key now so a concurrent track change will not skip
        // the next blur computation (BlurReady carries the key and will
        // only apply if it still matches the current track).
        self.blurred_cover_key = None;

        let key_clone = key.clone();
        cosmic::task::future(async move {
            // Compute blur and accent in the same blocking task, off the
            // async runtime, from the same bytes. Accent extraction is
            // bounded-cost (fixed 32x32 working set) regardless of source
            // resolution, so it adds no meaningful overhead next to the blur.
            let (blurred, accent) = tokio::task::spawn_blocking(move || {
                let blurred = crate::views::now_playing::blur::compute_blurred_cover(&bytes);
                let accent = crate::library::palette::extract(&bytes);
                (blurred, accent)
            })
            .await
            .unwrap_or((None, None));

            let handle = blurred.map(widget::icon::from_raster_bytes);
            cosmic::Action::App(Message::BlurReady(key_clone, handle, accent))
        })
    }

    /// Load genres from the active provider and dispatch a GenresLoaded message.
    pub(super) fn load_genres(&self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn load_podcasts(&self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn load_podcast_episodes(&self, podcast_id: i64) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn load_radio_stations(&self) -> Task<cosmic::Action<Message>> {
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
    pub(super) fn load_online_icons(&self, urls: Vec<String>) -> Task<cosmic::Action<Message>> {
        let tasks: Vec<_> = urls
            .into_iter()
            .filter(|url| !url.is_empty() && !self.online_icons.contains_key(url))
            .map(|url| {
                let fetch_url = url.clone();
                cosmic::task::future(async move {
                    let bytes = tokio::task::spawn_blocking(move || {
                        HTTP_CLIENT.clone()
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
    pub(super) fn save_podcast_position(
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

    /// Reads tags for a set of ad-hoc files -- double-clicked in a file
    /// manager via `Exec=lyra %U`, passed on the command line, or
    /// forwarded from another running instance's MPRIS `OpenUri` -- and
    /// queues the readable ones for playback. Deliberately bypasses the
    /// library database: these files need not live in any configured
    /// library directory, matching how every other desktop media player
    /// treats "open with".
    pub(super) fn open_files(&mut self, paths: Vec<PathBuf>) -> Task<cosmic::Action<Message>> {
        cosmic::task::future(async move {
            let tracks = tokio::task::spawn_blocking(move || {
                paths
                    .into_iter()
                    .filter_map(|path| match LibraryScanner::read_metadata(&path) {
                        Ok(track) => Some(track),
                        Err(e) => {
                            tracing::warn!("Skipping unreadable file {}: {e}", path.display());
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            cosmic::Action::App(Message::OpenFilesScanned(tracks))
        })
    }
}
