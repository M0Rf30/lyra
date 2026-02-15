// SPDX-License-Identifier: GPL-3.0

//! Albums grid view - displays album covers in a responsive grid (Lollypop-style).

use crate::library::{Album, CoverArt, Playlist};
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

    let cards: Vec<cosmic::Element<'_, AlbumMessage>> = albums
        .iter()
        .enumerate()
        .map(|(index, album)| {
            let key = CoverArt::album_key(&album.artist, &album.name);
            let art_widget: cosmic::Element<'_, AlbumMessage> =
                if let Some(handle) = cover_images.get(&key) {
                    widget::icon::icon(handle.clone()).size(150).into()
                } else {
                    widget::icon::from_name("media-optical-cd-audio-symbolic")
                        .size(80)
                        .into()
                };

            let album_card = widget::column()
                .push(
                    widget::container(art_widget)
                        .width(150)
                        .height(150)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center),
                )
                .push(
                    widget::column()
                        .push(widget::text(truncate_str(&album.name, 20)).width(150))
                        .push(widget::text::caption(truncate_str(&album.artist, 24)).width(150))
                        .spacing(2),
                )
                .spacing(8);

            widget::button::custom(album_card)
                .on_press(AlbumMessage::SelectAlbum(index))
                .padding(8)
                .class(cosmic::theme::Button::Text)
                .into()
        })
        .collect();

    let grid = widget::flex_row(cards)
        .column_spacing(16)
        .row_spacing(16)
        .width(Length::Fill);

    widget::scrollable(widget::container(grid).padding(16).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// Render the detail view for a selected album.
pub fn album_detail_view<'a>(
    album: &'a Album,
    album_index: usize,
    cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
    playlists: &'a [Playlist],
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

    let mut meta_col = widget::column()
        .push(widget::text::title1(album.name.as_str()))
        .push(widget::text::title3(album.artist.as_str()))
        .push(widget::text::caption(format!(
            "{} tracks  -  {}",
            album.track_count(),
            format_duration(album.total_duration())
        )))
        .push(
            widget::button::suggested("Play Album").on_press(AlbumMessage::PlayAlbum(album_index)),
        )
        .spacing(8);

    // Task 103: Genre chips in album header
    if !genres.is_empty() {
        let mut genre_row = widget::row().spacing(4).align_y(Alignment::Center);
        for genre in genres {
            // Use the owned String for both the message and the label.
            let label = genre.clone();
            genre_row = genre_row.push(
                widget::button::custom(widget::text::caption(label))
                    .on_press(AlbumMessage::FilterByGenre(genre))
                    .class(cosmic::theme::Button::Standard),
            );
        }
        meta_col = meta_col.push(genre_row);
    }

    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(AlbumMessage::BackToGrid),
        )
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

    let mut track_list = widget::column().spacing(2);

    for (track_idx, track) in album.tracks.iter().enumerate() {
        let track_id = track.id.to_string();

        // Task 99: Heart icon for favorite toggle
        let fav_icon_name = if track.is_favorite {
            "emblem-favorite-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let heart_btn = widget::button::icon(widget::icon::from_name(fav_icon_name).size(16))
            .on_press(AlbumMessage::ToggleFavorite(track_id.clone()));

        // Task 102: Star rating widget
        let rating_row = album_star_rating(track_id.clone(), track.rating);

        // Task 103: Genre chip per track
        let genre_widget: cosmic::Element<'_, AlbumMessage> = if !track.genre.is_empty() {
            widget::button::custom(widget::text::caption(&track.genre))
                .on_press(AlbumMessage::FilterByGenre(track.genre.clone()))
                .class(cosmic::theme::Button::Standard)
                .into()
        } else {
            widget::Space::with_width(0).into()
        };

        // Task 98: Add to playlist button
        let playlist_btn: cosmic::Element<'_, AlbumMessage> =
            if let Some(first_pl) = playlists.first() {
                widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16))
                    .on_press(AlbumMessage::AddToPlaylist(
                        track.source_uri.clone(),
                        first_pl.id.clone(),
                    ))
                    .into()
            } else {
                widget::button::icon(widget::icon::from_name("list-add-symbolic").size(16)).into()
            };

        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", track.track_number)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(60))
                .push(heart_btn)
                .push(rating_row)
                .push(genre_widget)
                .push(playlist_btn)
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

/// Star rating widget for album detail tracks (1-5 stars).
fn album_star_rating<'a>(
    track_id: String,
    current_rating: Option<u8>,
) -> cosmic::Element<'a, AlbumMessage> {
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
            .on_press(AlbumMessage::SetRating(track_id.clone(), new_rating));
        row = row.push(btn);
    }

    row.into()
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
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
