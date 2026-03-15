// SPDX-License-Identifier: GPL-3.0

//! Expanded now-playing view — full-screen overlay with centered cover art,
//! blurred background, and transparent controls overlay.

use super::{NowPlayingMessage, format_time, truncate_str};
use crate::config::RepeatMode;
use crate::library::Track;
use crate::player::PlaybackState;
use crate::views::spacing;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::widget::Stack;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

#[cfg(feature = "visualizer")]
use std::sync::{Arc, Mutex};

/// Maximum width for the centered content column (cover + controls).
const CONTENT_MAX_WIDTH: f32 = 480.0;
/// Cover art display size.
const COVER_ART_SIZE: f32 = 320.0;

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
    let _ = expand_progress;

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

    // ── Icons ─────────────────────────────────────────────────────────────
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

    // ── Build centered content column ─────────────────────────────────────
    let mut content_col = widget::column()
        .align_x(Alignment::Center)
        .width(Length::Fill);

    // Top bar: collapse button right-aligned
    let collapse_btn = widget::button::icon(widget::icon::from_name("go-down-symbolic").size(24))
        .on_press(NowPlayingMessage::Collapse);

    let top_bar = widget::row()
        .push(widget::Space::new(Length::Fill, Length::Shrink))
        .push(collapse_btn)
        .align_y(Alignment::Center);
    content_col = content_col.push(top_bar);

    // Flex spacer (pushes cover art toward center)
    content_col = content_col.push(widget::Space::new(Length::Shrink, Length::Fill));

    // ── Cover art (centered, with rounded corners + shadow) ─────────────────
    let cover_widget: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
        widget::container(
            widget::icon::icon(handle.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(Length::Fixed(COVER_ART_SIZE))
                .height(Length::Fixed(COVER_ART_SIZE)),
        )
        .width(Length::Fixed(COVER_ART_SIZE))
        .height(Length::Fixed(COVER_ART_SIZE))
        .class(cosmic::theme::Container::custom(|theme| {
            let cosmic = theme.cosmic();
            cosmic::iced::widget::container::Style {
                border: cosmic::iced::Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: cosmic.corner_radii.radius_m.into(),
                },
                shadow: cosmic::iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                    offset: cosmic::iced::Vector::new(0.0, 12.0),
                    blur_radius: 32.0,
                },
                ..Default::default()
            }
        }))
        .into()
    } else {
        widget::container(widget::icon::from_name("media-optical-cd-audio-symbolic").size(120))
            .width(Length::Fixed(COVER_ART_SIZE))
            .height(Length::Fixed(COVER_ART_SIZE))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    };

    content_col = content_col.push(
        widget::container(cover_widget)
            .width(Length::Fill)
            .align_x(Horizontal::Center),
    );

    // Gap between cover and metadata
    content_col = content_col.push(widget::Space::new(
        Length::Shrink,
        Length::Fixed(spacing::M as f32),
    ));

    // ── Track metadata (centered) ─────────────────────────────────────────
    // White text style helpers (fn pointers, no captures)
    fn white_text(_: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
        cosmic::iced::widget::text::Style {
            color: Some(Color::WHITE),
        }
    }
    fn dim_white_text(_: &cosmic::Theme) -> cosmic::iced::widget::text::Style {
        cosmic::iced::widget::text::Style {
            color: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
        }
    }

    if let Some(track) = current_track {
        // Track title — large, white, centered
        content_col = content_col.push(
            widget::container(
                widget::text::title3(truncate_str(&track.title, 50))
                    .class(cosmic::theme::Text::Custom(white_text)),
            )
            .width(Length::Fill)
            .align_x(Horizontal::Center),
        );

        // Artist — medium, white, centered
        if !track.artist.is_empty() {
            content_col = content_col.push(
                widget::container(
                    widget::text::body(&track.artist)
                        .class(cosmic::theme::Text::Custom(white_text)),
                )
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            );
        }

        // Album + year — smaller, dimmed, centered
        if !track.album.is_empty() {
            let album_text = if track.year > 0 {
                format!("{} \u{2022} {}", track.album, track.year)
            } else {
                track.album.clone()
            };
            content_col = content_col.push(
                widget::container(
                    widget::text::caption(album_text)
                        .class(cosmic::theme::Text::Custom(dim_white_text)),
                )
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            );
        } else if track.year > 0 {
            content_col = content_col.push(
                widget::container(
                    widget::text::caption(track.year.to_string())
                        .class(cosmic::theme::Text::Custom(dim_white_text)),
                )
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            );
        }

        // Favorite button
        let fav_icon_name = if track.is_favorite {
            "emblem-favorite-symbolic"
        } else {
            "non-starred-symbolic"
        };
        content_col = content_col.push(
            widget::container(
                widget::button::icon(widget::icon::from_name(fav_icon_name).size(24))
                    .on_press(NowPlayingMessage::ToggleFavorite(track.id.to_string())),
            )
            .width(Length::Fill)
            .align_x(Horizontal::Center),
        );
    } else {
        content_col = content_col.push(
            widget::container(
                widget::text::title3("No track playing")
                    .class(cosmic::theme::Text::Custom(white_text)),
            )
            .width(Length::Fill)
            .align_x(Horizontal::Center),
        );
    }

    // Gap between metadata and seek bar
    content_col = content_col.push(widget::Space::new(
        Length::Shrink,
        Length::Fixed(spacing::M as f32),
    ));

    // ── Seek bar ──────────────────────────────────────────────────────────
    let seek_bar = widget::row()
        .push(
            widget::text::caption(format_time(display_position))
                .class(cosmic::theme::Text::Custom(dim_white_text)),
        )
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .step(0.001)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::Fill),
        )
        .push(
            widget::text::caption(format_time(duration))
                .class(cosmic::theme::Text::Custom(dim_white_text)),
        )
        .spacing(spacing::XS)
        .align_y(Alignment::Center);
    content_col = content_col.push(seek_bar);

    // Gap
    content_col = content_col.push(widget::Space::new(
        Length::Shrink,
        Length::Fixed(spacing::XS as f32),
    ));

    // ── Transport controls (centered, large) ───────────────────────────────
    let transport = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name(shuffle_icon))
                .medium()
                .on_press(NowPlayingMessage::ToggleShuffle),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic"))
                .large()
                .on_press(NowPlayingMessage::Previous),
        )
        .push(
            widget::button::icon(widget::icon::from_name(play_icon))
                .extra_large()
                .on_press(NowPlayingMessage::TogglePlayback),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic"))
                .large()
                .on_press(NowPlayingMessage::Next),
        )
        .push(
            widget::button::icon(widget::icon::from_name(repeat_icon))
                .medium()
                .on_press(NowPlayingMessage::CycleRepeat),
        )
        .spacing(spacing::M)
        .align_y(Alignment::Center);

    content_col = content_col.push(
        widget::container(transport)
            .width(Length::Fill)
            .align_x(Horizontal::Center),
    );

    // Gap
    content_col = content_col.push(widget::Space::new(
        Length::Shrink,
        Length::Fixed(spacing::S as f32),
    ));

    // ── Volume + extras row ───────────────────────────────────────────────
    #[allow(unused_mut)]
    let mut bottom_row = widget::row()
        .push(widget::icon::from_name("audio-volume-high-symbolic").size(20))
        .push(
            widget::slider(0.0..=1.0, volume, NowPlayingMessage::SetVolume)
                .step(0.01)
                .width(Length::Fill),
        )
        .push(
            widget::button::icon(widget::icon::from_name("view-list-lyrics-symbolic").size(24))
                .on_press(NowPlayingMessage::ShowLyrics),
        );

    #[cfg(feature = "visualizer")]
    {
        let viz_icon = "applications-multimedia-symbolic";
        bottom_row = bottom_row.push(
            widget::button::icon(widget::icon::from_name(viz_icon).size(24))
                .on_press(NowPlayingMessage::ToggleVisualizer),
        );
        if visualizer_active {
            bottom_row = bottom_row.push(
                widget::button::icon(
                    widget::icon::from_name("media-skip-forward-symbolic").size(20),
                )
                .on_press(NowPlayingMessage::NextPreset),
            );
        }
    }

    content_col = content_col.push(bottom_row.spacing(spacing::XXS).align_y(Alignment::Center));

    // Bottom flex spacer
    content_col = content_col.push(widget::Space::new(Length::Shrink, Length::Fill));

    // ── Constrain content width and center ────────────────────────────────
    let centered_content = widget::container(content_col)
        .max_width(CONTENT_MAX_WIDTH)
        .height(Length::Fill)
        .padding([spacing::S, spacing::M, spacing::M, spacing::M]);

    let centered_row = widget::row()
        .push(widget::Space::new(Length::Fill, Length::Shrink))
        .push(centered_content)
        .push(widget::Space::new(Length::Fill, Length::Shrink))
        .height(Length::Fill)
        .width(Length::Fill);

    // ── Semi-transparent dark overlay for readability ─────────────────────
    let overlay_container: cosmic::Element<'_, NowPlayingMessage> = widget::container(centered_row)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(cosmic::theme::Container::custom(|_| {
            cosmic::iced::widget::container::Style {
                background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()),
                text_color: Some(Color::WHITE),
                ..Default::default()
            }
        }))
        .into();

    // ── Visualizer overlay (when active) ──────────────────────────────────
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
                    .spacing(spacing::XXXS);

                let overlay_pill = widget::container(overlay_col)
                    .padding([spacing::XS, spacing::S])
                    .class(cosmic::theme::Container::custom(move |theme| {
                        let cosmic = theme.cosmic();
                        let mut bg: Color = cosmic.background.base.into();
                        bg.a = 0.65 * viz_metadata_opacity;
                        let mut fg: Color = cosmic.on_bg_color().into();
                        fg.a = viz_metadata_opacity;
                        cosmic::iced::widget::container::Style {
                            background: Some(bg.into()),
                            text_color: Some(fg),
                            border: cosmic::iced::Border {
                                radius: cosmic.corner_radii.radius_m.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }));

                let positioned = widget::container(overlay_pill)
                    .padding([spacing::M, 0, 0, spacing::M])
                    .width(Length::Fill)
                    .height(Length::Fill);

                Some(positioned.into())
            } else {
                None
            }
        } else {
            None
        };

    // ── Background layer ──────────────────────────────────────────────────
    #[cfg(feature = "visualizer")]
    let bg_element: Option<cosmic::Element<'_, NowPlayingMessage>> = if visualizer_active {
        let shader = cosmic::iced::widget::Shader::new(super::viz_shader::VizProgram::new(
            Arc::clone(&viz_frame_buf),
        ))
        .width(Length::Fill)
        .height(Length::Fill);
        Some(shader.into())
    } else {
        blurred_cover.map(|h| {
            widget::icon::icon(h.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        })
    };

    #[cfg(not(feature = "visualizer"))]
    let bg_element: Option<cosmic::Element<'_, NowPlayingMessage>> = blurred_cover.map(|h| {
        widget::icon::icon(h.clone())
            .content_fit(cosmic::iced::ContentFit::Cover)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    });

    // ── Assemble stack: base → blur → dark overlay → content ──────────────
    if let Some(bg_layer) = bg_element {
        let black_base: cosmic::Element<'_, NowPlayingMessage> =
            widget::container(widget::Space::new(0, 0))
                .width(Length::Fill)
                .height(Length::Fill)
                .class(cosmic::theme::Container::custom(|theme| {
                    let cosmic = theme.cosmic();
                    let base: Color = cosmic.background.base.into();
                    cosmic::iced::widget::container::Style {
                        background: Some(base.into()),
                        ..Default::default()
                    }
                }))
                .into();

        #[allow(unused_mut)]
        let mut stack_widget = Stack::new()
            .push(black_base)
            .push(bg_layer)
            .push(overlay_container);

        #[cfg(feature = "visualizer")]
        if let Some(meta_overlay) = viz_metadata_overlay {
            stack_widget = stack_widget.push(meta_overlay);
        }

        #[cfg(feature = "visualizer")]
        if visualizer_active {
            let dbl_click_cap: cosmic::Element<'_, NowPlayingMessage> = widget::mouse_area(
                widget::container(widget::Space::new(Length::Fill, Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_double_press(NowPlayingMessage::ToggleVizFullscreen)
            .into();
            stack_widget = stack_widget.push(dbl_click_cap);
        }

        stack_widget.width(Length::Fill).into()
    } else {
        // No blurred cover — just dark background + content
        widget::container(overlay_container)
            .width(Length::Fill)
            .align_x(Horizontal::Center)
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                let base: Color = cosmic.background.base.into();
                cosmic::iced::widget::container::Style {
                    background: Some(base.into()),
                    ..Default::default()
                }
            }))
            .into()
    }
}
