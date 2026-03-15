// SPDX-License-Identifier: GPL-3.0

use crate::library::{Playlist, Track};
use crate::views::{empty_state, spacing};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

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
        return empty_state(
            "audio-x-generic-symbolic",
            "No songs found",
            "Add music directories in Settings to get started",
        );
    }

    // Filter bar: Favorites toggle + genre filter indicator
    let mut filter_bar = widget::row()
        .spacing(spacing::XXS)
        .align_y(Alignment::Center);

    let fav_icon = if favorites_filter {
        "emblem-favorite-symbolic"
    } else {
        "non-starred-symbolic"
    };
    let fav_button = widget::button::custom(
        widget::row()
            .push(widget::icon::from_name(fav_icon).size(16))
            .push(widget::text::body("Favorites"))
            .spacing(spacing::XXXS)
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
                .spacing(spacing::XXXS)
                .align_y(Alignment::Center),
        )
        .on_press(SongMessage::ClearGenreFilter)
        .class(cosmic::theme::Button::Suggested);
        filter_bar = filter_bar.push(genre_chip);
    }

    filter_bar = filter_bar
        .push(widget::text::caption(format!("{} tracks", filtered.len())).width(Length::Fill));

    // Column headers
    let header = widget::row()
        .push(widget::Space::with_width(40))
        .push(
            widget::button::custom(sort_header(
                "Title",
                SortField::Title,
                current_sort,
                sort_descending,
            ))
            .on_press(SongMessage::SortBy(SortField::Title))
            .width(Length::FillPortion(3))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(sort_header(
                "Artist",
                SortField::Artist,
                current_sort,
                sort_descending,
            ))
            .on_press(SongMessage::SortBy(SortField::Artist))
            .width(Length::FillPortion(2))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(sort_header(
                "Album",
                SortField::Album,
                current_sort,
                sort_descending,
            ))
            .on_press(SongMessage::SortBy(SortField::Album))
            .width(Length::FillPortion(2))
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(sort_header(
                "Duration",
                SortField::Duration,
                current_sort,
                sort_descending,
            ))
            .on_press(SongMessage::SortBy(SortField::Duration))
            .width(64)
            .class(cosmic::theme::Button::Text),
        )
        .push(widget::Space::with_width(Length::Shrink))
        .spacing(spacing::XXS)
        .align_y(Alignment::Center)
        .padding([spacing::XXXS, spacing::M]);

    if filtered.is_empty() {
        let message = if favorites_filter {
            "No favorites yet \u{2014} click the heart on any track to add one"
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
                        .spacing(spacing::XS)
                        .align_x(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
            )
            .padding(spacing::S)
            .spacing(spacing::XXXS)
            .into();
    }

    let mut track_list = cosmic::widget::list_column();

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

        let genre_widget: cosmic::Element<'_, SongMessage> = if !track.genre.is_empty() {
            widget::button::custom(widget::text::caption(&track.genre))
                .on_press(SongMessage::FilterByGenre(track.genre.clone()))
                .class(cosmic::theme::Button::Standard)
                .into()
        } else {
            widget::Space::with_width(0).into()
        };

        let playlist_btn: cosmic::Element<'_, SongMessage> = if !playlists.is_empty() {
            let source_uri = track.source_uri.clone();
            let pl_ids: Vec<String> = playlists.iter().map(|p| p.id.clone()).collect();
            playlist_dropdown_button(source_uri, &pl_ids)
        } else {
            widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
        };

        let row_content = widget::row()
            .push(
                widget::container(num_col)
                    .width(40)
                    .align_x(Horizontal::Center),
            )
            .push(widget::text(track.title.as_str()).width(Length::FillPortion(3)))
            .push(widget::text(track.artist.as_str()).width(Length::FillPortion(2)))
            .push(widget::text(track.album.as_str()).width(Length::FillPortion(2)))
            .push(widget::text(track.duration_string()).width(64))
            .push(heart_btn)
            .push(rating_row)
            .push(genre_widget)
            .push(playlist_btn)
            .spacing(spacing::XXS)
            .align_y(Alignment::Center);

        let row_btn = widget::button::custom(row_content)
            .on_press(SongMessage::PlayTrack(*original_index))
            .width(Length::Fill)
            .class(crate::views::card_button_class());

        track_list = track_list.add(row_btn);
    }

    widget::column()
        .push(filter_bar)
        .push(header)
        .push(widget::divider::horizontal::default())
        .push(widget::scrollable(track_list).height(Length::Fill))
        .padding(spacing::S)
        .spacing(spacing::XXXS)
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

fn sort_header<'a>(
    name: &'a str,
    field: SortField,
    current: SortField,
    descending: bool,
) -> cosmic::Element<'a, SongMessage> {
    if field == current {
        let icon_name = if descending {
            "pan-down-symbolic"
        } else {
            "pan-up-symbolic"
        };
        widget::row()
            .push(widget::text::heading(name))
            .push(widget::icon::from_name(icon_name).size(16))
            .spacing(spacing::XXXS)
            .align_y(Alignment::Center)
            .into()
    } else {
        widget::text::heading(name).into()
    }
}
