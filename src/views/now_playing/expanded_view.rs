// SPDX-License-Identifier: GPL-3.0

use super::{NowPlayingMessage, format_time, truncate_str};
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

    let cover_col: cosmic::Element<'_, NowPlayingMessage> = if let Some(handle) = cover_art {
        widget::container(
            widget::icon::icon(handle.clone())
                .content_fit(cosmic::iced::ContentFit::Cover)
                .width(Length::Fill)
                .height(Length::Fill),
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
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    offset: cosmic::iced::Vector::new(0.0, 8.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            }
        }))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        widget::container(widget::icon::from_name("media-optical-cd-audio-symbolic").size(160))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .class(cosmic::theme::Container::custom(|_| {
                cosmic::iced::widget::container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.25).into()),
                    ..Default::default()
                }
            }))
            .into()
    };

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

    let collapse_btn = widget::button::icon(widget::icon::from_name("go-down-symbolic").size(24))
        .on_press(NowPlayingMessage::Collapse);

    let mut right_col = widget::Column::new().spacing(0);

    let top_bar = widget::Row::new()
        .push(widget::Space::new().width(Length::Fill).height(Length::Shrink))
        .push(collapse_btn)
        .align_y(Alignment::Center);
    right_col = right_col.push(top_bar);

    right_col = right_col.push(widget::Space::new().width(Length::Shrink).height(Length::Fill));

    if let Some(track) = current_track {
        right_col = right_col
            .push(widget::text::title2(truncate_str(&track.title, 40)))
            .push(widget::Space::new().width(Length::Shrink).height(Length::Fixed(6.0)));

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
        if !sub_parts.is_empty() {
            right_col = right_col.push(widget::text::body(sub_parts.join(" \u{2022} ")).class(
                cosmic::theme::Text::Custom(|_theme| cosmic::iced::widget::text::Style {
                    color: Some(cosmic::iced::Color::from_rgba(1.0, 1.0, 1.0, 0.7)),
                    ..Default::default()
                }),
            ));
        }
    } else {
        right_col = right_col.push(widget::text::title2("No track playing"));
    }

    right_col = right_col.push(widget::Space::new().width(Length::Shrink).height(Length::Fixed(24.0)));

    let seek_bar = widget::Row::new()
        .push(widget::text::body(format_time(display_position)))
        .push(
            widget::slider(0.0..=1.0, progress, NowPlayingMessage::SeekPreview)
                .step(0.001)
                .on_release(NowPlayingMessage::SeekCommit)
                .width(Length::Fill),
        )
        .push(widget::text::body(format_time(duration)))
        .spacing(12)
        .align_y(Alignment::Center);
    right_col = right_col.push(seek_bar);

    right_col = right_col.push(widget::Space::new().width(Length::Shrink).height(Length::Fixed(16.0)));

    let transport = widget::Row::new()
        .push(
            widget::button::icon(widget::icon::from_name(shuffle_icon).size(24))
                .on_press(NowPlayingMessage::ToggleShuffle),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-backward-symbolic").size(28))
                .on_press(NowPlayingMessage::Previous),
        )
        .push(
            widget::button::icon(widget::icon::from_name(play_icon).size(36))
                .on_press(NowPlayingMessage::TogglePlayback),
        )
        .push(
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic").size(28))
                .on_press(NowPlayingMessage::Next),
        )
        .push(
            widget::button::icon(widget::icon::from_name(repeat_icon).size(24))
                .on_press(NowPlayingMessage::CycleRepeat),
        )
        .spacing(8)
        .align_y(Alignment::Center);

    right_col = right_col.push(
        widget::container(transport)
            .align_x(Horizontal::Center)
            .width(Length::Fill),
    );

    right_col = right_col.push(widget::Space::new().width(Length::Shrink).height(Length::Fixed(16.0)));

    #[allow(unused_mut)]
    let mut bottom_row = widget::Row::new()
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

    right_col = right_col.push(bottom_row.spacing(10).align_y(Alignment::Center));

    right_col = right_col.push(widget::Space::new().width(Length::Shrink).height(Length::Fill));

    let right_panel = widget::container(right_col.width(Length::Fill))
        .padding([20, 28, 28, 28])
        .width(Length::FillPortion(1))
        .height(Length::Fixed(800.0))
        .class(cosmic::theme::Container::custom(|_| {
            cosmic::iced::widget::container::Style {
                background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.72).into()),
                text_color: Some(Color::WHITE),
                ..Default::default()
            }
        }));

    let left_panel = widget::container(cover_col)
        .width(Length::FillPortion(1))
        .height(Length::Fixed(800.0));

    let two_col = widget::Row::new()
        .push(left_panel)
        .push(right_panel)
        .height(Length::Fixed(800.0))
        .width(Length::Fill);

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

                let overlay_col = widget::Column::new()
                    .push(widget::text::title3(truncate_str(&track.title, 40)))
                    .push(
                        widget::text::body(subtitle).class(cosmic::theme::Text::Custom(|theme| {
                            cosmic::iced::widget::text::Style {
                                color: Some(theme.cosmic().palette.neutral_7.into()),
                                ..Default::default()
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

    #[cfg(feature = "visualizer")]
    let bg_element: Option<cosmic::Element<'_, NowPlayingMessage>> = if visualizer_active {
        let shader = cosmic::iced::widget::Shader::new(super::viz_shader::VizProgram::new(
            Arc::clone(&viz_frame_buf),
        ))
        .width(Length::Fill)
        .height(Length::Fixed(800.0));
        Some(shader.into())
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
        let black_base: cosmic::Element<'_, NowPlayingMessage> =
            widget::container(widget::Space::new().width(0).height(0))
                .width(Length::Fill)
                .height(Length::Fixed(800.0))
                .class(cosmic::theme::Container::custom(|_theme| {
                    cosmic::iced::widget::container::Style {
                        background: Some(Color::BLACK.into()),
                        ..Default::default()
                    }
                }))
                .into();

        #[allow(unused_mut)]
        let mut stack_widget = Stack::new().push(black_base).push(bg_layer).push(two_col);

        #[cfg(feature = "visualizer")]
        if let Some(meta_overlay) = viz_metadata_overlay {
            stack_widget = stack_widget.push(meta_overlay);
        }

        #[cfg(feature = "visualizer")]
        if visualizer_active {
            let dbl_click_cap: cosmic::Element<'_, NowPlayingMessage> = widget::mouse_area(
                widget::container(widget::Space::new().width(Length::Fill).height(Length::Fixed(800.0)))
                    .width(Length::Fill)
                    .height(Length::Fixed(800.0)),
            )
            .on_double_press(NowPlayingMessage::ToggleVizFullscreen)
            .into();
            stack_widget = stack_widget.push(dbl_click_cap);
        }

        stack_widget.width(Length::Fill).into()
    } else {
        widget::container(two_col)
            .width(Length::Fill)
            .align_x(Horizontal::Center)
            .class(cosmic::theme::Container::custom(|theme| {
                let cosmic = theme.cosmic();
                cosmic::iced::widget::container::Style {
                    background: Some(
                        Color::from_rgba(
                            cosmic.background(false).base.red,
                            cosmic.background(false).base.green,
                            cosmic.background(false).base.blue,
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
