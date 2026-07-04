// SPDX-License-Identifier: GPL-3.0

//! Albums grid view - displays album covers in a responsive grid (Lollypop-style).

use crate::fl;
use crate::library::{Album, CoverArt, Playlist};
use crate::views::common;
use crate::views::{card_button_class, list_row_button_class};
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::core::text::Wrapping;
use cosmic::iced::{Alignment, Length, Padding};
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

/// Card artwork/label width — the grid, art frame, and clipped labels all
/// share this so a card's cover and its two-line caption line up exactly.
const CARD_WIDTH: f32 = 160.0;

/// Fixed height for the two-line label block under each card: body
/// line-height (21px) + caption line-height (17px) + 2px inter-line spacing.
/// A constant height (rather than sizing to content) is what keeps every
/// card in a row the same height, whether or not the artist line has text.
const CARD_LABEL_HEIGHT: f32 = 40.0;

/// Fixed width for genre chip buttons so a row of chips lines up neatly.
const GENRE_CHIP_WIDTH: f32 = 130.0;

/// Extra right-hand padding so the scrollbar gutter doesn't visually eat
/// into the grid's margin, keeping perceived left/right whitespace symmetric.
const SCROLLBAR_CLEARANCE: f32 = 16.0;

/// Caption-styled, single-line text dimmed to the theme's secondary
/// (neutral_7) color — used for artist subtitles under a bolder title so the
/// two lines read with clear hierarchy instead of matching weight and color.
fn secondary_caption<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a) -> common::Text<'a> {
    common::cell_caption(content).class(cosmic::theme::Text::Custom(|theme| {
        cosmic::iced::widget::text::Style {
            color: Some(theme.cosmic().palette.neutral_7.into()),
            ..Default::default()
        }
    }))
}

/// Render the album grid view.
pub fn album_grid_view<'a>(
    albums: &'a [Album],
    cover_images: &'a std::collections::HashMap<String, widget::icon::Handle>,
) -> cosmic::Element<'a, AlbumMessage> {
    if albums.is_empty() {
        return common::empty_state(
            "folder-music-symbolic",
            fl!("no-albums"),
            fl!("albums-empty-hint"),
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
                        .width(CARD_WIDTH)
                        .height(CARD_WIDTH)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center)
                        .class(cosmic::theme::Container::Card)
                        .into()
                };

            // A missing or title-echoing artist still reserves the caption
            // line's height (a non-breaking space) so every card's label
            // block is exactly two lines tall and rows stay aligned.
            let has_distinct_artist = !album.artist.trim().is_empty()
                && !album.artist.trim().eq_ignore_ascii_case(album.name.trim());
            let artist_display = if has_distinct_artist {
                album.artist.as_str()
            } else {
                "\u{a0}"
            };

            let label_block = widget::container(
                widget::Column::new()
                    .push(
                        widget::container(common::clipped_cell(
                            common::cell_text(album.name.as_str()).into(),
                        ))
                        .width(CARD_WIDTH),
                    )
                    .push(
                        widget::container(common::clipped_cell(
                            secondary_caption(artist_display).into(),
                        ))
                        .width(CARD_WIDTH),
                    )
                    .spacing(2),
            )
            .height(Length::Fixed(CARD_LABEL_HEIGHT));

            let album_card = widget::Column::new()
                .push(
                    widget::container(art_widget)
                        .width(CARD_WIDTH)
                        .height(CARD_WIDTH)
                        .align_x(Horizontal::Center)
                        .align_y(Vertical::Center),
                )
                .push(label_block)
                .spacing(8);

            let tooltip_label = if has_distinct_artist {
                fl!(
                    "album-tooltip",
                    title = album.name.clone(),
                    artist = album.artist.clone()
                )
            } else {
                album.name.clone()
            };

            widget::tooltip(
                widget::button::custom(album_card)
                    .on_press(AlbumMessage::SelectAlbum(index))
                    .padding(8)
                    .class(card_button_class()),
                widget::text::caption(tooltip_label),
                widget::tooltip::Position::Top,
            )
            .into()
        })
        .collect();

    let spacing = cosmic::theme::active().cosmic().spacing;

    let grid = widget::flex_row(cards)
        .column_spacing(20)
        .row_spacing(20)
        .width(Length::Fill)
        .justify_content(widget::JustifyContent::Center);

    widget::scrollable(
        widget::container(grid)
            .padding(Padding {
                top: f32::from(spacing.space_m),
                right: f32::from(spacing.space_m) + SCROLLBAR_CLEARANCE,
                bottom: f32::from(spacing.space_m),
                left: f32::from(spacing.space_m),
            })
            .width(Length::Fill),
    )
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
    let spacing = cosmic::theme::active().cosmic().spacing;

    let title_line = widget::container(common::clipped_cell(
        widget::text::title2(album.name.as_str())
            .wrapping(Wrapping::None)
            .into(),
    ))
    .width(Length::Fill);

    let artist_line = widget::container(common::clipped_cell(
        widget::text::body(album.artist.as_str())
            .wrapping(Wrapping::None)
            .class(cosmic::theme::Text::Custom(|theme| {
                cosmic::iced::widget::text::Style {
                    color: Some(theme.cosmic().palette.neutral_7.into()),
                    ..Default::default()
                }
            }))
            .into(),
    ))
    .width(Length::Fill);

    let mut meta_col = widget::Column::new()
        .push(title_line)
        .push(artist_line)
        .push(common::cell_caption(format!(
            "{track_label} \u{b7} {duration_label}"
        )))
        .push(
            widget::button::suggested(fl!("play-album"))
                .on_press(AlbumMessage::PlayAlbum(album_index)),
        )
        .width(Length::Fill)
        .spacing(8);

    // Task 103: Genre chips in album header
    if !genres.is_empty() {
        let mut genre_row = widget::Row::new()
            .spacing(spacing.space_xs)
            .align_y(Alignment::Center);
        for genre in genres {
            // Use the owned String for both the message and the label.
            let label = genre.clone();
            genre_row = genre_row.push(
                widget::button::custom(common::clipped_cell(common::cell_caption(label).into()))
                    .on_press(AlbumMessage::FilterByGenre(genre))
                    .class(cosmic::theme::Button::Standard)
                    .width(GENRE_CHIP_WIDTH),
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
            widget::button::custom(common::clipped_cell(
                common::cell_caption(track.genre.as_str()).into(),
            ))
            .on_press(AlbumMessage::FilterByGenre(track.genre.clone()))
            .class(cosmic::theme::Button::Standard)
            .width(GENRE_CHIP_WIDTH)
            .into()
        } else {
            widget::Space::new().width(GENRE_CHIP_WIDTH).into()
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
                .push(
                    widget::container(common::clipped_cell(
                        common::cell_text(track.title.as_str()).into(),
                    ))
                    .width(Length::FillPortion(4)),
                )
                .push(
                    widget::container(common::clipped_cell(
                        common::cell_text(track.artist.as_str()).into(),
                    ))
                    .width(Length::FillPortion(3)),
                )
                .push(heart_btn)
                .push(rating_row)
                .push(genre_widget)
                .push(playlist_btn)
                .push(common::duration_cell(track.duration.as_secs()))
                .spacing(8)
                .width(Length::Fill)
                .align_y(Alignment::Center)
                .padding(4),
        )
        .on_press(AlbumMessage::PlayTrack(album_index, track_idx))
        .width(Length::Fill)
        .class(list_row_button_class(is_playing));

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
