// SPDX-License-Identifier: GPL-3.0

//! Playback controls: a persistent bottom bar and expandable now-playing view.
//!
//! The bottom bar contains: cover art, track info, transport controls, seek bar,
//! volume slider, and utility buttons. Clicking the bar expands into a full
//! now-playing view with large cover art, metadata, and optional visualizer.

pub mod animation;
pub mod blur;
pub mod compact_bar;
pub mod expanded_view;
#[cfg(feature = "visualizer")]
pub mod visualizer;
#[cfg(feature = "visualizer")]
pub mod viz_shader;

use std::time::Duration;

/// Messages from the now-playing controls.
#[derive(Debug, Clone)]
pub enum NowPlayingMessage {
    TogglePlayback,
    Next,
    Previous,
    /// Continuous update during slider drag (visual feedback only).
    SeekPreview(f32),
    /// Emitted on mouse release — performs the actual backend seek.
    SeekCommit,
    SetVolume(f32),
    ToggleShuffle,
    CycleRepeat,
    ShowLyrics,
    /// Click on bar background — expand to full view.
    ExpandToggle,
    /// Collapse button or Escape — return to compact bar.
    Collapse,
    /// Toggle favorite for the currently playing track (track ID as string).
    ToggleFavorite(String),
    /// Toggle the ProjectM visualizer on/off.
    #[cfg(feature = "visualizer")]
    ToggleVisualizer,
    /// Cycle to the next visualizer preset.
    #[cfg(feature = "visualizer")]
    NextPreset,
    /// Double-click on visualizer background — toggle fullscreen.
    #[cfg(feature = "visualizer")]
    ToggleVizFullscreen,
}

/// Format a duration as `H:MM:SS` / `M:SS`.
pub fn format_time(d: Duration) -> String {
    super::common::format_duration(d.as_secs())
}

/// Truncate a string to `max_chars`, appending `…` if it exceeds the limit.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    super::common::truncate_str(s, max_chars)
}
