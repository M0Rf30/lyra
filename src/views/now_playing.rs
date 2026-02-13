// SPDX-License-Identifier: GPL-3.0

//! Now Playing / playback bar view rendered in the bottom of the window.
//! Also used for the "Now Playing" full page view with large cover art.

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
    Seek(f32),
    SetVolume(f32),
    ToggleShuffle,
    CycleRepeat,
    ShowLyrics,
}

/// Render the bottom playback bar (Lollypop-style persistent bar).
pub fn playback_bar<'a>(
    current_track: Option<&'a Track>,
    state: PlaybackState,
    position: Duration,
    duration: Duration,
    volume: f32,
    shuffle: bool,
    repeat_mode: &str,
) -> cosmic::Element<'a, NowPlayingMessage> {
    let progress = if duration.as_secs_f32() > 0.0 {
        position.as_secs_f32() / duration.as_secs_f32()
    } else {
        0.0
    };

    // Track info (left side)
    let track_info: cosmic::Element<'_, NowPlayingMessage> = if let Some(track) = current_track {
        widget::row()
            .push(
                widget::container(
                    widget::icon::from_name("media-optical-cd-audio-symbolic").size(40),
                )
                .width(48)
                .height(48)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Card),
            )
            .push(
                widget::column()
                    .push(widget::text(track.title.as_str()))
                    .push(widget::text::caption(track.artist.as_str()))
                    .spacing(2),
            )
            .spacing(8)
            .align_y(Alignment::Center)
            .width(250)
            .into()
    } else {
        widget::container(widget::text("No track playing"))
            .width(250)
            .into()
    };

    // Playback controls (center)
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

    let repeat_icon = match repeat_mode {
        "one" => "media-playlist-repeat-song-symbolic",
        "all" => "media-playlist-repeat-symbolic",
        _ => "media-playlist-no-repeat-symbolic",
    };

    let controls = widget::row()
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

    let position_text = format_time(position);
    let duration_text = format_time(duration);

    let seek_bar = widget::row()
        .push(widget::text::caption(position_text))
        .push(widget::slider(0.0..=1.0, progress, NowPlayingMessage::Seek).width(Length::Fill))
        .push(widget::text::caption(duration_text))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let center_section = widget::column()
        .push(controls)
        .push(seek_bar)
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::Fill);

    // Volume + extras (right side)
    let right_section = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic"))
                .on_press(NowPlayingMessage::ShowLyrics),
        )
        .push(widget::icon::from_name("audio-volume-high-symbolic").size(16))
        .push(widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume).width(100))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(250);

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

/// Render the header bar playback controls (CSD-style controls in the title bar).
pub fn header_playback_controls<'a>(
    state: PlaybackState,
    position: Duration,
    duration: Duration,
) -> Vec<cosmic::Element<'a, NowPlayingMessage>> {
    let play_icon = if state == PlaybackState::Playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    };

    let progress = if duration.as_secs_f32() > 0.0 {
        position.as_secs_f32() / duration.as_secs_f32()
    } else {
        0.0
    };

    vec![
        widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic"))
            .on_press(NowPlayingMessage::Previous)
            .into(),
        widget::button::icon(widget::icon::from_name(play_icon))
            .on_press(NowPlayingMessage::TogglePlayback)
            .into(),
        widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic"))
            .on_press(NowPlayingMessage::Next)
            .into(),
        widget::slider(0.0..=1.0, progress, NowPlayingMessage::Seek)
            .width(200)
            .into(),
        widget::text::caption(format!(
            "{} / {}",
            format_time(position),
            format_time(duration)
        ))
        .into(),
    ]
}

fn format_time(d: Duration) -> String {
    let total = d.as_secs();
    let min = total / 60;
    let sec = total % 60;
    format!("{min}:{sec:02}")
}
