// SPDX-License-Identifier: GPL-3.0

//! The compact bottom playback bar.
//!
//! Fixed at 84px tall so it can never balloon vertically — every text label
//! in this bar is single-line (`common::cell_text` / `cell_caption`,
//! truncated) and every other element has a fixed or portioned width.
//! Clicking the cover art or track info expands into the full now-playing
//! view.

use super::{NowPlayingMessage, format_time};
use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use crate::views::common;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::prelude::*;
use cosmic::widget;
use cosmic::widget::tooltip::Position as TooltipPosition;
use std::time::Duration;

/// Fixed height of the compact bar — never grows regardless of content.
const BAR_HEIGHT: f32 = 84.0;

/// Cover art side length.
const COVER_SIZE: f32 = 56.0;

/// An icon button that also renders an accent-tinted "active" state
/// (shuffle / repeat), optionally wrapped in a caption tooltip.
fn toggle_icon_button<'a, M: Clone + 'static>(
    icon_name: &'static str,
    icon_size: u16,
    active: bool,
    label: &'a str,
    on_press: M,
) -> cosmic::Element<'a, M> {
    let button = widget::button::icon(widget::icon::from_name(icon_name).size(icon_size))
        .selected(active)
        .on_press(on_press);
    widget::tooltip(button, widget::text::caption(label), TooltipPosition::Top).into()
}

/// Render the compact bottom playback bar.
///
/// Layout (left to right):
/// ```text
/// [Cover] [Title/Artist] [♥] | [Shuffle Prev Play Next Repeat] / [Seek bar] | [Vol slider] [Lyrics]
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

    // --- Cover art: fixed 56x56, cropped to fill; placeholder when missing ---
    let art: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
        widget::icon::icon(handle.clone())
            .content_fit(ContentFit::Cover)
            .width(Length::Fixed(COVER_SIZE))
            .height(Length::Fixed(COVER_SIZE))
            .into()
    } else {
        widget::icon::from_name("media-optical-cd-audio-symbolic")
            .size(40)
            .into()
    };
    let cover = widget::mouse_area(
        widget::container(art)
            .width(Length::Fixed(COVER_SIZE))
            .height(Length::Fixed(COVER_SIZE))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .on_press(NowPlayingMessage::ExpandToggle);

    // --- Track info: single-line title + single-line "artist — album" ---
    let (info_content, heart): (
        cosmic::Element<'_, NowPlayingMessage>,
        cosmic::Element<'_, NowPlayingMessage>,
    ) = if let Some(track) = current_track {
        let subtitle = if track.album.is_empty() {
            track.artist.clone()
        } else {
            format!("{} — {}", track.artist, track.album)
        };
        let column = widget::Column::new()
            .push(common::cell_text(common::truncate_str(&track.title, 60)))
            .push(common::cell_caption(common::truncate_str(&subtitle, 70)))
            .spacing(2)
            .width(Length::Fill)
            .into();
        let heart = common::favorite_button(
            track.is_favorite,
            NowPlayingMessage::ToggleFavorite(track.id.to_string()),
        );
        (column, heart)
    } else {
        let column = widget::Column::new()
            .push(common::cell_caption("No track playing"))
            .width(Length::Fill)
            .into();
        (column, widget::Space::new().width(0).height(0).into())
    };

    let info = widget::mouse_area(
        widget::container(info_content)
            .width(Length::FillPortion(2))
            .align_y(Vertical::Center),
    )
    .on_press(NowPlayingMessage::ExpandToggle);

    // --- Center: transport controls (row 1) + seek bar (row 2) ---
    let play_icon = if state == PlaybackState::Playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    };
    let play_label = if state == PlaybackState::Playing {
        "Pause"
    } else {
        "Play"
    };

    let shuffle_icon = if shuffle {
        "media-playlist-shuffle-symbolic"
    } else {
        "media-playlist-consecutive-symbolic"
    };
    let repeat_icon = repeat_mode.icon_name();
    let repeat_active = repeat_mode != RepeatMode::None;

    let transport = widget::Row::new()
        .push(toggle_icon_button(
            shuffle_icon,
            24,
            shuffle,
            "Shuffle",
            NowPlayingMessage::ToggleShuffle,
        ))
        .push(common::icon_button(
            "media-skip-backward-symbolic",
            24,
            "Previous",
            NowPlayingMessage::Previous,
        ))
        .push(common::icon_button(
            play_icon,
            32,
            play_label,
            NowPlayingMessage::TogglePlayback,
        ))
        .push(common::icon_button(
            "media-skip-forward-symbolic",
            24,
            "Next",
            NowPlayingMessage::Next,
        ))
        .push(toggle_icon_button(
            repeat_icon,
            24,
            repeat_active,
            "Repeat",
            NowPlayingMessage::CycleRepeat,
        ))
        .spacing(4)
        .align_y(Alignment::Center);

    let seek_bar = widget::Row::new()
        .push(common::cell_caption(format_time(display_position)))
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .step(0.001)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::Fill),
        )
        .push(common::cell_caption(format_time(duration)))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let center_column = widget::Column::new()
        .push(transport)
        .push(seek_bar)
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::FillPortion(3));

    // --- Volume: icon reflects current level, fixed-width slider ---
    let volume_icon_name = if volume <= 0.0 {
        "audio-volume-muted-symbolic"
    } else if volume < 0.33 {
        "audio-volume-low-symbolic"
    } else if volume < 0.66 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    };

    let volume_block = widget::Row::new()
        .push(widget::icon::from_name(volume_icon_name).size(20))
        .push(
            widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume)
                .step(0.01)
                .width(Length::Fixed(120.0)),
        )
        .spacing(8)
        .align_y(Alignment::Center);

    // --- Utility buttons ---
    let utility_buttons = widget::Row::new()
        .push(common::icon_button(
            "view-list-lyrics-symbolic",
            20,
            "Lyrics",
            NowPlayingMessage::ShowLyrics,
        ))
        .spacing(4)
        .align_y(Alignment::Center);

    let controls_row = widget::Row::new()
        .push(cover)
        .push(info)
        .push(heart)
        .push(center_column)
        .push(volume_block)
        .push(utility_buttons)
        .spacing(12)
        .padding([8, 16])
        .align_y(Alignment::Center);

    widget::container(controls_row)
        .width(Length::Fill)
        .height(Length::Fixed(BAR_HEIGHT))
        .align_y(Vertical::Center)
        .class(cosmic::theme::Container::Card)
        .into()
}
