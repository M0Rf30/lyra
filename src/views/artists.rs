// SPDX-License-Identifier: GPL-3.0

//! Artists view - list of artists with album sub-views.

use crate::library::{Artist, CoverArt};
use crate::views::{empty_state, spacing};
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
        return empty_state(
            "system-users-symbolic",
            "No artists found",
            "Add music directories in Settings to get started",
        );
    }

    let mut list = widget::column().spacing(spacing::XXXS);

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
            widget::row()
                .push(avatar)
                .push(
                    widget::column()
                        .push(widget::text(artist.name.as_str()))
                        .push(widget::text::caption(format!(
                            "{} albums, {} tracks",
                            artist.album_count(),
                            artist.track_count()
                        )))
                        .spacing(spacing::XXXS),
                )
                .spacing(spacing::XS)
                .align_y(Alignment::Center)
                .padding([spacing::XXS, spacing::XXS]),
        )
        .on_press(ArtistMessage::SelectArtist(index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        list = list.push(row);
    }

    widget::scrollable(
        widget::container(list)
            .padding(spacing::S)
            .width(Length::Fill),
    )
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

    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(ArtistMessage::BackToList),
        )
        .push(avatar)
        .push(
            widget::column()
                .push(widget::text::title1(artist.name.as_str()))
                .push(widget::text::caption(format!(
                    "{} albums, {} tracks",
                    artist.album_count(),
                    artist.track_count()
                )))
                .spacing(spacing::XXXS),
        )
        .spacing(spacing::S)
        .align_y(Alignment::Center);

    let mut content = widget::column().push(header).spacing(spacing::S);

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

        let album_header = widget::row()
            .push(
                widget::container(album_art)
                    .width(64)
                    .height(64)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            )
            .push(
                widget::column()
                    .push(widget::text::title4(album.name.as_str()))
                    .push(widget::text::caption(format!(
                        "{}  \u{2022}  {} tracks",
                        if album.year > 0 {
                            album.year.to_string()
                        } else {
                            String::new()
                        },
                        album.track_count()
                    )))
                    .spacing(spacing::XXXS),
            )
            .push(
                widget::button::suggested("Play")
                    .on_press(ArtistMessage::PlayArtistAlbum(artist_index, album_idx)),
            )
            .spacing(spacing::XS)
            .align_y(Alignment::Center);

        content = content.push(album_header);

        let mut track_list = widget::column().spacing(spacing::XXXS);
        for (track_idx, track) in album.tracks.iter().enumerate() {
            let track_id = track.id.to_string();
            let is_playing = current_track_id == Some(track.id);

            let num_col: cosmic::Element<'_, ArtistMessage> = if is_playing {
                widget::icon::from_name("media-playback-start-symbolic")
                    .size(14)
                    .into()
            } else {
                widget::text(format!("{}", track.track_number)).into()
            };

            let fav_icon_name = if track.is_favorite {
                "emblem-favorite-symbolic"
            } else {
                "non-starred-symbolic"
            };
            let heart_btn = widget::button::icon(widget::icon::from_name(fav_icon_name).size(16))
                .on_press(ArtistMessage::ToggleFavorite(track_id.clone()));

            let rating_row = artist_star_rating(track_id, track.rating);

            let genre_widget: cosmic::Element<'_, ArtistMessage> = if !track.genre.is_empty() {
                widget::button::custom(widget::text::caption(&track.genre))
                    .on_press(ArtistMessage::FilterByGenre(track.genre.clone()))
                    .class(cosmic::theme::Button::Standard)
                    .into()
            } else {
                widget::Space::with_width(0).into()
            };

            let row = widget::button::custom(
                widget::row()
                    .push(
                        widget::container(num_col)
                            .width(32)
                            .align_x(Horizontal::Center),
                    )
                    .push(widget::text(track.title.as_str()).width(Length::Fill))
                    .push(widget::text(track.duration_string()).width(60))
                    .push(heart_btn)
                    .push(rating_row)
                    .push(genre_widget)
                    .spacing(spacing::XXS)
                    .align_y(Alignment::Center)
                    .padding([spacing::XXXS, spacing::XXS]),
            )
            .on_press(ArtistMessage::PlayTrack(artist_index, album_idx, track_idx))
            .width(Length::Fill)
            .class(cosmic::theme::Button::Text);

            track_list = track_list.push(row);
        }

        content = content.push(track_list);
        content = content.push(widget::divider::horizontal::default());
    }

    widget::scrollable(
        widget::container(content)
            .padding(spacing::S)
            .width(Length::Fill),
    )
    .height(Length::Fill)
    .into()
}

/// Star rating widget for artist detail tracks (1-5 stars).
fn artist_star_rating<'a>(
    track_id: String,
    current_rating: Option<u8>,
) -> cosmic::Element<'a, ArtistMessage> {
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
            .on_press(ArtistMessage::SetRating(track_id.clone(), new_rating));
        row = row.push(btn);
    }

    row.into()
}
