// SPDX-License-Identifier: GPL-3.0

use crate::fl;
use crate::library::Track;
use crate::services::{LookupRelease, LookupSource};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Which tab is active in the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagEditorTab {
    Tags,
    Lookup,
    Info,
}

/// All messages emitted by the tag editor view.
#[derive(Debug, Clone)]
pub enum TagEditorMessage {
    // Track / album selection
    SelectTrack(usize),
    SelectAlbum(String),
    SearchChanged(String),
    ToggleAlbumMode,

    // Tab switching
    SwitchTab(TagEditorTab),

    // Tag field edits
    TitleChanged(String),
    ArtistChanged(String),
    AlbumChanged(String),
    AlbumArtistChanged(String),
    YearChanged(String),
    TrackNumberChanged(String),
    DiscNumberChanged(String),
    GenreChanged(String),
    CommentChanged(String),
    Save,

    // Lookup
    LookupQueryChanged(String),
    LookupSourceChanged(LookupSource),
    LookupSearch,
    ScanFingerprint,
    SelectResult(usize),
    FetchReleaseTracks(String),
    ApplyResult,

    // API key configuration
    AcoustIdKeyChanged(String),
    DiscogsTokenChanged(String),

    // Batch save (album mode)
    BatchSave,
}

/// All the state the tag editor view needs from AppModel.
pub struct TagEditorState<'a> {
    pub all_tracks: &'a [Track],
    pub selected_index: Option<usize>,
    pub selected_album_name: Option<&'a str>,
    pub album_mode: bool,
    pub active_tab: TagEditorTab,

    // Tag form fields
    pub edit_title: &'a str,
    pub edit_artist: &'a str,
    pub edit_album: &'a str,
    pub edit_album_artist: &'a str,
    pub edit_year: &'a str,
    pub edit_track_number: &'a str,
    pub edit_disc_number: &'a str,
    pub edit_genre: &'a str,
    pub edit_comment: &'a str,

    // UI state
    pub search_query: &'a str,
    pub save_status: Option<&'a str>,
    pub dirty: bool,

    // Lookup state
    pub lookup_query: &'a str,
    pub lookup_source: LookupSource,
    pub lookup_loading: bool,
    pub lookup_results: &'a [LookupRelease],
    pub selected_result: Option<usize>,
    pub fingerprinting: bool,

    // API keys
    pub acoustid_api_key: &'a str,
    pub discogs_token: &'a str,
}

pub fn tag_editor_view<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    let left_panel = build_left_panel(state);
    let right_panel = build_right_panel(state);

    widget::row()
        .push(left_panel)
        .push(right_panel)
        .height(Length::Fill)
        .width(Length::Fill)
        .align_y(Alignment::Start)
        .into()
}

// ── Left panel: file browser with track/album list ──────────────────────────

fn build_left_panel<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    let local_tracks: Vec<(usize, &Track)> = state
        .all_tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.provider_id.as_ref() == "local")
        .collect();

    let filtered: Vec<(usize, &Track)> = if state.search_query.is_empty() {
        local_tracks.clone()
    } else {
        let q = state.search_query.to_lowercase();
        local_tracks
            .into_iter()
            .filter(|(_, t)| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .collect()
    };

    // Mode toggle: Track / Album
    let mode_toggle = widget::row()
        .push(mode_button(
            fl!("tag-editor-track-mode"),
            !state.album_mode,
            TagEditorMessage::ToggleAlbumMode,
        ))
        .push(mode_button(
            fl!("tag-editor-album-mode"),
            state.album_mode,
            TagEditorMessage::ToggleAlbumMode,
        ))
        .spacing(4)
        .width(Length::Fill);

    let search_bar = widget::search_input(fl!("tag-editor-search"), state.search_query)
        .on_input(TagEditorMessage::SearchChanged)
        .width(Length::Fill);

    let list_content: cosmic::Element<'_, TagEditorMessage> = if state.album_mode {
        build_album_list(&filtered, state.selected_album_name)
    } else {
        build_track_list(&filtered, state.selected_index)
    };

    widget::container(
        widget::column()
            .push(mode_toggle)
            .push(widget::Space::new(Length::Shrink, Length::Fixed(4.0)))
            .push(search_bar)
            .push(widget::Space::new(Length::Shrink, Length::Fixed(8.0)))
            .push(
                widget::scrollable(list_content)
                    .height(Length::Fill)
                    .width(Length::Fill),
            )
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .padding([12, 12, 12, 12])
    .width(Length::FillPortion(2))
    .height(Length::Fill)
    .into()
}

fn build_track_list<'a>(
    tracks: &[(usize, &Track)],
    selected: Option<usize>,
) -> cosmic::Element<'a, TagEditorMessage> {
    let mut col = widget::column().spacing(2);
    for (orig_idx, track) in tracks {
        let is_selected = selected == Some(*orig_idx);
        let label = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} — {}", track.artist, track.title)
        };
        let btn = widget::button::text(label)
            .on_press(TagEditorMessage::SelectTrack(*orig_idx))
            .width(Length::Fill);
        if is_selected {
            col = col.push(
                widget::container(btn)
                    .class(cosmic::theme::Container::Primary)
                    .width(Length::Fill),
            );
        } else {
            col = col.push(btn);
        }
    }
    col.into()
}

fn build_album_list<'a>(
    tracks: &[(usize, &Track)],
    selected_album: Option<&str>,
) -> cosmic::Element<'a, TagEditorMessage> {
    // Group tracks by album
    let mut albums: Vec<(String, Vec<(usize, &Track)>)> = Vec::new();
    for (idx, track) in tracks {
        let album_name = if track.album.is_empty() {
            "Unknown Album".to_string()
        } else {
            track.album.clone()
        };
        if let Some(entry) = albums.iter_mut().find(|(name, _)| *name == album_name) {
            entry.1.push((*idx, track));
        } else {
            albums.push((album_name, vec![(*idx, track)]));
        }
    }

    let mut col = widget::column().spacing(4);
    for (album_name, album_tracks) in &albums {
        let is_selected = selected_album == Some(album_name.as_str());
        let label = format!("{} ({} tracks)", album_name, album_tracks.len());
        let btn = widget::button::text(label)
            .on_press(TagEditorMessage::SelectAlbum(album_name.clone()))
            .width(Length::Fill);
        if is_selected {
            col = col.push(
                widget::container(btn)
                    .class(cosmic::theme::Container::Primary)
                    .width(Length::Fill),
            );
            // Show tracks within the selected album
            let mut track_col = widget::column().spacing(2).padding([0, 0, 0, 16]);
            for (orig_idx, track) in album_tracks {
                let track_label = if track.track_number > 0 {
                    format!("{}. {}", track.track_number, track.title)
                } else {
                    track.title.clone()
                };
                track_col = track_col.push(
                    widget::button::text(track_label)
                        .on_press(TagEditorMessage::SelectTrack(*orig_idx))
                        .width(Length::Fill),
                );
            }
            col = col.push(track_col);
        } else {
            col = col.push(btn);
        }
    }
    col.into()
}

// ── Right panel: tabbed interface ───────────────────────────────────────────

fn build_right_panel<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    // Tab bar
    let tab_bar = widget::row()
        .push(tab_button(
            fl!("tag-editor-tab-tags"),
            state.active_tab == TagEditorTab::Tags,
            TagEditorMessage::SwitchTab(TagEditorTab::Tags),
        ))
        .push(tab_button(
            fl!("tag-editor-tab-lookup"),
            state.active_tab == TagEditorTab::Lookup,
            TagEditorMessage::SwitchTab(TagEditorTab::Lookup),
        ))
        .push(tab_button(
            fl!("tag-editor-tab-info"),
            state.active_tab == TagEditorTab::Info,
            TagEditorMessage::SwitchTab(TagEditorTab::Info),
        ))
        .spacing(4);

    let tab_content: cosmic::Element<'_, TagEditorMessage> = match state.active_tab {
        TagEditorTab::Tags => build_tags_tab(state),
        TagEditorTab::Lookup => build_lookup_tab(state),
        TagEditorTab::Info => build_info_tab(state),
    };

    widget::container(
        widget::column()
            .push(tab_bar)
            .push(widget::Space::new(Length::Shrink, Length::Fixed(8.0)))
            .push(tab_content)
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .padding([16, 20, 16, 20])
    .width(Length::FillPortion(3))
    .height(Length::Fill)
    .into()
}

// ── Tags tab ────────────────────────────────────────────────────────────────

fn build_tags_tab<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    if state.selected_index.is_none() && state.selected_album_name.is_none() {
        return widget::container(
            widget::text::body(fl!("tag-editor-select-track"))
                .apply(widget::container)
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .into();
    }

    let mut col = widget::column().spacing(12);

    col = col
        .push(labeled_field(
            fl!("tag-editor-title"),
            state.edit_title,
            TagEditorMessage::TitleChanged,
        ))
        .push(labeled_field(
            fl!("tag-editor-artist"),
            state.edit_artist,
            TagEditorMessage::ArtistChanged,
        ))
        .push(labeled_field(
            fl!("tag-editor-album"),
            state.edit_album,
            TagEditorMessage::AlbumChanged,
        ))
        .push(labeled_field(
            fl!("tag-editor-album-artist"),
            state.edit_album_artist,
            TagEditorMessage::AlbumArtistChanged,
        ))
        .push(
            widget::row()
                .push(labeled_field(
                    fl!("tag-editor-year"),
                    state.edit_year,
                    TagEditorMessage::YearChanged,
                ))
                .push(labeled_field(
                    fl!("tag-editor-track-number"),
                    state.edit_track_number,
                    TagEditorMessage::TrackNumberChanged,
                ))
                .push(labeled_field(
                    fl!("tag-editor-disc-number"),
                    state.edit_disc_number,
                    TagEditorMessage::DiscNumberChanged,
                ))
                .spacing(12),
        )
        .push(labeled_field(
            fl!("tag-editor-genre"),
            state.edit_genre,
            TagEditorMessage::GenreChanged,
        ))
        .push(labeled_field(
            fl!("tag-editor-comment"),
            state.edit_comment,
            TagEditorMessage::CommentChanged,
        ));

    if let Some(status) = state.save_status {
        col = col.push(widget::text::body(status));
    }

    // Save buttons
    let save_row = if state.album_mode && state.selected_album_name.is_some() {
        widget::row()
            .push(
                widget::button::suggested(fl!("tag-editor-save"))
                    .on_press_maybe(state.dirty.then_some(TagEditorMessage::Save)),
            )
            .push(
                widget::button::standard(fl!("tag-editor-batch-save"))
                    .on_press(TagEditorMessage::BatchSave),
            )
            .spacing(8)
    } else {
        widget::row().push(
            widget::button::suggested(fl!("tag-editor-save"))
                .on_press_maybe(state.dirty.then_some(TagEditorMessage::Save)),
        )
    };

    col = col.push(save_row);

    widget::scrollable(col)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

// ── Lookup tab ──────────────────────────────────────────────────────────────

fn build_lookup_tab<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    let mut col = widget::column().spacing(12);

    // Source selector row
    let source_row = widget::row()
        .push(source_button(
            fl!("tag-editor-lookup-source-acoustid"),
            state.lookup_source == LookupSource::AcoustId,
            TagEditorMessage::LookupSourceChanged(LookupSource::AcoustId),
        ))
        .push(source_button(
            fl!("tag-editor-lookup-source-musicbrainz"),
            state.lookup_source == LookupSource::MusicBrainz,
            TagEditorMessage::LookupSourceChanged(LookupSource::MusicBrainz),
        ))
        .push(source_button(
            fl!("tag-editor-lookup-source-discogs"),
            state.lookup_source == LookupSource::Discogs,
            TagEditorMessage::LookupSourceChanged(LookupSource::Discogs),
        ))
        .spacing(4);

    col = col.push(source_row);

    // AcoustID: fingerprint scan button instead of text search
    if state.lookup_source == LookupSource::AcoustId {
        let scan_label = if state.fingerprinting {
            fl!("tag-editor-lookup-scanning")
        } else {
            fl!("tag-editor-lookup-scan-fingerprint")
        };
        let scan_btn = widget::button::suggested(scan_label).on_press_maybe(
            (!state.fingerprinting && state.selected_index.is_some())
                .then_some(TagEditorMessage::ScanFingerprint),
        );
        col = col.push(scan_btn);
    } else {
        // Text search for MusicBrainz / Discogs
        let search_row = widget::row()
            .push(
                widget::text_input(fl!("tag-editor-lookup-search"), state.lookup_query)
                    .on_input(TagEditorMessage::LookupQueryChanged)
                    .on_submit(|_| TagEditorMessage::LookupSearch)
                    .width(Length::Fill),
            )
            .push(
                widget::button::suggested(fl!("tag-editor-lookup-search-btn")).on_press_maybe(
                    (!state.lookup_loading && !state.lookup_query.is_empty())
                        .then_some(TagEditorMessage::LookupSearch),
                ),
            )
            .spacing(8);
        col = col.push(search_row);
    }

    // Loading indicator
    if state.lookup_loading || state.fingerprinting {
        col = col.push(widget::text::body(fl!("tag-editor-lookup-searching")));
    }

    // Results list
    if !state.lookup_results.is_empty() {
        col = col.push(widget::text::body(fl!(
            "tag-editor-lookup-results",
            count = state.lookup_results.len().to_string()
        )));

        let mut results_col = widget::column().spacing(4);
        for (i, release) in state.lookup_results.iter().enumerate() {
            let is_selected = state.selected_result == Some(i);
            let label = format!(
                "{} — {} {}",
                release.artist,
                release.title,
                release
                    .year
                    .as_ref()
                    .map(|y| format!("({})", y))
                    .unwrap_or_default()
            );
            let btn = widget::button::text(label)
                .on_press(TagEditorMessage::SelectResult(i))
                .width(Length::Fill);

            if is_selected {
                results_col = results_col.push(
                    widget::container(btn)
                        .class(cosmic::theme::Container::Primary)
                        .width(Length::Fill),
                );
            } else {
                results_col = results_col.push(btn);
            }
        }

        col = col.push(
            widget::scrollable(results_col)
                .height(Length::FillPortion(2))
                .width(Length::Fill),
        );

        // Selected result details + apply button
        if let Some(idx) = state.selected_result
            && let Some(release) = state.lookup_results.get(idx)
        {
            col = col.push(build_result_detail(release, state.album_mode));
        }
    } else if !state.lookup_loading && !state.fingerprinting {
        // API key configuration section
        col = col.push(widget::Space::new(Length::Shrink, Length::Fixed(16.0)));
        col = col.push(widget::text::heading(fl!("tag-editor-api-keys")));
        col = col.push(widget::text::body(fl!("tag-editor-api-keys-hint")));
        col = col.push(labeled_field(
            fl!("tag-editor-acoustid-api-key"),
            state.acoustid_api_key,
            TagEditorMessage::AcoustIdKeyChanged,
        ));
        col = col.push(labeled_field(
            fl!("tag-editor-discogs-token"),
            state.discogs_token,
            TagEditorMessage::DiscogsTokenChanged,
        ));
    }

    widget::scrollable(col)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

fn build_result_detail<'a>(
    release: &LookupRelease,
    album_mode: bool,
) -> cosmic::Element<'a, TagEditorMessage> {
    let mut col = widget::column().spacing(8);

    col = col.push(widget::text::heading(format!(
        "{} — {}",
        release.artist, release.title
    )));

    if let Some(year) = &release.year {
        col = col.push(widget::text::body(format!(
            "{}: {}",
            fl!("tag-editor-year"),
            year
        )));
    }
    if let Some(label) = &release.label {
        col = col.push(widget::text::body(format!(
            "{}: {}",
            fl!("tag-editor-lookup-label"),
            label
        )));
    }
    if !release.genres.is_empty() {
        col = col.push(widget::text::body(format!(
            "{}: {}",
            fl!("tag-editor-genre"),
            release.genres.join(", ")
        )));
    }

    // Track listing
    if !release.tracks.is_empty() {
        col = col.push(widget::text::body(format!(
            "{}: {}",
            fl!("tag-editor-lookup-tracks"),
            release.tracks.len()
        )));
        let mut track_col = widget::column().spacing(2);
        for track in &release.tracks {
            let dur = track
                .duration_ms
                .map(|ms| {
                    let s = ms / 1000;
                    format!(" ({}:{:02})", s / 60, s % 60)
                })
                .unwrap_or_default();
            track_col = track_col.push(widget::text::body(format!(
                "  {}. {} — {}{}",
                track.position, track.title, track.artist, dur
            )));
        }
        col = col.push(track_col);
    } else {
        // Offer to fetch full tracklist
        col = col.push(
            widget::button::standard(fl!("tag-editor-lookup-fetch-tracks"))
                .on_press(TagEditorMessage::FetchReleaseTracks(release.id.clone())),
        );
    }

    // Apply button
    let apply_label = if album_mode {
        fl!("tag-editor-lookup-apply-album")
    } else {
        fl!("tag-editor-lookup-apply")
    };
    col = col.push(widget::button::suggested(apply_label).on_press(TagEditorMessage::ApplyResult));

    col.into()
}

// ── Info tab ────────────────────────────────────────────────────────────────

fn build_info_tab<'a>(state: &TagEditorState<'a>) -> cosmic::Element<'a, TagEditorMessage> {
    if state.selected_index.is_none() {
        return widget::container(
            widget::text::body(fl!("tag-editor-select-track"))
                .apply(widget::container)
                .align_x(cosmic::iced::alignment::Horizontal::Center)
                .align_y(cosmic::iced::alignment::Vertical::Center)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .into();
    }

    // Find the selected track to show file info
    let local_tracks: Vec<&Track> = state
        .all_tracks
        .iter()
        .filter(|t| t.provider_id.as_ref() == "local")
        .collect();

    let track = state
        .selected_index
        .and_then(|idx| local_tracks.get(idx).copied());

    let Some(track) = track else {
        return widget::text::body(fl!("tag-editor-select-track")).into();
    };

    let mut col = widget::column().spacing(12);

    col = col.push(info_row(
        fl!("tag-editor-info-path"),
        track.path.display().to_string(),
    ));

    // Determine codec from file extension
    let codec = track
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_uppercase();
    col = col.push(info_row(fl!("tag-editor-info-codec"), codec));

    if track.bitrate > 0 {
        col = col.push(info_row(
            fl!("tag-editor-info-bitrate"),
            format!("{} kbps", track.bitrate),
        ));
    }

    if track.sample_rate > 0 {
        col = col.push(info_row(
            fl!("tag-editor-info-sample-rate"),
            format!("{} Hz", track.sample_rate),
        ));
    }

    let secs = track.duration.as_secs();
    col = col.push(info_row(
        fl!("tag-editor-info-duration"),
        format!("{}:{:02}", secs / 60, secs % 60),
    ));

    // File size
    if let Ok(metadata) = std::fs::metadata(&track.path) {
        let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
        col = col.push(info_row(
            fl!("tag-editor-info-file-size"),
            format!("{:.1} MB", size_mb),
        ));
    }

    widget::scrollable(col)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

// ── Helper widgets ──────────────────────────────────────────────────────────

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

fn info_row<'a>(label: String, value: String) -> cosmic::Element<'a, TagEditorMessage> {
    widget::row()
        .push(
            widget::text::body(label)
                .apply(widget::container)
                .width(Length::FillPortion(1)),
        )
        .push(
            widget::text::body(value)
                .apply(widget::container)
                .width(Length::FillPortion(2)),
        )
        .spacing(8)
        .width(Length::Fill)
        .into()
}

fn tab_button<'a>(
    label: String,
    active: bool,
    msg: TagEditorMessage,
) -> cosmic::Element<'a, TagEditorMessage> {
    if active {
        widget::container(widget::button::suggested(label).on_press(msg)).into()
    } else {
        widget::container(widget::button::standard(label).on_press(msg)).into()
    }
}

fn mode_button<'a>(
    label: String,
    active: bool,
    msg: TagEditorMessage,
) -> cosmic::Element<'a, TagEditorMessage> {
    if active {
        widget::button::suggested(label)
            .on_press(msg)
            .width(Length::FillPortion(1))
            .into()
    } else {
        widget::button::standard(label)
            .on_press(msg)
            .width(Length::FillPortion(1))
            .into()
    }
}

fn source_button<'a>(
    label: String,
    active: bool,
    msg: TagEditorMessage,
) -> cosmic::Element<'a, TagEditorMessage> {
    if active {
        widget::button::suggested(label).on_press(msg).into()
    } else {
        widget::button::standard(label).on_press(msg).into()
    }
}
