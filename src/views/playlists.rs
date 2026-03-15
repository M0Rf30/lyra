// SPDX-License-Identifier: GPL-3.0

//! Playlists view - list of playlists with create/delete/rename actions,
//! and a detail view showing tracks in a selected playlist.

use crate::library::{Playlist, Track};
use crate::views::{empty_state, spacing};
use cosmic::iced::alignment::{Horizontal, Vertical};
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
    let mut col = widget::column().spacing(spacing::XS).padding(spacing::S);

    // Create playlist row
    let create_row = widget::row()
        .push(
            widget::text_input("New playlist name...", new_playlist_name)
                .on_input(PlaylistMessage::NewPlaylistNameChanged)
                .width(Length::Fill),
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
        .spacing(spacing::XXS)
        .align_y(Alignment::Center);

    col = col.push(create_row);
    col = col.push(widget::divider::horizontal::default());

    if playlists.is_empty() {
        col = col.push(empty_state::<PlaylistMessage>(
            "playlist-symbolic",
            "No playlists",
            "Create a playlist to get started",
        ));

        return col.into();
    }

    let mut list = widget::column().spacing(spacing::XXXS);

    for (index, playlist) in playlists.iter().enumerate() {
        let info = widget::column()
            .push(widget::text(playlist.name.as_str()))
            .push(widget::text::caption(format!(
                "{} tracks  \u{2022}  {}",
                playlist.track_count,
                format_duration(playlist.total_duration)
            )))
            .spacing(spacing::XXXS);

        let delete_btn = widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
            .on_press(PlaylistMessage::DeletePlaylist(index));

        let playlist_icon: cosmic::Element<'_, PlaylistMessage> =
            widget::icon::from_name("playlist-symbolic").size(40).into();

        let row = widget::button::custom(
            widget::row()
                .push(playlist_icon)
                .push(info.width(Length::Fill))
                .push(delete_btn)
                .spacing(spacing::XS)
                .align_y(Alignment::Center)
                .padding(spacing::XXS),
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

    let rename_row = widget::row()
        .push(
            widget::text_input("Playlist name...", edit_name)
                .on_input(move |text| PlaylistMessage::RenameInputChanged(playlist_index, text))
                .width(Length::Fill),
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
        .spacing(spacing::XXS)
        .align_y(Alignment::Center);

    let header = widget::row()
        .push(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(PlaylistMessage::BackToList),
        )
        .push(detail_icon)
        .push(
            widget::column()
                .push(widget::text::title1(playlist.name.as_str()))
                .push(widget::text::caption(format!(
                    "{} tracks  \u{2022}  {}",
                    playlist.track_count,
                    format_duration(playlist.total_duration)
                )))
                .push(rename_row)
                .push(
                    widget::button::suggested("Play All")
                        .on_press(PlaylistMessage::PlayPlaylist(playlist_index)),
                )
                .spacing(spacing::XXS),
        )
        .spacing(spacing::S)
        .align_y(Alignment::Center);

    let mut track_list = widget::column().spacing(spacing::XXXS);

    for (track_idx, track) in playlist.tracks.iter().enumerate() {
        let remove_btn = widget::button::icon(widget::icon::from_name("list-remove-symbolic"))
            .on_press(PlaylistMessage::RemoveTrack(playlist_index, track_idx));

        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", track_idx + 1)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(60))
                .push(remove_btn)
                .spacing(spacing::XXS)
                .align_y(Alignment::Center)
                .padding([spacing::XXXS, spacing::XXS]),
        )
        .on_press(PlaylistMessage::PlayTrack(playlist_index, track_idx))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    if playlist.tracks.is_empty() {
        track_list = track_list.push(
            widget::container(widget::text(
                "This playlist is empty. Add tracks from the Songs view.",
            ))
            .padding(spacing::S),
        );
    }

    widget::scrollable(
        widget::column()
            .push(header)
            .push(widget::divider::horizontal::default())
            .push(track_list)
            .spacing(spacing::S)
            .padding(spacing::S),
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
