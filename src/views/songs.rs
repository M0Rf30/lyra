// SPDX-License-Identifier: GPL-3.0

//! Songs list view - flat track listing with sorting options.

use crate::library::{Playlist, Track};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the songs view.
#[derive(Debug, Clone)]
pub enum SongMessage {
    PlayTrack(usize),
    SortBy(SortField),
    /// Toggle favorite status for a track (track ID as string).
    ToggleFavorite(String),
    /// Set rating (1-5) for a track. 0 clears the rating.
    SetRating(String, u8),
    /// Add track to playlist (track source_uri, playlist ID).
    AddToPlaylist(String, String),
    /// Toggle the favorites-only filter.
    ToggleFavoritesFilter,
    /// Filter by genre.
    FilterByGenre(String),
    /// Clear the active genre filter.
    ClearGenreFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Title,
    Artist,
    Album,
    Duration,
}

/// Render the songs list view with column headers.
pub fn songs_list_view<'a>(
    tracks: &'a [Track],
    current_sort: SortField,
    favorites_filter: bool,
    genre_filter: Option<&'a str>,
    playlists: &'a [Playlist],
) -> cosmic::Element<'a, SongMessage> {
    // Apply filters
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

    // Filter bar: Favorites toggle + genre filter indicator
    let mut filter_bar = widget::row().spacing(8).align_y(Alignment::Center);

    // Task 106: Favorites toggle button
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

    // Genre filter chip (shown when active)
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

    // Show count
    filter_bar = filter_bar
        .push(widget::text::caption(format!("{} tracks", filtered.len())).width(Length::Fill));

    // Column headers
    let header = widget::row()
        .push(widget::text("#").width(40))
        .push(
            widget::button::custom(widget::text(sort_label(
                "Title",
                SortField::Title,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Title))
            .width(Length::Fill)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Artist",
                SortField::Artist,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Artist))
            .width(200)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Album",
                SortField::Album,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Album))
            .width(200)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Duration",
                SortField::Duration,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Duration))
            .width(80)
            .class(cosmic::theme::Button::Text),
        )
        // Space for favorite + rating + genre + playlist action columns
        .push(widget::Space::with_width(180))
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([4, 8]);

    let mut track_list = widget::column().spacing(1);

    for (original_index, track) in &filtered {
        let track_id = track.id.to_string();

        // Task 99: Heart icon toggle
        let fav_icon_name = if track.is_favorite {
            "emblem-favorite-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let heart_btn = widget::button::icon(widget::icon::from_name(fav_icon_name).size(16))
            .on_press(SongMessage::ToggleFavorite(track_id.clone()));

        // Task 101: Star rating widget (1-5 stars)
        let rating_row = star_rating_widget(track_id.clone(), track.rating);

        // Task 103: Genre chip
        let genre_widget: cosmic::Element<'_, SongMessage> = if !track.genre.is_empty() {
            widget::button::custom(widget::text::caption(&track.genre))
                .on_press(SongMessage::FilterByGenre(track.genre.clone()))
                .class(cosmic::theme::Button::Standard)
                .into()
        } else {
            widget::Space::with_width(0).into()
        };

        // Task 98: Add to playlist button
        let playlist_btn: cosmic::Element<'_, SongMessage> = if !playlists.is_empty() {
            let source_uri = track.source_uri.clone();
            let items: Vec<String> = playlists.iter().map(|p| p.name.clone()).collect();
            let pl_ids: Vec<String> = playlists.iter().map(|p| p.id.clone()).collect();
            playlist_dropdown_button(source_uri, &items, &pl_ids)
        } else {
            widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
        };

        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", original_index + 1)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.album.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(80))
                .push(heart_btn)
                .push(rating_row)
                .push(genre_widget)
                .push(playlist_btn)
                .spacing(8)
                .align_y(Alignment::Center)
                .padding([4, 8]),
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

/// Render a 1-5 star rating widget.
///
/// Shows 5 star icons: filled for rated stars, outlined for unrated.
/// Clicking a star sets the rating; clicking the current rating clears it.
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
        // Clicking the current rating value clears it (sets to 0).
        let new_rating = if star == rating { 0 } else { star };
        let btn = widget::button::icon(widget::icon::from_name(icon_name).size(14))
            .on_press(SongMessage::SetRating(track_id.clone(), new_rating));
        row = row.push(btn);
    }

    row.into()
}

/// Simple "Add to Playlist" button.
///
/// Since cosmic doesn't have a simple dropdown/popover from a button, we
/// use the first playlist as a quick-add action. For a full picker, a context
/// drawer or separate dialog would be needed.
fn playlist_dropdown_button<'a>(
    source_uri: String,
    _names: &[String],
    ids: &[String],
) -> cosmic::Element<'a, SongMessage> {
    // Simplified approach: icon button that adds to the first playlist
    if let Some(first_id) = ids.first() {
        widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16))
            .on_press(SongMessage::AddToPlaylist(source_uri, first_id.clone()))
            .into()
    } else {
        widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
    }
}

fn sort_label(name: &str, field: SortField, current: SortField) -> String {
    if field == current {
        format!("{name} ^")
    } else {
        name.to_string()
    }
}
