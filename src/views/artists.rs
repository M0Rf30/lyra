// SPDX-License-Identifier: GPL-3.0

//! Artists view - list of artists with album sub-views.

use crate::library::{Artist, CoverArt};
use crate::views::common;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the artist view.
#[derive(Debug, Clone)]
pub enum ArtistMessage {
    SelectArtist(usize),
    PlayArtistAlbum(usize, usize),
    PlayTrack(usize, usize, usize),
    BackToList,
    /// Toggle favorite for a track (track ID as string).
    ToggleFavorite(String),
    /// Set rating for a track (track ID, rating 0-5).
    SetRating(String, u8),
    /// Filter by genre.
    FilterByGenre(String),
}

/// Render the artist list.
pub fn artist_list_view<'a>(
    artists: &'a [Artist],
    artist_avatars: &'a std::collections::HashMap<String, widget::icon::Handle>,
) -> cosmic::Element<'a, ArtistMessage> {
    if artists.is_empty() {
        return common::empty_state(
            "system-users-symbolic",
            "No artists found",
            "Artists will appear here once your library is scanned",
        );
    }

    let mut list = widget::Column::new().spacing(2);

    for (index, artist) in artists.iter().enumerate() {
        let avatar: cosmic::Element<'_, ArtistMessage> =
            if let Some(handle) = artist_avatars.get(&artist.name) {
                widget::icon::icon(handle.clone()).size(48).into()
            } else {
                widget::icon::from_name("avatar-default-symbolic")
                    .size(48)
                    .into()
            };

        let row = widget::button::custom(
            widget::Row::new()
                .push(avatar)
                .push(
                    widget::Column::new()
                        .push(common::cell_text(artist.name.as_str()))
                        .push(common::cell_caption(format!(
                            "{} album{}, {} track{}",
                            artist.album_count(),
                            if artist.album_count() == 1 { "" } else { "s" },
                            artist.track_count(),
                            if artist.track_count() == 1 { "" } else { "s" }
                        )))
                        .spacing(2),
                )
                .spacing(14)
                .align_y(Alignment::Center)
                .padding([10, 8]),
        )
        .on_press(ArtistMessage::SelectArtist(index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        list = list.push(row);
    }

    widget::scrollable(widget::container(list).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// Render the detail view for a selected artist.
pub fn artist_detail_view<'a>(
    artist: &'a Artist,
    artist_index: usize,
    artist_avatars: &'a std::collections::HashMap<String, widget::icon::Handle>,
    cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
    current_track_id: Option<i64>,
) -> cosmic::Element<'a, ArtistMessage> {
    let avatar: cosmic::Element<'_, ArtistMessage> =
        if let Some(handle) = artist_avatars.get(&artist.name) {
            widget::icon::icon(handle.clone()).size(80).into()
        } else {
            widget::icon::from_name("avatar-default-symbolic")
                .size(80)
                .into()
        };

    let header = widget::Row::new()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(ArtistMessage::BackToList),
        )
        .push(avatar)
        .push(
            widget::Column::new()
                .push(widget::text::title1(artist.name.as_str()))
                .push(widget::text::caption(format!(
                    "{} album{}, {} track{}",
                    artist.album_count(),
                    if artist.album_count() == 1 { "" } else { "s" },
                    artist.track_count(),
                    if artist.track_count() == 1 { "" } else { "s" }
                )))
                .spacing(4),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut content = widget::Column::new().push(header).spacing(16);

    for (album_idx, album) in artist.albums.iter().enumerate() {
        let key = CoverArt::album_key(&artist.name, &album.name);
        let album_art: cosmic::Element<'_, ArtistMessage> =
            if let Some(handle) = cover_images.get(&key) {
                widget::icon::icon(handle.clone()).size(64).into()
            } else {
                widget::icon::from_name("media-optical-cd-audio-symbolic")
                    .size(48)
                    .into()
            };

        let album_header = widget::Row::new()
            .push(
                widget::container(album_art)
                    .width(64)
                    .height(64)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            )
            .push(
                widget::Column::new()
                    .push(widget::text::title4(album.name.as_str()))
                    .push(widget::text::caption({
                        let n = album.track_count();
                        let track_word = if n == 1 { "track" } else { "tracks" };
                        if album.year > 0 {
                            format!("{} · {n} {track_word}", album.year)
                        } else {
                            format!("{n} {track_word}")
                        }
                    })),
            )
            .push(
                widget::button::suggested("Play")
                    .on_press(ArtistMessage::PlayArtistAlbum(artist_index, album_idx)),
            )
            .spacing(12)
            .align_y(Alignment::Center);

        content = content.push(album_header);

        let mut track_list = widget::Column::new().spacing(1);
        for (track_idx, track) in album.tracks.iter().enumerate() {
            let track_id = track.id.to_string();
            let is_playing = current_track_id == Some(track.id);

            let num_col: cosmic::Element<'_, ArtistMessage> = if is_playing {
                widget::icon::from_name("media-playback-start-symbolic")
                    .size(14)
                    .into()
            } else {
                common::cell_text(format!("{}", track.track_number)).into()
            };

            let heart_btn = common::favorite_button(
                track.is_favorite,
                ArtistMessage::ToggleFavorite(track_id.clone()),
            );

            let rating_row = widget::container(common::star_rating(track.rating, {
                let track_id = track_id.clone();
                move |r| ArtistMessage::SetRating(track_id.clone(), r)
            }))
            .width(112);

            let genre_widget: cosmic::Element<'_, ArtistMessage> = if !track.genre.is_empty() {
                widget::button::custom(common::cell_caption(track.genre.as_str()))
                    .on_press(ArtistMessage::FilterByGenre(track.genre.clone()))
                    .class(cosmic::theme::Button::Standard)
                    .into()
            } else {
                widget::Space::new().width(Length::Shrink).into()
            };
            let genre_col = widget::container(genre_widget).width(130);

            let row = widget::button::custom(
                widget::Row::new()
                    .push(
                        widget::container(num_col)
                            .width(40)
                            .align_x(Horizontal::Center),
                    )
                    .push(common::cell_text(track.title.as_str()).width(Length::FillPortion(4)))
                    .push(heart_btn)
                    .push(rating_row)
                    .push(genre_col)
                    .push(common::duration_cell(track.duration.as_secs()))
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .padding(4),
            )
            .on_press(ArtistMessage::PlayTrack(artist_index, album_idx, track_idx))
            .width(Length::Fill)
            .class(cosmic::theme::Button::Text);

            track_list = track_list.push(row);
        }

        content = content.push(track_list);
        content = content.push(widget::divider::horizontal::default());
    }

    widget::scrollable(widget::container(content).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}
