// SPDX-License-Identifier: GPL-3.0

//! Lyrics display view (context drawer panel).
//!
//! Tasks 104-105: Synced lyrics rendering with highlighted current line.

use crate::library::{LyricLine, Lyrics};
use crate::views::spacing;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;
use std::time::Duration;

/// Messages from the lyrics view.
#[derive(Debug, Clone)]
pub enum LyricsMessage {
    FetchLyrics,
    Close,
}

/// Find the index of the current lyric line based on playback position.
///
/// Returns the index of the last line whose timestamp is <= current position,
/// or `None` if no line has started yet.
fn find_current_line_index(lines: &[LyricLine], position: Duration) -> Option<usize> {
    let pos_ms = position.as_millis() as u64;
    let mut current = None;
    for (i, line) in lines.iter().enumerate() {
        if line.timestamp_ms <= pos_ms {
            current = Some(i);
        } else {
            break;
        }
    }
    current
}

/// Render the lyrics panel (shown in the context drawer).
///
/// Task 104-105: `playback_position` drives synced lyrics highlighting.
pub fn lyrics_view<'a>(
    lyrics: Option<&'a Lyrics>,
    track_title: &'a str,
    track_artist: &'a str,
    is_loading: bool,
    playback_position: Duration,
) -> cosmic::Element<'a, LyricsMessage> {
    let header = widget::column()
        .push(widget::text::title4(track_title))
        .push(widget::text::caption(track_artist))
        .spacing(spacing::XXXS);

    let content: cosmic::Element<'_, LyricsMessage> = if is_loading {
        widget::container(widget::text("Loading lyrics..."))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    } else if let Some(lyrics_data) = lyrics {
        match lyrics_data {
            Lyrics::Synced(lines) => {
                let current_idx = find_current_line_index(lines, playback_position);
                let mut col = widget::column().spacing(spacing::XXXS);
                for (i, line) in lines.iter().enumerate() {
                    let is_current = current_idx == Some(i);
                    col = col.push(synced_line_widget(line, is_current));
                }
                widget::scrollable(widget::container(col).padding(spacing::XXS))
                    .height(Length::Fill)
                    .into()
            }
            Lyrics::Unsynced(text) => widget::scrollable(
                widget::container(widget::text(text.as_str())).padding(spacing::XXS),
            )
            .height(Length::Fill)
            .into(),
        }
    } else {
        widget::container(
            widget::column()
                .push(widget::text("No lyrics available"))
                .push(
                    widget::button::suggested("Search Online").on_press(LyricsMessage::FetchLyrics),
                )
                .spacing(spacing::XXS)
                .align_x(Alignment::Center),
        )
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into()
    };

    widget::column()
        .push(header)
        .push(widget::divider::horizontal::default())
        .push(content)
        .spacing(spacing::XS)
        .padding(spacing::S)
        .width(Length::Fill)
        .height(Length::Shrink)
        .into()
}

/// Render a single synced lyric line with a timestamp prefix.
///
/// The current line uses the COSMIC accent color; others use the theme's
/// neutral_7 (dimmed) color. This ensures proper adaptation to light/dark themes.
fn synced_line_widget(line: &LyricLine, is_current: bool) -> cosmic::Element<'_, LyricsMessage> {
    let total_secs = line.timestamp_ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    let timestamp = format!("[{mins:02}:{secs:02}]");

    let text_class = if is_current {
        cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
            color: Some(theme.cosmic().accent_color().into()),
        })
    } else {
        cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
            color: Some(theme.cosmic().palette.neutral_7.into()),
        })
    };

    let line_text: cosmic::Element<'_, LyricsMessage> = if is_current {
        widget::text::body(&line.text).class(text_class).into()
    } else {
        widget::text(&line.text).class(text_class).into()
    };

    widget::row()
        .push(widget::text::caption(timestamp).class(if is_current {
            cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                color: Some(theme.cosmic().accent_color().into()),
            })
        } else {
            cosmic::theme::Text::Custom(|theme| cosmic::iced::widget::text::Style {
                color: Some(theme.cosmic().palette.neutral_7.into()),
            })
        }))
        .push(line_text)
        .spacing(spacing::XXS)
        .align_y(Alignment::Center)
        .into()
}
