// SPDX-License-Identifier: GPL-3.0

use crate::fl;
use crate::library::Track;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

#[derive(Debug, Clone)]
pub enum TagEditorMessage {
    SelectTrack(usize),
    TitleChanged(String),
    ArtistChanged(String),
    AlbumChanged(String),
    AlbumArtistChanged(String),
    YearChanged(String),
    TrackNumberChanged(String),
    DiscNumberChanged(String),
    GenreChanged(String),
    CommentChanged(String),
    SearchChanged(String),
    Save,
}

pub fn tag_editor_view<'a>(
    all_tracks: &'a [Track],
    selected_index: Option<usize>,
    edit_title: &'a str,
    edit_artist: &'a str,
    edit_album: &'a str,
    edit_album_artist: &'a str,
    edit_year: &'a str,
    edit_track_number: &'a str,
    edit_disc_number: &'a str,
    edit_genre: &'a str,
    edit_comment: &'a str,
    search_query: &'a str,
    save_status: Option<&'a str>,
    dirty: bool,
) -> cosmic::Element<'a, TagEditorMessage> {
    let local_tracks: Vec<(usize, &Track)> = all_tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.provider_id.as_ref() == "local")
        .collect();

    let filtered: Vec<(usize, &Track)> = if search_query.is_empty() {
        local_tracks.clone()
    } else {
        let q = search_query.to_lowercase();
        local_tracks
            .into_iter()
            .filter(|(_, t)| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .collect()
    };

    let search_bar = widget::search_input(fl!("tag-editor-search"), search_query)
        .on_input(TagEditorMessage::SearchChanged)
        .width(Length::Fill);

    let mut track_list = widget::column().spacing(2);
    for (orig_idx, track) in &filtered {
        let is_selected = selected_index == Some(*orig_idx);
        let label = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} — {}", track.artist, track.title)
        };
        let btn = widget::button::text(label)
            .on_press(TagEditorMessage::SelectTrack(*orig_idx))
            .width(Length::Fill);
        if is_selected {
            track_list = track_list.push(
                widget::container(btn)
                    .class(cosmic::theme::Container::Primary)
                    .width(Length::Fill),
            );
        } else {
            track_list = track_list.push(btn);
        }
    }

    let track_panel = widget::container(
        widget::column()
            .push(search_bar)
            .push(widget::Space::new(Length::Shrink, Length::Fixed(8.0)))
            .push(
                widget::scrollable(track_list)
                    .height(Length::Fill)
                    .width(Length::Fill),
            )
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .padding([12, 12, 12, 12])
    .width(Length::FillPortion(2))
    .height(Length::Fill);

    let form: cosmic::Element<'_, TagEditorMessage> = if selected_index.is_some() {
        let mut col = widget::column().spacing(12);

        col = col
            .push(labeled_field(
                fl!("tag-editor-title"),
                edit_title,
                TagEditorMessage::TitleChanged,
            ))
            .push(labeled_field(
                fl!("tag-editor-artist"),
                edit_artist,
                TagEditorMessage::ArtistChanged,
            ))
            .push(labeled_field(
                fl!("tag-editor-album"),
                edit_album,
                TagEditorMessage::AlbumChanged,
            ))
            .push(labeled_field(
                fl!("tag-editor-album-artist"),
                edit_album_artist,
                TagEditorMessage::AlbumArtistChanged,
            ))
            .push(
                widget::row()
                    .push(labeled_field(
                        fl!("tag-editor-year"),
                        edit_year,
                        TagEditorMessage::YearChanged,
                    ))
                    .push(labeled_field(
                        fl!("tag-editor-track-number"),
                        edit_track_number,
                        TagEditorMessage::TrackNumberChanged,
                    ))
                    .push(labeled_field(
                        fl!("tag-editor-disc-number"),
                        edit_disc_number,
                        TagEditorMessage::DiscNumberChanged,
                    ))
                    .spacing(12),
            )
            .push(labeled_field(
                fl!("tag-editor-genre"),
                edit_genre,
                TagEditorMessage::GenreChanged,
            ))
            .push(labeled_field(
                fl!("tag-editor-comment"),
                edit_comment,
                TagEditorMessage::CommentChanged,
            ));

        if let Some(status) = save_status {
            col = col.push(widget::text::body(status));
        }

        col = col.push(
            widget::button::suggested(fl!("tag-editor-save"))
                .on_press_maybe(dirty.then_some(TagEditorMessage::Save)),
        );

        widget::scrollable(col)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    } else {
        widget::container(
            widget::text::body(fl!("tag-editor-select-track"))
                .apply(widget::container)
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
    };

    let form_panel = widget::container(form)
        .padding([16, 20, 16, 20])
        .width(Length::FillPortion(3))
        .height(Length::Fill);

    widget::row()
        .push(track_panel)
        .push(form_panel)
        .height(Length::Fill)
        .width(Length::Fill)
        .align_y(Alignment::Start)
        .into()
}

fn labeled_field<'a>(
    label: String,
    value: &'a str,
    on_input: impl Fn(String) -> TagEditorMessage + 'a,
) -> cosmic::Element<'a, TagEditorMessage> {
    widget::column()
        .push(widget::text::body(label))
        .push(
            widget::text_input("", value)
                .on_input(on_input)
                .width(Length::Fill),
        )
        .spacing(4)
        .width(Length::Fill)
        .into()
}
