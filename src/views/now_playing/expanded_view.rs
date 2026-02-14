// SPDX-License-Identifier: GPL-3.0

//! The full-screen expanded now-playing view.
//!
//! Displays large cover art, complete metadata, and enlarged controls.
//! The view uses a blurred album art background with a dark overlay.

use super::{format_time, truncate_str, NowPlayingMessage};
use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

/// Render the expanded now-playing view.
///
/// This view fills the available space and shows:
/// - Large cover art (300-400px) with rounded corners
/// - Title, artist, album, year, genre metadata
/// - Technical info (bitrate, sample rate, disc/track number)
/// - Wide seek bar with time labels
/// - Enlarged transport controls
/// - Volume slider and utility buttons
/// - Collapse button (down chevron)
#[allow(clippy::too_many_arguments)]
pub fn expanded_now_playing<'a>(
    current_track: Option<&'a Track>,
    state: PlaybackState,
    position: Duration,
    duration: Duration,
    volume: f32,
    shuffle: bool,
    repeat_mode: RepeatMode,
    cover_art: Option<&'a widget::icon::Handle>,
    blurred_cover: Option<&'a widget::icon::Handle>,
    seeking_preview: Option<f32>,
    expand_progress: f32,
    #[cfg(feature = "visualizer")] visualizer_active: bool,
    #[cfg(feature = "visualizer")] _visualizer_frame: Option<&'a widget::icon::Handle>,
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

    // Opacity for fade-in/out effect during animation
    let content_opacity = expand_progress;

    // --- Collapse button at top ---
    let collapse_btn = widget::button::icon(widget::icon::from_name("go-down-symbolic").size(24))
        .on_press(NowPlayingMessage::Collapse);

    let top_bar = widget::row()
        .push(widget::Space::new(Length::Fill, Length::Shrink))
        .push(collapse_btn)
        .push(widget::Space::new(Length::Fill, Length::Shrink))
        .padding([8, 16]);

    // --- Cover art (large, centered) ---
    let cover_size = Length::Fixed(320.0); // Fixed size for expanded view
    let cover_art_widget: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
        widget::container(
            widget::icon::icon(handle.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(cover_size)
                .height(cover_size),
        )
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            cosmic::iced::widget::container::Style {
                border: cosmic::iced::Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: cosmic.radius_l().into(),
                },
                shadow: cosmic::iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                    offset: cosmic::iced::Vector::new(0.0, 4.0),
                    blur_radius: 16.0,
                },
                ..Default::default()
            }
        }))
        .width(cover_size)
        .height(cover_size)
        .into()
    } else {
        let fallback_icon: cosmic::Element<'_, NowPlayingMessage> =
            widget::icon::from_name("media-optical-cd-audio-symbolic")
                .size(200)
                .into();
        widget::container(fallback_icon)
            .width(cover_size)
            .height(cover_size)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                cosmic::iced::widget::container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.2).into()),
                    border: cosmic::iced::Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: cosmic.radius_l().into(),
                    },
                    ..Default::default()
                }
            }))
            .into()
    };

    // --- Metadata section ---
    let metadata: cosmic::Element<'_, NowPlayingMessage> = if let Some(track) = current_track {
        let mut meta_col = widget::column()
            .push(widget::text::title1(truncate_str(&track.title, 50)))
            .push(widget::text::title3(truncate_str(&track.artist, 50)).class(
                cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                    color: Some(theme.cosmic().palette.neutral_7.into()),
                }),
            ))
            .spacing(4)
            .align_x(Alignment::Center);

        // Album + year + genre line (only if data exists)
        let mut album_line_parts = Vec::new();
        if !track.album.is_empty() {
            album_line_parts.push(track.album.clone());
        }
        if track.year > 0 {
            album_line_parts.push(track.year.to_string());
        }
        if !track.genre.is_empty() {
            album_line_parts.push(track.genre.clone());
        }
        if !album_line_parts.is_empty() {
            meta_col = meta_col.push(widget::text::body(album_line_parts.join(" \u{2022} ")));
        }

        // Technical info line (bitrate, sample_rate, disc/track)
        let mut tech_parts = Vec::new();
        if track.bitrate > 0 {
            tech_parts.push(format!("{} kbps", track.bitrate));
        }
        if track.sample_rate > 0 {
            tech_parts.push(format!("{} Hz", track.sample_rate));
        }
        if track.disc_number > 0 || track.track_number > 0 {
            let disc_track = if track.disc_number > 0 {
                format!("Disc {} / Track {}", track.disc_number, track.track_number)
            } else {
                format!("Track {}", track.track_number)
            };
            tech_parts.push(disc_track);
        }
        if !tech_parts.is_empty() {
            meta_col = meta_col.push(widget::text::caption(tech_parts.join(" \u{2022} ")).class(
                cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                    color: Some(theme.cosmic().palette.neutral_6.into()),
                }),
            ));
        }

        meta_col.into()
    } else {
        widget::column()
            .push(widget::text::title2("No track playing"))
            .align_x(Alignment::Center)
            .into()
    };

    // --- Seek bar (wide) ---
    let seek_bar = widget::row()
        .push(widget::text::body(format_time(display_position)))
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .step(0.001)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::FillPortion(3)),
        )
        .push(widget::text::body(format_time(duration)))
        .spacing(16)
        .align_y(Alignment::Center)
        .padding([0, 32]);

    // --- Transport controls (large) ---
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
            widget::button::icon(widget::icon::from_name(shuffle_icon).size(32))
                .on_press(NowPlayingMessage::ToggleShuffle),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic").size(32))
                .on_press(NowPlayingMessage::Previous),
        )
        .push(
            widget::button::icon(widget::icon::from_name(play_icon).size(36))
                .on_press(NowPlayingMessage::TogglePlayback),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic").size(32))
                .on_press(NowPlayingMessage::Next),
        )
        .push(
            widget::button::icon(widget::icon::from_name(repeat_icon).size(32))
                .on_press(NowPlayingMessage::CycleRepeat),
        )
        .spacing(12)
        .align_y(Alignment::Center);

    // --- Volume + lyrics + visualizer buttons ---
    #[allow(unused_mut)]
    let mut bottom_controls = widget::row()
        .push(widget::icon::from_name("audio-volume-high-symbolic").size(20))
        .push(
            widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume)
                .step(0.01)
                .width(150),
        )
        .push(
            widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic").size(24))
                .on_press(NowPlayingMessage::ShowLyrics),
        );

    // Visualizer toggle button (cfg-gated)
    #[cfg(feature = "visualizer")]
    {
        let viz_icon = "preferences-desktop-effects-symbolic";
        bottom_controls = bottom_controls.push(
            widget::button::icon(widget::icon::from_name(viz_icon).size(24))
                .on_press(NowPlayingMessage::ToggleVisualizer),
        );
        // Next preset button (only visible when visualizer is active)
        if visualizer_active {
            bottom_controls = bottom_controls.push(
                widget::button::icon(
                    widget::icon::from_name("media-skip-forward-symbolic").size(20),
                )
                .on_press(NowPlayingMessage::NextPreset),
            );
        }
    }

    let bottom_controls = bottom_controls.spacing(12).align_y(Alignment::Center);

    // --- Main content column ---
    // Use Shrink height — the COSMIC framework wraps view() output in a
    // scrollable, which panics if content uses Length::Fill on the scroll axis.
    let content = widget::column()
        .push(top_bar)
        .push(widget::Space::new(Length::Shrink, 24))
        .push(cover_art_widget)
        .push(widget::Space::new(Length::Shrink, 24))
        .push(metadata)
        .push(widget::Space::new(Length::Shrink, 24))
        .push(seek_bar)
        .push(widget::Space::new(Length::Shrink, 16))
        .push(transport)
        .push(widget::Space::new(Length::Shrink, 16))
        .push(bottom_controls)
        .push(widget::Space::new(Length::Shrink, 24))
        .align_x(Alignment::Center)
        .width(Length::Fill);

    // Wrap content in a container with dark background styling.
    // Avoid Length::Fill on the vertical axis — COSMIC wraps view() in a
    // scrollable which panics on Fill height content.
    let _ = content_opacity;
    let _ = blurred_cover;

    let styled_content = widget::container(content)
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .class(cosmic::theme::Container::custom(move |theme| {
            let cosmic = theme.cosmic();
            // Use the background image tint or solid dark fallback
            cosmic::iced::widget::container::Style {
                background: Some(
                    Color::from_rgba(
                        cosmic.background.base.red,
                        cosmic.background.base.green,
                        cosmic.background.base.blue,
                        1.0,
                    )
                    .into(),
                ),
                ..Default::default()
            }
        }));

    styled_content.into()
}
