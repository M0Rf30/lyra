// SPDX-License-Identifier: GPL-3.0

use super::{AppModel, Message};
use crate::config::Config;
use crate::convert::JobState;
use crate::player::{ActiveBackend, PlaybackState};
use crate::provider::MusicProvider;
use crate::provider::mpd::MpdProvider;
use cosmic::Application;
use cosmic::iced::Subscription;
use futures_util::{SinkExt, Stream};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "visualizer")]
use std::sync::Mutex;
use std::time::Duration;

impl AppModel {
    pub(super) fn build_subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            // Watch config changes
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];

        // MPRIS2 D-Bus server — unconditional and keyed by the zero-argument
        // `mpris_stream` function, so it starts once and never restarts.
        subs.push(Subscription::run(crate::mpris::mpris_stream).map(Message::Mpris));

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
        if self
            .convert_jobs
            .iter()
            .any(|j| j.state == JobState::Running)
        {
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

        // Global keyboard shortcuts (play/pause, seek, volume, shuffle,
        // repeat, favorite, lyrics, expand/collapse, nav-page jumps,
        // search focus, escape). Replaces the old ad hoc Space/Escape/
        // Ctrl+F listeners this block used to hold. This libcosmic revision
        // exposes no `keyboard::on_key_press`, so we filter the raw event
        // stream ourselves; `event::listen_with` only yields events a
        // focused widget didn't already capture (`Status::Ignored`), which
        // is exactly the guard those hand-rolled listeners implemented.
        // `crate::keybinds::resolve` is the pure, unit-tested mapping from
        // a raw key press to a `Shortcut`; `update()` decides what each one
        // does given the current app state.
        subs.push(cosmic::iced::event::listen_with(|event, status, _id| {
            if status != cosmic::iced::event::Status::Ignored {
                return None;
            }
            match event {
                cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    ..
                }) => crate::keybinds::resolve(&key, modifiers).map(Message::Shortcut),
                _ => None,
            }
        }));

        Subscription::batch(subs)
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
