// SPDX-License-Identifier: GPL-3.0

//! Playlists view - list of playlists with create/delete/rename actions,
//! and a detail view showing tracks in a selected playlist.

use crate::library::{Playlist, Track};
use crate::views::common;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the playlists view.
#[derive(Debug, Clone)]
pub enum PlaylistMessage {
    /// User selected a playlist to view its tracks.
    SelectPlaylist(usize),
    /// Go back to the playlist list from detail view.
    BackToList,
    /// User wants to create a new playlist (carries the name).
    CreatePlaylist(String),
    /// User wants to delete a playlist by index.
    DeletePlaylist(usize),
    /// User wants to rename a playlist (index, new name).
    RenamePlaylist(usize, String),
    /// Play all tracks in a playlist.
    PlayPlaylist(usize),
    /// Play a specific track within a playlist detail view.
    PlayTrack(usize, usize),
    /// Remove a track from a playlist (playlist index, track index).
    RemoveTrack(usize, usize),
    /// The new-playlist name input changed.
    NewPlaylistNameChanged(String),
    /// The rename input changed (playlist index, new text).
    RenameInputChanged(usize, String),
}

/// Render the playlist list view.
pub fn playlist_list_view<'a>(
    playlists: &'a [Playlist],
    new_playlist_name: &'a str,
) -> cosmic::Element<'a, PlaylistMessage> {
    let mut col = widget::Column::new().spacing(12).padding(16);

    // Create playlist row
    let create_row = widget::Row::new()
        .push(
            widget::text_input("New playlist name...", new_playlist_name)
                .on_input(PlaylistMessage::NewPlaylistNameChanged)
                .width(Length::Fixed(360.0)),
        )
        .push(
            widget::button::suggested("Create").on_press_maybe(if new_playlist_name.is_empty() {
                None
            } else {
                Some(PlaylistMessage::CreatePlaylist(
                    new_playlist_name.to_string(),
                ))
            }),
        )
        .spacing(8)
        .align_y(Alignment::Center);

    col = col.push(create_row);
    col = col.push(widget::divider::horizontal::default());

    if playlists.is_empty() {
        col = col.push(common::empty_state(
            "playlist-symbolic",
            "No playlists",
            "Create a playlist to get started",
        ));

        return col.into();
    }

    let mut list = widget::Column::new().spacing(2);

    for (index, playlist) in playlists.iter().enumerate() {
        let track_count = playlist.track_count;
        let info = widget::Column::new()
            .push(common::cell_text(playlist.name.as_str()))
            .push(common::cell_caption(format!(
                "{track_count} track{}  -  {}",
                if track_count == 1 { "" } else { "s" },
                common::format_duration_coarse(playlist.total_duration.as_secs())
            )))
            .spacing(2);

        let delete_btn = common::icon_button(
            "edit-delete-symbolic",
            16,
            "Delete playlist",
            PlaylistMessage::DeletePlaylist(index),
        );

        let playlist_icon: cosmic::Element<'_, PlaylistMessage> =
            widget::icon::from_name("playlist-symbolic").size(40).into();

        let row = widget::button::custom(
            widget::Row::new()
                .push(playlist_icon)
                .push(info.width(Length::Fill))
                .push(delete_btn)
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(8),
        )
        .on_press(PlaylistMessage::SelectPlaylist(index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        list = list.push(row);
    }

    col = col
        .push(widget::scrollable(widget::container(list).width(Length::Fill)).height(Length::Fill));

    col.into()
}

pub fn playlist_detail_view<'a>(
    playlist: &'a Playlist,
    playlist_index: usize,
    edit_name: &'a str,
) -> cosmic::Element<'a, PlaylistMessage> {
    let detail_icon: cosmic::Element<'_, PlaylistMessage> =
        widget::icon::from_name("playlist-symbolic").size(80).into();

    let rename_row = widget::Row::new()
        .push(
            widget::text_input("Playlist name...", edit_name)
                .on_input(move |text| PlaylistMessage::RenameInputChanged(playlist_index, text))
                .width(Length::Fixed(360.0)),
        )
        .push(widget::button::standard("Rename").on_press_maybe(
            if !edit_name.trim().is_empty() && edit_name != playlist.name.as_str() {
                Some(PlaylistMessage::RenamePlaylist(
                    playlist_index,
                    edit_name.to_string(),
                ))
            } else {
                None
            },
        ))
        .spacing(8)
        .align_y(Alignment::Center);

    let track_count = playlist.track_count;
    let header = widget::Row::new()
        .push(common::icon_button(
            "go-previous-symbolic",
            16,
            "Back to playlists",
            PlaylistMessage::BackToList,
        ))
        .push(detail_icon)
        .push(
            widget::Column::new()
                .push(widget::text::title1(playlist.name.as_str()))
                .push(common::cell_caption(format!(
                    "{track_count} track{}  -  {}",
                    if track_count == 1 { "" } else { "s" },
                    common::format_duration_coarse(playlist.total_duration.as_secs())
                )))
                .push(rename_row)
                .push(
                    widget::button::suggested("Play All")
                        .on_press(PlaylistMessage::PlayPlaylist(playlist_index)),
                )
                .spacing(8),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut track_list = widget::Column::new().spacing(2);

    for (track_idx, track) in playlist.tracks.iter().enumerate() {
        let remove_btn = widget::tooltip(
            widget::button::icon(widget::icon::from_name("list-remove-symbolic").size(16))
                .on_press(PlaylistMessage::RemoveTrack(playlist_index, track_idx)),
            widget::text::caption("Remove from playlist"),
            widget::tooltip::Position::Top,
        );

        let row = widget::button::custom(
            widget::Row::new()
                .push(common::cell_text(format!("{}", track_idx + 1)).width(40))
                .push(common::cell_text(track.title.as_str()).width(Length::FillPortion(4)))
                .push(common::cell_text(track.artist.as_str()).width(Length::FillPortion(3)))
                .push(common::duration_cell(track.duration.as_secs()))
                .push(remove_btn)
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(4),
        )
        .on_press(PlaylistMessage::PlayTrack(playlist_index, track_idx))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    if playlist.tracks.is_empty() {
        track_list = track_list.push(common::empty_state(
            "playlist-symbolic",
            "This playlist is empty",
            "Add tracks from the Songs view.",
        ));
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
