// SPDX-License-Identifier: GPL-3.0

//! The full-screen expanded now-playing view.
//!
//! Layout:
//! ```text
//! Stack [
//!   black_base  (Fill × Fixed(800)),
//!   bg_layer    (blur / visualizer, Fill × Fixed(800)),
//!   outer_col   (column: Space(Fill) + cover_art + frosted_panel, Fixed(800)),
//!   [viz_metadata_overlay]  (optional, cfg visualizer)
//! ]
//! ```
//!
//! The frosted panel is a `Container::custom` with `rgba(0,0,0,0.72)` and
//! radius only on the top corners so it looks flush at the bottom.

use super::{format_time, truncate_str, NowPlayingMessage};
use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Stack;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

#[cfg(feature = "visualizer")]
use std::sync::{Arc, Mutex};

/// Render the expanded now-playing view.
///
/// Uses a frosted bottom panel for all controls/metadata, with the cover art
/// floating freely above the full-bleed background.
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
    #[cfg(feature = "visualizer")] viz_frame_buf: Arc<Mutex<super::viz_shader::VizFrameBuffer>>,
    #[cfg(feature = "visualizer")] viz_metadata_opacity: f32,
) -> cosmic::Element<'a, NowPlayingMessage> {
    let _ = expand_progress; // used by outer fade animation, not per-panel

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

    // ── Collapse button (lives in the panel header row) ───────────────────
    let collapse_btn = widget::button::icon(widget::icon::from_name("go-down-symbolic").size(24))
        .on_press(NowPlayingMessage::Collapse);

    // ── Cover art hero widget ─────────────────────────────────────────────
    let cover_size = Length::Fixed(280.0);
    let hero_widget: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
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
                .size(180)
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

    // ── Seek bar ──────────────────────────────────────────────────────────
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
        .align_y(Alignment::Center);
    // Note: no extra horizontal padding here; the panel container provides it.

    // ── Transport controls ────────────────────────────────────────────────
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

    // ── Volume + lyrics + visualizer buttons ──────────────────────────────
    #[allow(unused_mut)]
    let mut bottom_controls = widget::row()
        .push(widget::icon::from_name("audio-volume-high-symbolic").size(24))
        .push(
            widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume)
                .step(0.01)
                .width(180),
        )
        .push(
            widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic").size(28))
                .on_press(NowPlayingMessage::ShowLyrics),
        );

    #[cfg(feature = "visualizer")]
    {
        let viz_icon = "applications-multimedia-symbolic";
        bottom_controls = bottom_controls.push(
            widget::button::icon(widget::icon::from_name(viz_icon).size(28))
                .on_press(NowPlayingMessage::ToggleVisualizer),
        );
        if visualizer_active {
            bottom_controls = bottom_controls.push(
                widget::button::icon(
                    widget::icon::from_name("media-skip-forward-symbolic").size(24),
                )
                .on_press(NowPlayingMessage::NextPreset),
            );
        }
    }

    let bottom_controls = bottom_controls.spacing(12).align_y(Alignment::Center);

    // ── Frosted bottom panel ──────────────────────────────────────────────
    //
    // Contains (top-to-bottom):
    //   1. Title row: track title (title2) + Space(Fill) + collapse button
    //   2. Subtitle row: Artist · Album · Year · Genre (body, muted)
    //   3. Technical caption (bitrate · sample_rate · disc/track) — if present
    //   4. Seek bar
    //   5. Transport controls (centered)
    //   6. Volume + extras row

    let panel_content: cosmic::Element<'_, NowPlayingMessage> = {
        let mut col = widget::column();

        if let Some(track) = current_track {
            // Title row
            let title_row = widget::row()
                .push(widget::text::title2(truncate_str(&track.title, 40)))
                .push(widget::Space::new(Length::Fill, Length::Shrink))
                .push(collapse_btn)
                .align_y(Alignment::Center);
            col = col.push(title_row);

            // Subtitle: Artist · Album · Year · Genre
            let mut sub_parts: Vec<String> = Vec::new();
            if !track.artist.is_empty() {
                sub_parts.push(track.artist.clone());
            }
            if !track.album.is_empty() {
                sub_parts.push(track.album.clone());
            }
            if track.year > 0 {
                sub_parts.push(track.year.to_string());
            }
            if !track.genre.is_empty() {
                sub_parts.push(track.genre.clone());
            }
            if !sub_parts.is_empty() {
                col = col.push(widget::text::body(sub_parts.join(" \u{2022} ")).class(
                    cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                        color: Some(theme.cosmic().palette.neutral_7.into()),
                    }),
                ));
            }

            // Technical info caption
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
                col = col.push(widget::text::caption(tech_parts.join(" \u{2022} ")).class(
                    cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                        color: Some(theme.cosmic().palette.neutral_6.into()),
                    }),
                ));
            }
        } else {
            // No track: show placeholder title row with collapse btn
            let title_row = widget::row()
                .push(widget::text::title2("No track playing"))
                .push(widget::Space::new(Length::Fill, Length::Shrink))
                .push(collapse_btn)
                .align_y(Alignment::Center);
            col = col.push(title_row);
        }

        col = col
            .push(seek_bar)
            .push(
                widget::container(transport)
                    .align_x(Horizontal::Center)
                    .width(Length::Fill),
            )
            .push(bottom_controls);

        col.spacing(16).width(Length::Fill).into()
    };

    // Frosted container: dark semi-opaque background, top corners rounded
    let panel = widget::container(panel_content)
        .width(Length::Fill)
        .padding([20, 24, 24, 24])
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            let r = cosmic.radius_m();
            cosmic::iced::widget::container::Style {
                background: Some(cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.72).into()),
                border: cosmic::iced::Border {
                    color: cosmic::iced::Color::from_rgba(1.0, 1.0, 1.0, 0.08),
                    width: 1.0,
                    // Top corners rounded, bottom flush with window edge
                    radius: [r[0], r[1], 0.0, 0.0].into(),
                },
                ..Default::default()
            }
        }));

    // ── outer_col: Space(Fill) pushes cover art + panel to the bottom ─────
    let outer_col = widget::column()
        .push(widget::Space::new(Length::Shrink, Length::Fill))
        .push(
            widget::container(hero_widget)
                .align_x(Horizontal::Center)
                .width(Length::Fill)
                .padding([0, 0, 24, 0]),
        )
        .push(panel)
        .width(Length::Fill)
        .height(Length::Fixed(800.0));

    // ── Visualizer metadata overlay (top-left pill, fades out) ────────────
    #[cfg(feature = "visualizer")]
    let viz_metadata_overlay: Option<cosmic::Element<'_, NowPlayingMessage>> =
        if visualizer_active && viz_metadata_opacity > 0.0 {
            if let Some(track) = current_track {
                let subtitle = {
                    let mut parts = vec![track.artist.as_str()];
                    if !track.album.is_empty() {
                        parts.push(track.album.as_str());
                    }
                    parts.join(" \u{2022} ")
                };

                let overlay_col = widget::column()
                    .push(widget::text::title3(truncate_str(&track.title, 40)))
                    .push(
                        widget::text::body(subtitle).class(cosmic::theme::Text::Custom(|theme| {
                            cosmic::iced::widget::text::Style {
                                color: Some(theme.cosmic().palette.neutral_7.into()),
                            }
                        })),
                    )
                    .spacing(4);

                let overlay_pill = widget::container(overlay_col).padding([12, 16]).class(
                    cosmic::theme::Container::custom(move |_theme| {
                        cosmic::iced::widget::container::Style {
                            background: Some(
                                cosmic::iced::Color::from_rgba(
                                    0.0,
                                    0.0,
                                    0.0,
                                    0.65 * viz_metadata_opacity,
                                )
                                .into(),
                            ),
                            // Propagate opacity to child text so text and
                            // background fade in lockstep.
                            text_color: Some(cosmic::iced::Color::from_rgba(
                                1.0,
                                1.0,
                                1.0,
                                viz_metadata_opacity,
                            )),
                            border: cosmic::iced::Border {
                                radius: [8.0; 4].into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }),
                );

                let positioned = widget::container(overlay_pill)
                    .padding([24, 0, 0, 24])
                    .width(Length::Fill)
                    .height(Length::Fixed(800.0));

                Some(positioned.into())
            } else {
                None
            }
        } else {
            None
        };

    // ── Background element ────────────────────────────────────────────────
    #[cfg(feature = "visualizer")]
    let bg_element: Option<cosmic::Element<'_, NowPlayingMessage>> = if visualizer_active {
        let shader = cosmic::iced::widget::Shader::new(super::viz_shader::VizProgram::new(
            Arc::clone(&viz_frame_buf),
        ))
        .width(Length::Fill)
        .height(Length::Fixed(800.0));

        Some(
            widget::mouse_area(shader)
                .on_double_press(NowPlayingMessage::ToggleVizFullscreen)
                .into(),
        )
    } else {
        blurred_cover.map(|h| {
            let el: cosmic::Element<'_, NowPlayingMessage> = widget::icon::icon(h.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(Length::Fill)
                .height(Length::Fixed(800.0))
                .into();
            el
        })
    };

    #[cfg(not(feature = "visualizer"))]
    let bg_element: Option<cosmic::Element<'_, NowPlayingMessage>> = blurred_cover.map(|h| {
        let el: cosmic::Element<'_, NowPlayingMessage> = widget::icon::icon(h.clone())
            .content_fit(cosmic::iced::ContentFit::Cover)
            .width(Length::Fill)
            .height(Length::Fixed(800.0))
            .into();
        el
    });

    // ── Assemble Stack ────────────────────────────────────────────────────
    if let Some(bg_layer) = bg_element {
        // Solid black base: safety net before first shader frame upload.
        let black_base: cosmic::Element<'_, NowPlayingMessage> =
            widget::container(widget::Space::new(0, 0))
                .width(Length::Fill)
                .height(Length::Fixed(800.0))
                .class(cosmic::theme::Container::custom(|_theme| {
                    cosmic::iced::widget::container::Style {
                        background: Some(Color::BLACK.into()),
                        ..Default::default()
                    }
                }))
                .into();

        // Stack: black_base → bg_layer → outer_col → [viz_metadata_overlay]
        #[allow(unused_mut)]
        let mut stack_widget = Stack::new().push(black_base).push(bg_layer).push(outer_col);

        #[cfg(feature = "visualizer")]
        if let Some(meta_overlay) = viz_metadata_overlay {
            stack_widget = stack_widget.push(meta_overlay);
        }

        stack_widget.width(Length::Fill).into()
    } else {
        // No background image — themed solid background, frosted panel still shown.
        widget::container(outer_col)
            .width(Length::Fill)
            .align_x(Horizontal::Center)
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
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
            }))
            .into()
    }
}
