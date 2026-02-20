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
use cosmic::iced::widget::Stack;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

#[cfg(feature = "visualizer")]
use std::sync::{Arc, Mutex};

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
    #[cfg(feature = "visualizer")] viz_frame_buf: Arc<Mutex<super::viz_shader::VizFrameBuffer>>,
    #[cfg(feature = "visualizer")] viz_metadata_opacity: f32,
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

    // --- Cover art hero widget ---
    // The visualizer, when active, is rendered as the Stack background layer
    // only (not inline) so it doesn't push controls off-screen.
    let cover_size = Length::Fixed(320.0);
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

    // Visualizer toggle button (cfg-gated)
    #[cfg(feature = "visualizer")]
    {
        let viz_icon = "applications-multimedia-symbolic";
        bottom_controls = bottom_controls.push(
            widget::button::icon(widget::icon::from_name(viz_icon).size(28))
                .on_press(NowPlayingMessage::ToggleVisualizer),
        );
        // Next preset button (only visible when visualizer is active)
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

    // --- Visualizer metadata overlay (frosted pill, top-left corner) ---
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
                            // Propagate opacity to all child text so text and
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

    // --- Main content column ---
    let content = widget::column()
        .push(top_bar)
        .push(widget::Space::new(Length::Shrink, 24.0))
        .push(hero_widget)
        .push(widget::Space::new(Length::Shrink, 24.0))
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

    let _ = content_opacity;

    // --- Assemble with Stack for background layering ---
    //
    // Stack renders children bottom-to-top: first child = bottom layer.
    // The first child also dictates intrinsic size.
    //
    // When a background image (blur or visualizer) is available, we use:
    //   Layer 0 (bottom): background image — Fill width, Fill height
    //   Layer 1 (middle): semi-transparent dark overlay
    //   Layer 2 (top):    content column with all the controls
    //
    // We set the Stack height to Fixed(800) — a generous value that
    // covers the content without using Length::Fill (which would panic
    // in COSMIC's scrollable wrapper). The content column inside uses
    // Shrink height so it naturally sizes to its children.

    // Determine whether the visualizer is driving the background.
    #[cfg(feature = "visualizer")]
    let viz_is_bg = visualizer_active;
    #[cfg(not(feature = "visualizer"))]
    let viz_is_bg = false;

    // Build the background layer.
    //
    // When the visualizer is active, use the Shader widget backed by a
    // persistent wgpu texture (no image::Handle churn, no flashing).
    // Otherwise fall back to the blurred cover art, or a themed solid.
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

    if let Some(bg_layer) = bg_element {
        // Solid black base layer — safety net so nothing shows through
        // during the very first frame before the shader texture is uploaded.
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

        // Dark overlay for text legibility.
        // When the visualizer is the background, use a lighter overlay so
        // the animation is visible; for static blurred art use a heavier one.
        let overlay_alpha = if viz_is_bg { 0.35 } else { 0.65 };
        let overlay: cosmic::Element<'_, NowPlayingMessage> =
            widget::container(widget::Space::new(0, 0))
                .width(Length::Fill)
                .height(Length::Fixed(800.0))
                .class(cosmic::theme::Container::custom(move |_theme| {
                    cosmic::iced::widget::container::Style {
                        background: Some(Color::from_rgba(0.0, 0.0, 0.0, overlay_alpha).into()),
                        ..Default::default()
                    }
                }))
                .into();

        // Stack: black base → background (shader or blur) → overlay → content → [viz metadata overlay]
        #[allow(unused_mut)]
        let mut stack_widget = Stack::new()
            .push(black_base)
            .push(bg_layer)
            .push(overlay)
            .push(content);

        #[cfg(feature = "visualizer")]
        if let Some(meta_overlay) = viz_metadata_overlay {
            stack_widget = stack_widget.push(meta_overlay);
        }

        let stack: cosmic::Element<'_, NowPlayingMessage> = stack_widget.width(Length::Fill).into();

        stack
    } else {
        // No background image — use solid themed background
        let styled_content = widget::container(content)
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
            }));

        styled_content.into()
    }
}
