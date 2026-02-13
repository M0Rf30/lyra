// SPDX-License-Identifier: GPL-3.0

//! Artists view - list of artists with album sub-views.

use crate::library::Artist;
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
}

/// Render the artist list.
pub fn artist_list_view(artists: &[Artist]) -> cosmic::Element<'_, ArtistMessage> {
    if artists.is_empty() {
        return widget::container(
            widget::column()
                .push(widget::icon::from_name("system-users-symbolic").size(64))
                .push(widget::text::title3("No artists found"))
                .spacing(12)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into();
    }

    let mut list = widget::column().spacing(2);

    for (index, artist) in artists.iter().enumerate() {
        let row = widget::button::custom(
            widget::row()
                .push(widget::icon::from_name("avatar-default-symbolic").size(40))
                .push(
                    widget::column()
                        .push(widget::text(artist.name.as_str()))
                        .push(widget::text::caption(format!(
                            "{} albums, {} tracks",
                            artist.album_count(),
                            artist.track_count()
                        )))
                        .spacing(2),
                )
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(8),
        )
        .on_press(ArtistMessage::SelectArtist(index))
        .width(Length::Fill);

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
) -> cosmic::Element<'a, ArtistMessage> {
    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(ArtistMessage::BackToList),
        )
        .push(widget::icon::from_name("avatar-default-symbolic").size(80))
        .push(
            widget::column()
                .push(widget::text::title1(artist.name.as_str()))
                .push(widget::text::caption(format!(
                    "{} albums, {} tracks",
                    artist.album_count(),
                    artist.track_count()
                )))
                .spacing(4),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut content = widget::column().push(header).spacing(16);

    for (album_idx, album) in artist.albums.iter().enumerate() {
        let album_header = widget::row()
            .push(
                widget::container(
                    widget::icon::from_name("media-optical-cd-audio-symbolic").size(48),
                )
                .width(64)
                .height(64)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Card),
            )
            .push(
                widget::column()
                    .push(widget::text::title4(album.name.as_str()))
                    .push(widget::text::caption(format!(
                        "{}  -  {} tracks",
                        if album.year > 0 {
                            album.year.to_string()
                        } else {
                            String::new()
                        },
                        album.track_count()
                    )))
                    .spacing(2),
            )
            .push(
                widget::button::suggested("Play")
                    .on_press(ArtistMessage::PlayArtistAlbum(artist_index, album_idx)),
            )
            .spacing(12)
            .align_y(Alignment::Center);

        content = content.push(album_header);

        let mut track_list = widget::column().spacing(1);
        for (track_idx, track) in album.tracks.iter().enumerate() {
            let row = widget::button::custom(
                widget::row()
                    .push(widget::text(format!("{}", track.track_number)).width(32))
                    .push(widget::text(track.title.as_str()).width(Length::Fill))
                    .push(widget::text(track.duration_string()).width(60))
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .padding(4),
            )
            .on_press(ArtistMessage::PlayTrack(artist_index, album_idx, track_idx))
            .width(Length::Fill);

            track_list = track_list.push(row);
        }

        content = content.push(track_list);
        content = content.push(widget::divider::horizontal::default());
    }

    widget::scrollable(widget::container(content).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}
