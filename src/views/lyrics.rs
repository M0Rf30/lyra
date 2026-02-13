// SPDX-License-Identifier: GPL-3.0

//! Lyrics display view (context drawer panel).

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the lyrics view.
#[derive(Debug, Clone)]
pub enum LyricsMessage {
    FetchLyrics,
    Close,
}

/// Render the lyrics panel (shown in the context drawer).
pub fn lyrics_view<'a>(
    lyrics: Option<&'a str>,
    track_title: &'a str,
    track_artist: &'a str,
    is_loading: bool,
) -> cosmic::Element<'a, LyricsMessage> {
    let header = widget::column()
        .push(widget::text::title4(track_title))
        .push(widget::text::caption(track_artist))
        .spacing(4);

    let content: cosmic::Element<'_, LyricsMessage> = if is_loading {
        widget::container(widget::text("Loading lyrics..."))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into()
    } else if let Some(text) = lyrics {
        widget::scrollable(widget::container(widget::text(text)).padding(8))
            .height(Length::Fill)
            .into()
    } else {
        widget::container(
            widget::column()
                .push(widget::text("No lyrics available"))
                .push(
                    widget::button::suggested("Search Online").on_press(LyricsMessage::FetchLyrics),
                )
                .spacing(8)
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
        .spacing(12)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
