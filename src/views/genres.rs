// SPDX-License-Identifier: GPL-3.0

//! Genres view - grid of all genres; clicking one shows filtered tracks.

use crate::library::Track;
use crate::views::card_button_class;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the genres view.
#[derive(Debug, Clone)]
pub enum GenreMessage {
    /// User selected a genre to view its tracks.
    SelectGenre(usize),
    /// Go back to the genre grid from the filtered track list.
    BackToGrid,
    /// Play a specific track in the genre detail view.
    PlayTrack(usize),
}

/// Render the genre grid view.
pub fn genre_grid_view(genres: &[String]) -> cosmic::Element<'_, GenreMessage> {
    if genres.is_empty() {
        return widget::container(
            widget::column()
                .push(widget::icon::from_name("audio-x-generic-symbolic").size(64))
                .push(widget::text::title3("No genres found"))
                .spacing(12)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into();
    }

    let cards: Vec<cosmic::Element<'_, GenreMessage>> = genres
        .iter()
        .enumerate()
        .map(|(index, genre)| {
            let icon_name = genre_icon_name(genre);
            let genre_icon: cosmic::Element<'_, GenreMessage> =
                widget::icon::from_name(icon_name).size(48).into();

            let icon_container: cosmic::Element<'_, GenreMessage> = widget::container(genre_icon)
                .width(140)
                .height(100)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Card)
                .into();

            let card = widget::column()
                .push(icon_container)
                .push(
                    widget::text(truncate_str(genre, 22))
                        .width(140)
                        .align_x(Horizontal::Center),
                )
                .spacing(6)
                .align_x(Alignment::Center);

            widget::button::custom(card)
                .on_press(GenreMessage::SelectGenre(index))
                .padding(8)
                .class(card_button_class())
                .into()
        })
        .collect();

    let grid = widget::flex_row(cards)
        .column_spacing(20)
        .row_spacing(20)
        .width(Length::Fill);

    widget::scrollable(widget::container(grid).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// Render the detail view for a selected genre, showing filtered tracks.
pub fn genre_detail_view<'a>(
    genre_name: &'a str,
    tracks: &'a [Track],
) -> cosmic::Element<'a, GenreMessage> {
    let detail_icon: cosmic::Element<'_, GenreMessage> =
        widget::icon::from_name(genre_icon_name(genre_name))
            .size(64)
            .into();

    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(GenreMessage::BackToGrid),
        )
        .push(detail_icon)
        .push(
            widget::column()
                .push(widget::text::title1(genre_name))
                .push(widget::text::caption(format!("{} tracks", tracks.len())))
                .spacing(4),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut track_list = widget::column().spacing(1);

    for (index, track) in tracks.iter().enumerate() {
        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", index + 1)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.album.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(80))
                .spacing(8)
                .align_y(Alignment::Center)
                .padding([4, 8]),
        )
        .on_press(GenreMessage::PlayTrack(index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    if tracks.is_empty() {
        track_list = track_list
            .push(widget::container(widget::text("No tracks found for this genre.")).padding(16));
    }

    widget::scrollable(
        widget::column()
            .push(header)
            .push(widget::divider::horizontal::default())
            .push(track_list)
            .spacing(16)
            .padding(16),
    )
    .height(Length::Fill)
    .into()
}

fn genre_icon_name(genre: &str) -> &'static str {
    let lower = genre.to_lowercase();
    if lower.contains("rock") || lower.contains("metal") || lower.contains("punk") {
        "audio-x-generic-symbolic"
    } else if lower.contains("classic") || lower.contains("orchestra") || lower.contains("opera") {
        "media-optical-cd-audio-symbolic"
    } else if lower.contains("jazz") || lower.contains("blues") || lower.contains("soul") {
        "audio-card-symbolic"
    } else if lower.contains("electronic")
        || lower.contains("techno")
        || lower.contains("trance")
        || lower.contains("house")
        || lower.contains("edm")
        || lower.contains("synth")
    {
        "computer-symbolic"
    } else if lower.contains("folk") || lower.contains("country") || lower.contains("acoustic") {
        "emblem-music-symbolic"
    } else if lower.contains("hip") || lower.contains("rap") || lower.contains("r&b") {
        "media-record-symbolic"
    } else if lower.contains("pop") {
        "starred-symbolic"
    } else if lower.contains("ambient") || lower.contains("new age") || lower.contains("asmr") {
        "weather-clear-night-symbolic"
    } else if lower.contains("soundtrack") || lower.contains("score") || lower.contains("theme") {
        "applications-multimedia-symbolic"
    } else if lower.contains("reggae") || lower.contains("ska") {
        "weather-clear-symbolic"
    } else {
        "audio-x-generic-symbolic"
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}
