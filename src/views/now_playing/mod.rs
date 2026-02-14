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
    /// Toggle the ProjectM visualizer on/off.
    #[cfg(feature = "visualizer")]
    ToggleVisualizer,
    /// Cycle to the next visualizer preset.
    #[cfg(feature = "visualizer")]
    NextPreset,
}

/// Format a duration as M:SS.
pub fn format_time(d: Duration) -> String {
    let total = d.as_secs();
    let min = total / 60;
    let sec = total % 60;
    format!("{min}:{sec:02}")
}

/// Truncate a string to `max_chars` and add "..." if it exceeds the limit.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
