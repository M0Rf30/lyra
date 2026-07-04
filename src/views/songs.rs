// SPDX-License-Identifier: GPL-3.0

use crate::library::{Playlist, Track};
use crate::views::common;
use cosmic::iced::alignment::Horizontal;
use cosmic::iced::{Alignment, Length, Size};
use cosmic::prelude::*;
use cosmic::widget;

/// Fixed width of the leading track-number / now-playing indicator column.
const NUM_WIDTH: f32 = 40.0;
/// Fixed width of the genre chip column.
const GENRE_WIDTH: f32 = 130.0;
/// Fixed width of the favorite-heart column.
const HEART_WIDTH: f32 = 32.0;
/// Fixed width of the star-rating column.
const RATING_WIDTH: f32 = 112.0;
/// Fixed width of the add-to-playlist column.
const ADD_WIDTH: f32 = 32.0;

/// Minimum responsive width (px) at which the Artist column appears.
const ARTIST_BREAKPOINT: f32 = 640.0;
/// Minimum responsive width (px) at which the Album and Rating columns appear.
const ALBUM_RATING_BREAKPOINT: f32 = 900.0;
/// Minimum responsive width (px) at which the Genre column appears.
const GENRE_BREAKPOINT: f32 = 1100.0;

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
    if tracks.is_empty() {
        return common::empty_state(
            "audio-x-generic-symbolic",
            "No songs found",
            "Scan your library from File > Rescan",
        );
    }

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

    // Filter bar: Favorites toggle + genre filter indicator + track count.
    let mut filter_bar = widget::Row::new().spacing(8).align_y(Alignment::Center);

    let fav_icon = if favorites_filter {
        "emblem-favorite-symbolic"
    } else {
        "non-starred-symbolic"
    };
    let fav_button = widget::button::custom(
        widget::Row::new()
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
            widget::Row::new()
                .push(widget::text::caption(genre))
                .push(widget::icon::from_name("window-close-symbolic").size(12))
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .on_press(SongMessage::ClearGenreFilter)
        .class(cosmic::theme::Button::Suggested);
        filter_bar = filter_bar.push(genre_chip);
    }

    filter_bar = filter_bar.push(
        common::cell_caption(format!(
            "{} track{}",
            filtered.len(),
            if filtered.len() == 1 { "" } else { "s" }
        ))
        .width(Length::Fill),
    );

    let table_area: cosmic::Element<'a, SongMessage> = if filtered.is_empty() {
        common::empty_state(
            "edit-find-symbolic",
            "No matching tracks",
            "Try clearing the favorites or genre filter",
        )
    } else {
        widget::responsive(move |size: Size| {
            let show_artist = size.width >= ARTIST_BREAKPOINT;
            let show_album = size.width >= ALBUM_RATING_BREAKPOINT;
            let show_rating = size.width >= ALBUM_RATING_BREAKPOINT;
            let show_genre = size.width >= GENRE_BREAKPOINT;

            let header = build_header(
                current_sort,
                sort_descending,
                show_artist,
                show_album,
                show_rating,
                show_genre,
            );

            let mut track_list = widget::Column::new().spacing(2);
            for &(original_index, track) in &filtered {
                let track_id = track.id.to_string();
                let is_playing = current_track_id == Some(track.id);
                track_list = track_list.push(build_row(
                    original_index,
                    track,
                    track_id,
                    is_playing,
                    playlists,
                    show_artist,
                    show_album,
                    show_rating,
                    show_genre,
                ));
            }

            widget::Column::new()
                .push(header)
                .push(widget::divider::horizontal::default())
                .push(
                    widget::scrollable(widget::container(track_list).width(Length::Fill))
                        .height(Length::Fill),
                )
                .spacing(4)
                .into()
        })
        .into()
    };

    widget::Column::new()
        .push(filter_bar)
        .push(table_area)
        .padding(16)
        .spacing(4)
        .into()
}

/// Build the column header row. Uses the exact same fixed widths /
/// `FillPortion`s as [`build_row`] so labels line up with their values.
fn build_header<'a>(
    current_sort: SortField,
    sort_descending: bool,
    show_artist: bool,
    show_album: bool,
    show_rating: bool,
    show_genre: bool,
) -> cosmic::Element<'a, SongMessage> {
    let mut row = widget::Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([4, 8]);

    row = row.push(widget::Space::new().width(NUM_WIDTH));

    row = row.push(
        widget::button::custom(common::cell_text(sort_label(
            "Title",
            SortField::Title,
            current_sort,
            sort_descending,
        )))
        .on_press(SongMessage::SortBy(SortField::Title))
        .width(Length::FillPortion(4))
        .class(cosmic::theme::Button::Text),
    );

    if show_artist {
        row = row.push(
            widget::button::custom(common::cell_text(sort_label(
                "Artist",
                SortField::Artist,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Artist))
            .width(Length::FillPortion(3))
            .class(cosmic::theme::Button::Text),
        );
    }

    if show_album {
        row = row.push(
            widget::button::custom(common::cell_text(sort_label(
                "Album",
                SortField::Album,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Album))
            .width(Length::FillPortion(3))
            .class(cosmic::theme::Button::Text),
        );
    }

    if show_genre {
        row = row.push(widget::Space::new().width(GENRE_WIDTH));
    }

    row = row.push(widget::Space::new().width(HEART_WIDTH));

    if show_rating {
        row = row.push(widget::Space::new().width(RATING_WIDTH));
    }

    row = row.push(widget::Space::new().width(ADD_WIDTH));

    row = row.push(
        widget::container(
            widget::button::custom(common::cell_text(sort_label(
                "Duration",
                SortField::Duration,
                current_sort,
                sort_descending,
            )))
            .on_press(SongMessage::SortBy(SortField::Duration))
            .class(cosmic::theme::Button::Text),
        )
        .width(common::DURATION_WIDTH)
        .align_x(Horizontal::Right),
    );

    row.into()
}

/// Build a single track row. Column widths mirror [`build_header`] exactly.
#[allow(clippy::too_many_arguments)]
fn build_row<'a>(
    original_index: usize,
    track: &'a Track,
    track_id: String,
    is_playing: bool,
    playlists: &'a [Playlist],
    show_artist: bool,
    show_album: bool,
    show_rating: bool,
    show_genre: bool,
) -> cosmic::Element<'a, SongMessage> {
    let num_col: cosmic::Element<'a, SongMessage> = if is_playing {
        widget::icon::from_name("media-playback-start-symbolic")
            .size(14)
            .into()
    } else {
        common::cell_text(format!("{}", original_index + 1)).into()
    };

    let mut row = widget::Row::new().spacing(8).align_y(Alignment::Center);

    row = row.push(
        widget::container(num_col)
            .width(NUM_WIDTH)
            .align_x(Horizontal::Center),
    );

    row = row.push(common::cell_text(track.title.as_str()).width(Length::FillPortion(4)));

    if show_artist {
        row = row.push(common::cell_text(track.artist.as_str()).width(Length::FillPortion(3)));
    }

    if show_album {
        row = row.push(common::cell_text(track.album.as_str()).width(Length::FillPortion(3)));
    }

    if show_genre {
        let genre_col: cosmic::Element<'a, SongMessage> = if track.genre.is_empty() {
            widget::Space::new().width(GENRE_WIDTH).into()
        } else {
            widget::container(
                widget::button::custom(common::cell_caption(track.genre.as_str()))
                    .on_press(SongMessage::FilterByGenre(track.genre.clone()))
                    .class(cosmic::theme::Button::Standard),
            )
            .width(GENRE_WIDTH)
            .into()
        };
        row = row.push(genre_col);
    }

    row = row.push(
        widget::container(common::favorite_button(
            track.is_favorite,
            SongMessage::ToggleFavorite(track_id.clone()),
        ))
        .width(HEART_WIDTH)
        .align_x(Horizontal::Center),
    );

    if show_rating {
        let rating_track_id = track_id.clone();
        row = row.push(
            widget::container(common::star_rating(track.rating, move |r| {
                SongMessage::SetRating(rating_track_id.clone(), r)
            }))
            .width(RATING_WIDTH)
            .align_x(Horizontal::Center),
        );
    }

    row = row.push(
        widget::container(playlist_dropdown_button(
            track.source_uri.clone(),
            playlists,
        ))
        .width(ADD_WIDTH)
        .align_x(Horizontal::Center),
    );

    row = row.push(common::duration_cell(track.duration.as_secs()));

    widget::button::custom(row.padding([6, 8]))
        .on_press(SongMessage::PlayTrack(original_index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text)
        .into()
}

/// Add-to-playlist button. Adds to the first playlist (existing behavior),
/// honestly labelled via tooltip with that playlist's name. Renders empty
/// space instead of a dead button when there are no playlists yet.
fn playlist_dropdown_button<'a>(
    source_uri: String,
    playlists: &[Playlist],
) -> cosmic::Element<'a, SongMessage> {
    if let Some(playlist) = playlists.first() {
        let button = widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16))
            .on_press(SongMessage::AddToPlaylist(source_uri, playlist.id.clone()));
        widget::tooltip(
            button,
            widget::text::caption(format!("Add to \"{}\"", playlist.name)),
            widget::tooltip::Position::Top,
        )
        .into()
    } else {
        widget::Space::new().width(ADD_WIDTH).into()
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
