// SPDX-License-Identifier: GPL-3.0

use crate::library::{Playlist, Track};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Fixed width (px) for the trailing action-button cluster in every track row.
/// The header row carries a spacer of exactly this width so that the
/// FillPortion columns (Title / Artist / Album) compute identically in both
/// the header and the data rows, giving perfect column alignment.
///
/// Breakdown (with row spacing = 4 inside the cluster):
///   heart  (icon 16) ≈ 32 px
///   5 stars (icon 14, spacing 0) ≈ 5 × 28 = 140 px
///   playlist (icon 16) ≈ 32 px
///   2 × 4 px gaps = 8 px
///   total ≈ 212 px
const ACTIONS_WIDTH: f32 = 212.0;

#[derive(Debug, Clone)]
pub enum SongMessage {
    PlayTrack(usize),
    SortBy(SortField),
    ToggleFavorite(String),
    SetRating(String, u8),
    AddToPlaylist(String, String),
    ToggleFavoritesFilter,
    FilterByGenre(String),
    ClearGenreFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Title,
    Artist,
    Album,
    Duration,
}

pub fn songs_list_view<'a>(
    tracks: &'a [Track],
    current_sort: SortField,
    sort_descending: bool,
    favorites_filter: bool,
    genre_filter: Option<&'a str>,
    playlists: &'a [Playlist],
    current_track_id: Option<i64>,
) -> cosmic::Element<'a, SongMessage> {
    let filtered: Vec<(usize, &Track)> = tracks
        .iter()
        .enumerate()
        .filter(|(_i, t)| {
            if favorites_filter && !t.is_favorite {
                return false;
            }
            if let Some(genre) = genre_filter
                && !t.genre.eq_ignore_ascii_case(genre)
            {
                return false;
            }
            true
        })
        .collect();

    if tracks.is_empty() {
        return widget::container(
            widget::column()
                .push(widget::icon::from_name("audio-x-generic-symbolic").size(64))
                .push(widget::text::title3("No songs found"))
                .spacing(12)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into();
    }

    // --- Filter bar: Favorites toggle + genre filter indicator + track count ---
    let mut filter_bar = widget::row().spacing(8).align_y(Alignment::Center);

    let fav_icon = if favorites_filter {
        "emblem-favorite-symbolic"
    } else {
        "non-starred-symbolic"
    };
    let fav_button = widget::button::custom(
        widget::row()
            .push(widget::icon::from_name(fav_icon).size(16))
            .push(widget::text::body("Favorites"))
            .spacing(4)
            .align_y(Alignment::Center),
    )
    .on_press(SongMessage::ToggleFavoritesFilter)
    .class(if favorites_filter {
        cosmic::theme::Button::Suggested
    } else {
        cosmic::theme::Button::Standard
    });
    filter_bar = filter_bar.push(fav_button);

    if let Some(genre) = genre_filter {
        let genre_chip = widget::button::custom(
            widget::row()
                .push(widget::text::caption(genre))
                .push(widget::icon::from_name("window-close-symbolic").size(12))
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .on_press(SongMessage::ClearGenreFilter)
        .class(cosmic::theme::Button::Suggested);
        filter_bar = filter_bar.push(genre_chip);
    }

    // Track count badge (right-aligned)
    filter_bar = filter_bar.push(
        widget::container(widget::text::caption(format!("{} tracks", filtered.len())))
            .align_x(Horizontal::Right)
            .width(Length::Fill),
    );

    // --- Column headers ---
    // The trailing Space matches ACTIONS_WIDTH so that FillPortion columns
    // resolve to the same pixel widths as in the data rows below.
    let header = widget::row()
        .push(widget::Space::with_width(40))
        .push(
            widget::button::custom(widget::text(sort_label(
                "Title",
                SortField::Title,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Title))
            .width(Length::FillPortion(3))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Artist",
                SortField::Artist,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Artist))
            .width(Length::FillPortion(2))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Album",
                SortField::Album,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Album))
            .width(Length::FillPortion(2))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Duration",
                SortField::Duration,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Duration))
            .width(64)
            .class(cosmic::theme::Button::Text),
        )
        // Spacer that mirrors the fixed-width action cluster in data rows.
        .push(widget::Space::with_width(ACTIONS_WIDTH))
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([4, 8]);

    if filtered.is_empty() {
        let message = if favorites_filter {
            "No favorites yet — click the heart on any track to add one"
        } else {
            "No tracks match the current filter"
        };
        return widget::column()
            .push(filter_bar)
            .push(header)
            .push(widget::divider::horizontal::default())
            .push(
                widget::container(
                    widget::column()
                        .push(widget::icon::from_name("edit-find-symbolic").size(48))
                        .push(widget::text::title3(message))
                        .spacing(12)
                        .align_x(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
            )
            .padding(16)
            .spacing(4)
            .into();
    }

    let mut track_list = widget::column().spacing(2);

    for (original_index, track) in &filtered {
        let track_id = track.id.to_string();
        let is_playing = current_track_id == Some(track.id);

        let num_col: cosmic::Element<'_, SongMessage> = if is_playing {
            widget::icon::from_name("media-playback-start-symbolic")
                .size(14)
                .into()
        } else {
            widget::text(format!("{}", original_index + 1)).into()
        };

        let fav_icon_name = if track.is_favorite {
            "emblem-favorite-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let heart_btn = widget::button::icon(widget::icon::from_name(fav_icon_name).size(16))
            .on_press(SongMessage::ToggleFavorite(track_id.clone()));

        let rating_row = star_rating_widget(track_id.clone(), track.rating);

        let playlist_btn: cosmic::Element<'_, SongMessage> = if !playlists.is_empty() {
            let source_uri = track.source_uri.clone();
            let pl_ids: Vec<String> = playlists.iter().map(|p| p.id.clone()).collect();
            playlist_dropdown_button(source_uri, &pl_ids)
        } else {
            widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
        };

        // Actions cluster — fixed width to match the header spacer above.
        let actions = widget::container(
            widget::row()
                .push(heart_btn)
                .push(rating_row)
                .push(playlist_btn)
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .width(Length::Fixed(ACTIONS_WIDTH));

        let row = widget::button::custom(
            widget::row()
                .push(
                    widget::container(num_col)
                        .width(40)
                        .align_x(Horizontal::Center),
                )
                .push(
                    widget::text(truncate_str(track.title.as_str(), 40))
                        .width(Length::FillPortion(3)),
                )
                .push(
                    widget::text(non_empty_or_dash(track.artist.as_str()))
                        .width(Length::FillPortion(2)),
                )
                .push(
                    widget::text(non_empty_or_dash(track.album.as_str()))
                        .width(Length::FillPortion(2)),
                )
                .push(widget::text(track.duration_string()).width(64))
                .push(actions)
                .spacing(8)
                .align_y(Alignment::Center)
                .padding([6, 8]),
        )
        .on_press(SongMessage::PlayTrack(*original_index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    widget::column()
        .push(filter_bar)
        .push(header)
        .push(widget::divider::horizontal::default())
        .push(
            widget::scrollable(widget::container(track_list).width(Length::Fill))
                .height(Length::Fill),
        )
        .padding(16)
        .spacing(4)
        .into()
}

pub fn star_rating_widget<'a>(
    track_id: String,
    current_rating: Option<u8>,
) -> cosmic::Element<'a, SongMessage> {
    let rating = current_rating.unwrap_or(0);
    let mut row = widget::row().spacing(0).align_y(Alignment::Center);
    for star in 1u8..=5 {
        let icon_name = if star <= rating {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let new_rating = if star == rating { 0 } else { star };
        let btn = widget::button::icon(widget::icon::from_name(icon_name).size(14))
            .on_press(SongMessage::SetRating(track_id.clone(), new_rating));
        row = row.push(btn);
    }
    row.into()
}

fn playlist_dropdown_button<'a>(
    source_uri: String,
    ids: &[String],
) -> cosmic::Element<'a, SongMessage> {
    if let Some(first_id) = ids.first() {
        widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16))
            .on_press(SongMessage::AddToPlaylist(source_uri, first_id.clone()))
            .into()
    } else {
        widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
    }
}

fn sort_label(name: &str, field: SortField, current: SortField, descending: bool) -> String {
    if field == current {
        let arrow = if descending { "▼" } else { "▲" };
        format!("{name} {arrow}")
    } else {
        name.to_string()
    }
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending "…" if
/// it was cut short.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}

/// Return `s` unchanged if non-empty, otherwise an em-dash placeholder.
fn non_empty_or_dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}
