// SPDX-License-Identifier: GPL-3.0

//! The compact bottom playback bar.
//!
//! This matches the original playback bar appearance. Clicking the bar
//! background expands into the full now-playing view.

use super::{NowPlayingMessage, format_time, truncate_str};
use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use crate::views::spacing;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

/// Render the compact bottom playback bar.
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
    seeking_preview: Option<f32>,
    _blurred_cover: Option<&'a widget::icon::Handle>,
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

    // --- Left: cover art + track info + heart icon ---
    let track_info: cosmic::Element<'_, NowPlayingMessage> = if let Some(track) = current_track {
        let art: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
            widget::icon::icon(handle.clone()).size(60).into()
        } else {
            widget::icon::from_name("media-optical-cd-audio-symbolic")
                .size(48)
                .into()
        };

        let fav_icon_name = if track.is_favorite {
            "emblem-favorite-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let heart_btn = widget::button::icon(widget::icon::from_name(fav_icon_name).size(20))
            .on_press(NowPlayingMessage::ToggleFavorite(track.id.to_string()));

        widget::row()
            .push(
                widget::container(art)
                    .width(60)
                    .height(60)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            )
            .push(
                widget::column()
                    .push(widget::text::body(truncate_str(&track.title, 30)))
                    .push(widget::text::caption(truncate_str(&track.artist, 30)))
                    .spacing(spacing::XXXS)
                    .width(Length::Fill),
            )
            .push(heart_btn)
            .spacing(spacing::XS)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(1))
            .into()
    } else {
        widget::row()
            .push(widget::icon::from_name("media-optical-cd-audio-symbolic").size(40))
            .push(widget::text::caption("No track playing"))
            .spacing(spacing::XS)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(1))
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
            widget::button::icon(widget::icon::from_name(shuffle_icon).size(24))
                .on_press(NowPlayingMessage::ToggleShuffle),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic").size(24))
                .on_press(NowPlayingMessage::Previous),
        )
        .push(
            widget::button::icon(widget::icon::from_name(play_icon).size(32))
                .on_press(NowPlayingMessage::TogglePlayback),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic").size(24))
                .on_press(NowPlayingMessage::Next),
        )
        .push(
            widget::button::icon(widget::icon::from_name(repeat_icon).size(24))
                .on_press(NowPlayingMessage::CycleRepeat),
        )
        .spacing(spacing::XXXS)
        .align_y(Alignment::Center);

    // --- Seek bar with time labels ---
    let seek_bar = widget::row()
        .push(widget::text::caption(format_time(display_position)))
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .step(0.001)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::Fill),
        )
        .push(widget::text::caption(format_time(duration)))
        .spacing(spacing::XXS)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // --- Right: volume + lyrics ---
    let right_section = widget::container(
        widget::row()
            .push(widget::icon::from_name("audio-volume-high-symbolic").size(20))
            .push(
                widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume)
                    .step(0.01)
                    .width(120),
            )
            .push(
                widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic"))
                    .on_press(NowPlayingMessage::ShowLyrics),
            )
            .spacing(spacing::XXS)
            .align_y(Alignment::Center),
    )
    .align_x(Horizontal::Right)
    .width(Length::FillPortion(1));

    let center_section = widget::column()
        .push(transport)
        .push(seek_bar)
        .spacing(spacing::XXXS)
        .align_x(Alignment::Center)
        .width(Length::FillPortion(2));

    let expandable_info = widget::mouse_area(track_info).on_press(NowPlayingMessage::ExpandToggle);

    let controls_row = widget::row()
        .push(expandable_info)
        .push(center_section)
        .push(right_section)
        .spacing(spacing::S)
        .align_y(Alignment::Center)
        .padding(spacing::XS);

    widget::container(controls_row)
        .width(Length::Fill)
        .class(cosmic::theme::Container::Card)
        .into()
}
