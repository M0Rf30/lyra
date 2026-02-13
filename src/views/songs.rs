// SPDX-License-Identifier: GPL-3.0

//! Songs list view - flat track listing with sorting options.

use crate::library::Track;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the songs view.
#[derive(Debug, Clone)]
pub enum SongMessage {
    PlayTrack(usize),
    SortBy(SortField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Title,
    Artist,
    Album,
    Duration,
}

/// Render the songs list view with column headers.
pub fn songs_list_view<'a>(
    tracks: &'a [Track],
    current_sort: SortField,
) -> cosmic::Element<'a, SongMessage> {
    if tracks.is_empty() {
        return widget::container(
            widget::column()
                .push(widget::icon::from_name("audio-x-generic-symbolic").size(64))
                .push(widget::text::title3("No songs found"))
                .spacing(12)
                .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into();
    }

    // Column headers
    let header = widget::row()
        .push(widget::text("#").width(40))
        .push(
            widget::button::custom(widget::text(sort_label(
                "Title",
                SortField::Title,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Title))
            .width(Length::Fill)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Artist",
                SortField::Artist,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Artist))
            .width(200)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Album",
                SortField::Album,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Album))
            .width(200)
            .class(cosmic::theme::Button::Text),
        )
        .push(
            widget::button::custom(widget::text(sort_label(
                "Duration",
                SortField::Duration,
                current_sort,
            )))
            .on_press(SongMessage::SortBy(SortField::Duration))
            .width(80)
            .class(cosmic::theme::Button::Text),
        )
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([4, 8]);

    let mut track_list = widget::column().spacing(1);

    for (index, track) in tracks.iter().enumerate() {
        let row = widget::button::custom(
            widget::row()
                .push(widget::text(format!("{}", index + 1)).width(40))
                .push(widget::text(track.title.as_str()).width(Length::Fill))
                .push(widget::text(track.artist.as_str()).width(200))
                .push(widget::text(track.album.as_str()).width(200))
                .push(widget::text(track.duration_string()).width(80))
                .spacing(8)
                .align_y(Alignment::Center)
                .padding([4, 8]),
        )
        .on_press(SongMessage::PlayTrack(index))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

        track_list = track_list.push(row);
    }

    widget::column()
        .push(header)
        .push(widget::divider::horizontal::default())
        .push(
            widget::scrollable(widget::container(track_list).width(Length::Fill))
                .height(Length::Fill),
        )
        .padding(16)
        .spacing(4)
        .into()
}

fn sort_label(name: &str, field: SortField, current: SortField) -> String {
    if field == current {
        format!("{name} ^")
    } else {
        name.to_string()
    }
}
