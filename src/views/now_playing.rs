// SPDX-License-Identifier: GPL-3.0

//! Playback controls: a persistent bottom bar (Brigadeiro / Lollypop style).
//!
//! The bottom bar contains: cover art, track info, transport controls, seek bar,
//! volume slider, and utility buttons. The CSD header is kept clean for navigation.

use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;
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
}

/// Render the bottom playback bar.
///
/// Layout (left to right):
/// ```text
/// [CoverArt] [Title/Artist] | [Shuffle] [Prev] [Play/Pause] [Next] [Repeat] | [Time Seek Time] | [Vol] [Lyrics]
/// ```
#[allow(clippy::too_many_arguments)]
pub fn playback_bar<'a>(
    current_track: Option<&'a Track>,
    state: PlaybackState,
    position: Duration,
    duration: Duration,
    volume: f32,
    shuffle: bool,
    repeat_mode: RepeatMode,
    cover_art: Option<&'a widget::icon::Handle>,
    // When `Some`, the user is dragging the seek slider — fraction 0.0–1.0.
    seeking_preview: Option<f32>,
) -> cosmic::Element<'a, NowPlayingMessage> {
    // While dragging, show the preview position; otherwise the backend position.
    let (progress, display_position) = if let Some(frac) = seeking_preview {
        let preview_pos = Duration::from_secs_f32(frac * duration.as_secs_f32());
        (frac, preview_pos)
    } else {
        let p = if duration.as_secs_f32() > 0.0 {
            position.as_secs_f32() / duration.as_secs_f32()
        } else {
            0.0
        };
        (p, position)
    };

    // --- Left: cover art + track info ---
    let track_info: cosmic::Element<'_, NowPlayingMessage> = if let Some(track) = current_track {
        let art: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
            widget::icon::icon(handle.clone()).size(48).into()
        } else {
            widget::icon::from_name("media-optical-cd-audio-symbolic")
                .size(40)
                .into()
        };

        widget::row()
            .push(
                widget::container(art)
                    .width(48)
                    .height(48)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            )
            .push(
                widget::column()
                    .push(widget::text(track.title.as_str()))
                    .push(widget::text::caption(track.artist.as_str()))
                    .spacing(2),
            )
            .spacing(12)
            .align_y(Alignment::Center)
            .width(Length::Shrink)
            .into()
    } else {
        widget::row()
            .push(widget::icon::from_name("media-optical-cd-audio-symbolic").size(40))
            .push(widget::text::caption("No track playing"))
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
    };

    // --- Center: transport controls ---
    let play_icon = if state == PlaybackState::Playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    };

    let shuffle_icon = if shuffle {
        "media-playlist-shuffle-symbolic"
    } else {
        "media-playlist-consecutive-symbolic"
    };

    let repeat_icon = repeat_mode.icon_name();

    let transport = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name(shuffle_icon))
                .on_press(NowPlayingMessage::ToggleShuffle),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic"))
                .on_press(NowPlayingMessage::Previous),
        )
        .push(
            widget::button::icon(widget::icon::from_name(play_icon))
                .on_press(NowPlayingMessage::TogglePlayback),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic"))
                .on_press(NowPlayingMessage::Next),
        )
        .push(
            widget::button::icon(widget::icon::from_name(repeat_icon))
                .on_press(NowPlayingMessage::CycleRepeat),
        )
        .spacing(4)
        .align_y(Alignment::Center);

    // --- Seek bar with time labels ---
    let seek_bar = widget::row()
        .push(widget::text::caption(format_time(display_position)))
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::Fill),
        )
        .push(widget::text::caption(format_time(duration)))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // --- Right: volume + lyrics ---
    let right_section = widget::row()
        .push(widget::icon::from_name("audio-volume-high-symbolic").size(16))
        .push(widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume).width(100))
        .push(
            widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic"))
                .on_press(NowPlayingMessage::ShowLyrics),
        )
        .spacing(8)
        .align_y(Alignment::Center);

    // --- Compose: left | center+seek | right ---
    let center_section = widget::column()
        .push(transport)
        .push(seek_bar)
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::Fill);

    widget::container(
        widget::row()
            .push(track_info)
            .push(center_section)
            .push(right_section)
            .spacing(16)
            .align_y(Alignment::Center)
            .padding(8),
    )
    .width(Length::Fill)
    .class(cosmic::theme::Container::Card)
    .into()
}

fn format_time(d: Duration) -> String {
    let total = d.as_secs();
    let min = total / 60;
    let sec = total % 60;
    format!("{min}:{sec:02}")
}
