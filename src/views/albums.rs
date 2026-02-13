// SPDX-License-Identifier: GPL-3.0

//! Albums grid view - displays album covers in a responsive grid (Lollypop-style).

use crate::library::Album;
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
}

/// Render the album grid view.
pub fn album_grid_view<'a>(
    albums: &'a [Album],
    _cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
) -> cosmic::Element<'a, AlbumMessage> {
    if albums.is_empty() {
        return widget::container(
            widget::column()
                .push(widget::icon::from_name("folder-music-symbolic").size(64))
                .push(widget::text::title3("No albums found"))
                .push(widget::text(
                    "Add music directories in Settings to get started",
                ))
                .spacing(12)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into();
    }

    // Build album cards into rows of ~4 items
    let cards_per_row = 4;
    let mut rows = widget::column().spacing(16);
    let mut current_row = widget::row().spacing(16);
    let mut count = 0;

    for (index, album) in albums.iter().enumerate() {
        let album_card = widget::column()
            .push(
                // Album art placeholder (square container with icon)
                widget::container(
                    widget::icon::from_name("media-optical-cd-audio-symbolic").size(80),
                )
                .width(180)
                .height(180)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Card),
            )
            .push(
                widget::column()
                    .push(widget::text(album.name.as_str()).width(180))
                    .push(widget::text::caption(album.artist.as_str()).width(180))
                    .spacing(2),
            )
            .spacing(8);

        current_row = current_row.push(
            widget::button::custom(album_card)
                .on_press(AlbumMessage::SelectAlbum(index))
                .padding(8),
        );
        count += 1;

        if count >= cards_per_row {
            rows = rows.push(current_row);
            current_row = widget::row().spacing(16);
            count = 0;
        }
    }

    // Push remaining cards
    if count > 0 {
        rows = rows.push(current_row);
    }

    widget::scrollable(widget::container(rows).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// Render the detail view for a selected album.
pub fn album_detail_view<'a>(
    album: &'a Album,
    album_index: usize,
) -> cosmic::Element<'a, AlbumMessage> {
    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(AlbumMessage::BackToGrid),
        )
        .push(
            widget::container(widget::icon::from_name("media-optical-cd-audio-symbolic").size(120))
                .width(160)
                .height(160)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .class(cosmic::theme::Container::Card),
        )
        .push(
            widget::column()
                .push(widget::text::title1(album.name.as_str()))
                .push(widget::text::title3(album.artist.as_str()))
                .push(widget::text::caption(format!(
                    "{} tracks  -  {}",
                    album.track_count(),
                    format_duration(album.total_duration())
                )))
                .push(
                    widget::button::suggested("Play Album")
                        .on_press(AlbumMessage::PlayAlbum(album_index)),
                )
                .spacing(8),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut track_list = widget::column().spacing(2);

    for (track_idx, track) in album.tracks.iter().enumerate() {
        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", track.track_number)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(60))
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(4),
        )
        .on_press(AlbumMessage::PlayTrack(album_index, track_idx))
        .width(Length::Fill);

        track_list = track_list.push(row);
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

fn format_duration(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}
