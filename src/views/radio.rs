// SPDX-License-Identifier: GPL-3.0

//! Radio view - saved internet radio stations with add-by-URL and a
//! radio-browser.info directory search.

use crate::fl;
use crate::online::radio::StationSearchResult;
use crate::online::store::RadioStation;
use crate::views::common;
use crate::views::list_row_button_class;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use std::collections::HashMap;

/// Messages from the radio view.
#[derive(Debug, Clone)]
pub enum RadioMessage {
    /// The radio-browser directory search query changed.
    SearchChanged(String),
    /// Run the directory search for the current query.
    SearchSubmit,
    /// Fetch globally popular stations into the same search-results list.
    Discover,
    /// The add-by-URL name field changed.
    AddNameChanged(String),
    /// The add-by-URL stream URL field changed.
    AddUrlChanged(String),
    /// Save the station from the add-by-URL fields.
    AddByUrl(String, String),
    /// Save a station found via directory search, by its index.
    AddFromSearch(usize),
    /// Remove a saved station by its index.
    RemoveStation(usize),
    /// Play a saved station by its index.
    PlayStation(usize),
    /// Play a directory search result by its index.
    PlaySearchResult(usize),
}

fn station_icon<'a, M: 'a + 'static>(
    favicon_url: &str,
    icons: &HashMap<String, widget::icon::Handle>,
    size: u16,
) -> cosmic::Element<'a, M> {
    match icons.get(favicon_url) {
        Some(handle) => widget::icon::icon(handle.clone()).size(size).into(),
        None => widget::icon::from_name("network-wireless-symbolic").size(size).into(),
    }
}

/// Render the radio view: saved stations, add-by-URL, and directory search.
pub fn radio_view<'a>(
    stations: &'a [RadioStation],
    search_query: &'a str,
    search_results: &'a [StationSearchResult],
    search_loading: bool,
    add_name: &'a str,
    add_url: &'a str,
    icons: &'a HashMap<String, widget::icon::Handle>,
    current_stream_url: Option<&'a str>,
) -> cosmic::Element<'a, RadioMessage> {
    let mut col = widget::Column::new().spacing(12).padding(16);

    let search_row = widget::Row::new()
        .push(
            widget::text_input(fl!("radio-search-placeholder"), search_query)
                .on_input(RadioMessage::SearchChanged)
                .on_submit_maybe(if search_query.trim().is_empty() {
                    None
                } else {
                    Some(|_| RadioMessage::SearchSubmit)
                })
                .width(Length::Fill),
        )
        .push(widget::button::standard(fl!("search")).on_press_maybe(
            if search_query.trim().is_empty() {
                None
            } else {
                Some(RadioMessage::SearchSubmit)
            },
        ))
        .spacing(8)
        .align_y(Alignment::Center);
    col = col.push(search_row);

    // Only offer discovery while the search box is idle and empty, so it
    // never competes with an active name search for the same results list.
    if search_query.trim().is_empty() && search_results.is_empty() {
        col = col.push(
            widget::button::standard(fl!("radio-discover")).on_press(RadioMessage::Discover),
        );
    }

    if search_loading {
        col = col.push(common::cell_caption(fl!("searching")));
    } else if !search_results.is_empty() {
        let mut results_col = widget::Column::new().spacing(2);
        for (index, result) in search_results.iter().enumerate() {
            let info = widget::Column::new()
                .push(common::cell_text(result.name.as_str()))
                .push(common::cell_caption(format!(
                    "{}  -  {} kbps  -  {}",
                    result.codec, result.bitrate, result.tags
                )))
                .spacing(2);
            let row = widget::Row::new()
                .push(station_icon(&result.favicon, icons, 32))
                .push(common::clipped_cell(info.into()))
                .push(widget::tooltip(
                    widget::button::icon(
                        widget::icon::from_name("media-playback-start-symbolic").size(16),
                    )
                    .on_press(RadioMessage::PlaySearchResult(index)),
                    widget::text::caption(fl!("play-station-tooltip")),
                    widget::tooltip::Position::Top,
                ))
                .push(
                    widget::button::standard(fl!("add-station"))
                        .on_press(RadioMessage::AddFromSearch(index)),
                )
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(8);
            results_col = results_col.push(row);
        }
        col = col.push(widget::container(results_col).width(Length::Fill));
    }

    col = col.push(widget::divider::horizontal::default());

    let add_row = widget::Row::new()
        .push(
            widget::text_input(fl!("station-name-placeholder"), add_name)
                .on_input(RadioMessage::AddNameChanged)
                .width(Length::FillPortion(1)),
        )
        .push(
            widget::text_input(fl!("station-url-placeholder"), add_url)
                .on_input(RadioMessage::AddUrlChanged)
                .width(Length::FillPortion(2)),
        )
        .push(
            widget::button::suggested(fl!("add-station")).on_press_maybe(
                if add_url.trim().is_empty() {
                    None
                } else {
                    Some(RadioMessage::AddByUrl(add_name.to_string(), add_url.to_string()))
                },
            ),
        )
        .spacing(8)
        .align_y(Alignment::Center);
    col = col.push(add_row);
    col = col.push(widget::text::title4(fl!("my-stations")));

    if stations.is_empty() {
        col = col.push(common::empty_state(
            "network-wireless-symbolic",
            fl!("no-stations"),
            fl!("stations-empty-hint"),
        ));
        return col.into();
    }

    let mut list = widget::Column::new().spacing(2);
    for (index, station) in stations.iter().enumerate() {
        let is_current = current_stream_url == Some(station.stream_url.as_str());
        let info = widget::Column::new()
            .push(common::cell_text(station.name.as_str()))
            .push(common::cell_caption(station.tags.as_str()))
            .spacing(2);

        let remove_btn = widget::tooltip(
            widget::button::icon(widget::icon::from_name("edit-delete-symbolic").size(16))
                .class(cosmic::theme::Button::Destructive)
                .on_press(RadioMessage::RemoveStation(index)),
            widget::text::caption(fl!("remove-station-tooltip")),
            widget::tooltip::Position::Top,
        );

        let row = widget::button::custom(
            widget::Row::new()
                .push(station_icon(&station.favicon_url, icons, 32))
                .push(common::clipped_cell(info.into()))
                .push(remove_btn)
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(8),
        )
        .on_press(RadioMessage::PlayStation(index))
        .width(Length::Fill)
        .class(list_row_button_class(is_current));

        list = list.push(row);
    }

    col = col
        .push(widget::scrollable(widget::container(list).width(Length::Fill)).height(Length::Fill));
    col.into()
}
