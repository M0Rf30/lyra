// SPDX-License-Identifier: GPL-3.0

//! Albums grid view - displays album covers in a responsive grid (Lollypop-style).

use crate::library::{Album, CoverArt, Playlist};
use crate::views::card_button_class;
use crate::views::common;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the album view.
#[derive(Debug, Clone)]
pub enum AlbumMessage {
    /// User clicked on an album (index in the albums list).
    SelectAlbum(usize),
    /// User wants to play the whole album.
    PlayAlbum(usize),
    /// User clicked a specific track within the album detail.
    PlayTrack(usize, usize),
    /// Go back to the grid from detail view.
    BackToGrid,
    /// Toggle favorite for a track (track ID as string).
    ToggleFavorite(String),
    /// Set rating for a track (track ID, rating 0-5).
    SetRating(String, u8),
    /// Filter by genre.
    FilterByGenre(String),
    /// Add track to playlist (source_uri, playlist_id).
    AddToPlaylist(String, String),
}

/// Render the album grid view.
pub fn album_grid_view<'a>(
    albums: &'a [Album],
    cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
) -> cosmic::Element<'a, AlbumMessage> {
    if albums.is_empty() {
        return common::empty_state(
            "folder-music-symbolic",
            "No albums found",
            "Add music directories in Settings to get started",
        );
    }

    let cards: Vec<cosmic::Element<'_, AlbumMessage>> = albums
        .iter()
        .enumerate()
        .map(|(index, album)| {
            let key = CoverArt::album_key(&album.artist, &album.name);
            let art_widget: cosmic::Element<'_, AlbumMessage> =
                if let Some(handle) = cover_images.get(&key) {
                    widget::icon::icon(handle.clone()).size(160).into()
                } else {
                    let placeholder_icon: cosmic::Element<'_, AlbumMessage> =
                        widget::icon::from_name("media-optical-cd-audio-symbolic")
                            .size(64)
                            .into();
                    widget::container(placeholder_icon)
                        .width(160)
                        .height(160)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center)
                        .class(cosmic::theme::Container::Card)
                        .into()
                };

            let album_card = widget::Column::new()
                .push(
                    widget::container(art_widget)
                        .width(160)
                        .height(160)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center),
                )
                .push(
                    widget::Column::new()
                        .push(common::cell_text(common::truncate_str(&album.name, 28)).width(160))
                        .push(
                            common::cell_caption(common::truncate_str(&album.artist, 28))
                                .width(160),
                        )
                        .spacing(2),
                )
                .spacing(8);

            widget::button::custom(album_card)
                .on_press(AlbumMessage::SelectAlbum(index))
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

pub fn album_detail_view<'a>(
    album: &'a Album,
    album_index: usize,
    cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
    playlists: &'a [Playlist],
    current_track_id: Option<i64>,
) -> cosmic::Element<'a, AlbumMessage> {
    let key = CoverArt::album_key(&album.artist, &album.name);
    let art_widget: cosmic::Element<'_, AlbumMessage> = if let Some(handle) = cover_images.get(&key)
    {
        widget::icon::icon(handle.clone()).size(160).into()
    } else {
        widget::icon::from_name("media-optical-cd-audio-symbolic")
            .size(120)
            .into()
    };

    // Task 103: Collect distinct genres from album tracks for header chips
    let genres: Vec<String> = {
        let mut g: Vec<String> = album
            .tracks
            .iter()
            .filter(|t| !t.genre.is_empty())
            .map(|t| t.genre.clone())
            .collect();
        g.sort();
        g.dedup();
        g
    };

    let track_count = album.track_count();
    let track_label = format!(
        "{track_count} track{}",
        if track_count == 1 { "" } else { "s" }
    );
    let duration_label = common::format_duration_coarse(album.total_duration().as_secs());

    let mut meta_col = widget::Column::new()
        .push(widget::text::title2(album.name.as_str()))
        .push(
            widget::text::body(album.artist.as_str()).class(cosmic::theme::Text::Custom(|theme| {
                cosmic::iced::widget::text::Style {
                    color: Some(theme.cosmic().palette.neutral_7.into()),
                    ..Default::default()
                }
            })),
        )
        .push(common::cell_caption(format!(
            "{track_label} \u{b7} {duration_label}"
        )))
        .push(
            widget::button::suggested("Play Album").on_press(AlbumMessage::PlayAlbum(album_index)),
        )
        .spacing(8);

    // Task 103: Genre chips in album header
    if !genres.is_empty() {
        let mut genre_row = widget::Row::new().spacing(4).align_y(Alignment::Center);
        for genre in genres {
            // Use the owned String for both the message and the label.
            let label = genre.clone();
            genre_row = genre_row.push(
                widget::button::custom(common::cell_caption(label))
                    .on_press(AlbumMessage::FilterByGenre(genre))
                    .class(cosmic::theme::Button::Standard),
            );
        }
        meta_col = meta_col.push(genre_row);
    }

    let header = widget::Row::new()
        .push(common::icon_button(
            "go-previous-symbolic",
            16,
            "Back to albums",
            AlbumMessage::BackToGrid,
        ))
        .push(
            widget::container(art_widget)
                .width(160)
                .height(160)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .push(meta_col)
        .spacing(16)
        .align_y(Alignment::Center);

    let mut track_list = widget::Column::new().spacing(2);

    for (track_idx, track) in album.tracks.iter().enumerate() {
        let track_id = track.id.to_string();
        let is_playing = current_track_id == Some(track.id);

        let num_col: cosmic::Element<'_, AlbumMessage> = if is_playing {
            widget::icon::from_name("media-playback-start-symbolic")
                .size(14)
                .into()
        } else {
            common::cell_text(format!("{}", track.track_number)).into()
        };

        let heart_btn = common::favorite_button(
            track.is_favorite,
            AlbumMessage::ToggleFavorite(track_id.clone()),
        );

        // Task 102: Star rating widget, fixed-width so columns stay aligned.
        let rating_row = widget::container(common::star_rating(track.rating, move |r| {
            AlbumMessage::SetRating(track_id.clone(), r)
        }))
        .width(112);

        // Task 103: Genre chip per track, fixed-width so columns stay aligned.
        let genre_widget: cosmic::Element<'_, AlbumMessage> = if !track.genre.is_empty() {
            widget::container(
                widget::button::custom(common::cell_caption(track.genre.as_str()))
                    .on_press(AlbumMessage::FilterByGenre(track.genre.clone()))
                    .class(cosmic::theme::Button::Standard),
            )
            .width(130)
            .into()
        } else {
            widget::Space::new().width(130).into()
        };

        // Task 98: Add to playlist button - honest about its destination, or
        // absent entirely when there is nowhere to add to.
        let playlist_btn: cosmic::Element<'_, AlbumMessage> =
            if let Some(first_pl) = playlists.first() {
                widget::tooltip(
                    widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16))
                        .on_press(AlbumMessage::AddToPlaylist(
                            track.source_uri.clone(),
                            first_pl.id.clone(),
                        )),
                    widget::text::caption(format!("Add to \"{}\"", first_pl.name)),
                    widget::tooltip::Position::Top,
                )
                .into()
            } else {
                widget::Space::new().width(32).height(32).into()
            };

        let row = widget::button::custom(
            widget::Row::new()
                .push(
                    widget::container(num_col)
                        .width(40)
                        .align_x(Horizontal::Center),
                )
                .push(common::cell_text(track.title.as_str()).width(Length::FillPortion(4)))
                .push(common::cell_text(track.artist.as_str()).width(Length::FillPortion(3)))
                .push(heart_btn)
                .push(rating_row)
                .push(genre_widget)
                .push(playlist_btn)
                .push(common::duration_cell(track.duration.as_secs()))
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(4),
        )
        .on_press(AlbumMessage::PlayTrack(album_index, track_idx))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    widget::scrollable(
        widget::Column::new()
            .push(header)
            .push(widget::divider::horizontal::default())
            .push(track_list)
            .spacing(16)
            .padding(16),
    )
    .height(Length::Fill)
    .into()
}
