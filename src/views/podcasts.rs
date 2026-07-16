// SPDX-License-Identifier: GPL-3.0

//! Podcasts view - subscription list with iTunes directory search and
//! add-by-URL, plus a detail view listing episodes for a subscribed show.

use crate::fl;
use crate::online::podcast::PodcastSearchResult;
use crate::online::store::{Episode, Podcast};
use crate::views::common;
use crate::views::list_row_button_class;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use std::collections::HashMap;

/// Messages from the podcasts view.
#[derive(Debug, Clone)]
pub enum PodcastMessage {
    /// The iTunes directory search query changed.
    SearchChanged(String),
    /// Run the directory search for the current query.
    SearchSubmit,
    /// The add-by-URL field changed.
    AddUrlChanged(String),
    /// Subscribe using the add-by-URL field's feed URL.
    AddByUrl(String),
    /// Subscribe to a feed URL found via directory search.
    SubscribeFromSearch(String),
    /// Open a subscribed podcast's episode list.
    SelectPodcast(usize),
    /// Return to the subscription list.
    BackToList,
    /// Unsubscribe from a podcast by its index in the list.
    RemovePodcast(usize),
    /// Re-fetch a single podcast's feed by index.
    RefreshPodcast(usize),
    /// Re-fetch every subscribed podcast's feed.
    RefreshAll,
    /// Play an episode by its index in the current detail view.
    PlayEpisode(usize),
    /// Toggle the played marker for an episode by index.
    TogglePlayed(usize),
    /// Download an episode's enclosure for offline playback, by index.
    Download(usize),
    /// Delete an episode's downloaded local file, by index.
    DeleteDownload(usize),
}

fn podcast_icon<'a, M: 'a + 'static>(
    image_url: &str,
    icons: &HashMap<String, widget::icon::Handle>,
    size: u16,
) -> cosmic::Element<'a, M> {
    match icons.get(image_url) {
        Some(handle) => widget::icon::icon(handle.clone()).size(size).into(),
        None => widget::icon::from_name("application-rss+xml-symbolic").size(size).into(),
    }
}

/// Render the podcast subscription list, with directory search and
/// add-by-URL above it.
pub fn podcast_list_view<'a>(
    podcasts: &'a [Podcast],
    search_query: &'a str,
    search_results: &'a [PodcastSearchResult],
    search_loading: bool,
    add_url: &'a str,
    icons: &'a HashMap<String, widget::icon::Handle>,
) -> cosmic::Element<'a, PodcastMessage> {
    let mut col = widget::Column::new().spacing(12).padding(16);

    let search_row = widget::Row::new()
        .push(
            widget::text_input(fl!("podcast-search-placeholder"), search_query)
                .on_input(PodcastMessage::SearchChanged)
                .on_submit_maybe(if search_query.trim().is_empty() {
                    None
                } else {
                    Some(|_| PodcastMessage::SearchSubmit)
                })
                .width(Length::Fill),
        )
        .push(widget::button::standard(fl!("search")).on_press_maybe(
            if search_query.trim().is_empty() {
                None
            } else {
                Some(PodcastMessage::SearchSubmit)
            },
        ))
        .spacing(8)
        .align_y(Alignment::Center);
    col = col.push(search_row);

    if search_loading {
        col = col.push(common::cell_caption(fl!("searching")));
    } else if !search_results.is_empty() {
        let mut results_col = widget::Column::new().spacing(2);
        for result in search_results {
            let info = widget::Column::new()
                .push(common::cell_text(result.title.as_str()))
                .push(common::cell_caption(result.author.as_str()))
                .spacing(2);
            let row = widget::Row::new()
                .push(podcast_icon(&result.image, icons, 40))
                .push(common::clipped_cell(info.into()))
                .push(
                    widget::button::suggested(fl!("subscribe"))
                        .on_press(PodcastMessage::SubscribeFromSearch(result.feed_url.clone())),
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
            widget::text_input(fl!("podcast-url-placeholder"), add_url)
                .on_input(PodcastMessage::AddUrlChanged)
                .on_submit_maybe(if add_url.trim().is_empty() {
                    None
                } else {
                    Some(|text: String| PodcastMessage::AddByUrl(text))
                })
                .width(Length::Fill),
        )
        .push(
            widget::button::suggested(fl!("subscribe")).on_press_maybe(
                if add_url.trim().is_empty() {
                    None
                } else {
                    Some(PodcastMessage::AddByUrl(add_url.to_string()))
                },
            ),
        )
        .spacing(8)
        .align_y(Alignment::Center);
    col = col.push(add_row);

    let header_row = widget::Row::new()
        .push(widget::text::title4(fl!("subscriptions")))
        .push(widget::Space::new().width(Length::Fill))
        .push(widget::button::standard(fl!("refresh-all")).on_press(PodcastMessage::RefreshAll))
        .align_y(Alignment::Center);
    col = col.push(header_row);

    if podcasts.is_empty() {
        col = col.push(common::empty_state(
            "application-rss+xml-symbolic",
            fl!("no-podcasts"),
            fl!("podcasts-empty-hint"),
        ));
        return col.into();
    }

    let mut list = widget::Column::new().spacing(2);
    for (index, podcast) in podcasts.iter().enumerate() {
        let info = widget::Column::new()
            .push(common::cell_text(podcast.title.as_str()))
            .push(common::cell_caption(podcast.description.as_str()))
            .spacing(2);

        let refresh_btn = widget::tooltip(
            widget::button::icon(widget::icon::from_name("view-refresh-symbolic").size(16))
                .on_press(PodcastMessage::RefreshPodcast(index)),
            widget::text::caption(fl!("refresh-podcast-tooltip")),
            widget::tooltip::Position::Top,
        );
        let remove_btn = widget::tooltip(
            widget::button::icon(widget::icon::from_name("edit-delete-symbolic").size(16))
                .class(cosmic::theme::Button::Destructive)
                .on_press(PodcastMessage::RemovePodcast(index)),
            widget::text::caption(fl!("unsubscribe-tooltip")),
            widget::tooltip::Position::Top,
        );

        let row = widget::button::custom(
            widget::Row::new()
                .push(podcast_icon(&podcast.image_url, icons, 40))
                .push(common::clipped_cell(info.into()))
                .push(refresh_btn)
                .push(remove_btn)
                .spacing(12)
                .align_y(Alignment::Center)
                .padding(8),
        )
        .on_press(PodcastMessage::SelectPodcast(index))
        .width(Length::Fill)
        .class(list_row_button_class(false));

        list = list.push(row);
    }

    col = col
        .push(widget::scrollable(widget::container(list).width(Length::Fill)).height(Length::Fill));
    col.into()
}

/// Render a subscribed podcast's episode list.
pub fn podcast_detail_view<'a>(
    podcast: &'a Podcast,
    episodes: &'a [Episode],
    current_episode_id: Option<i64>,
    icons: &'a HashMap<String, widget::icon::Handle>,
    downloading: &std::collections::HashSet<i64>,
) -> cosmic::Element<'a, PodcastMessage> {
    let header = widget::Row::new()
        .push(widget::tooltip(
            widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                .on_press(PodcastMessage::BackToList),
            widget::text::caption(fl!("back-to-podcasts")),
            widget::tooltip::Position::Top,
        ))
        .push(podcast_icon(&podcast.image_url, icons, 80))
        .push(
            widget::Column::new()
                .push(widget::text::title1(podcast.title.as_str()))
                .push(common::cell_caption(podcast.description.as_str()))
                .spacing(8)
                .width(Length::Fill),
        )
        .spacing(16)
        .align_y(Alignment::Center);

    let mut episode_list = widget::Column::new().spacing(2);
    for (index, episode) in episodes.iter().enumerate() {
        let is_current = current_episode_id == Some(episode.id);
        let date = format_pub_date(episode.pub_date);

        let mut info = widget::Column::new()
            .push(common::cell_text(episode.title.as_str()))
            .push(common::cell_caption(date))
            .spacing(2);
        if episode.position_ms > 0 && !episode.played {
            let resumed_at = common::format_duration_coarse((episode.position_ms / 1000).max(0) as u64);
            info = info.push(common::cell_caption(fl!("resume-at", position = resumed_at)));
        }

        let played_icon = if episode.played {
            "emblem-ok-symbolic"
        } else {
            "emblem-default-symbolic"
        };
        let played_btn = widget::tooltip(
            widget::button::icon(widget::icon::from_name(played_icon).size(16))
                .on_press(PodcastMessage::TogglePlayed(index)),
            widget::text::caption(fl!("mark-played-tooltip")),
            widget::tooltip::Position::Top,
        );

        let download_control: cosmic::Element<'a, PodcastMessage> =
            if !episode.downloaded_path.is_empty() {
                widget::Row::new()
                    .push(widget::icon::from_name("emblem-downloads-symbolic").size(16))
                    .push(widget::tooltip(
                        widget::button::icon(widget::icon::from_name("user-trash-symbolic").size(16))
                            .class(cosmic::theme::Button::Destructive)
                            .on_press(PodcastMessage::DeleteDownload(index)),
                        widget::text::caption(fl!("delete-download-tooltip")),
                        widget::tooltip::Position::Top,
                    ))
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .into()
            } else if downloading.contains(&episode.id) {
                widget::button::icon(widget::icon::from_name("content-loading-symbolic").size(16))
                    .on_press_maybe(None::<PodcastMessage>)
                    .into()
            } else {
                widget::tooltip(
                    widget::button::icon(widget::icon::from_name("document-save-symbolic").size(16))
                        .on_press(PodcastMessage::Download(index)),
                    widget::text::caption(fl!("download-episode-tooltip")),
                    widget::tooltip::Position::Top,
                )
                .into()
            };

        let row = widget::button::custom(
            widget::Row::new()
                .push(common::clipped_cell(info.into()))
                .push(common::duration_cell(episode.duration_secs.max(0) as u64))
                .push(played_btn)
                .push(download_control)
                .spacing(8)
                .width(Length::Fill)
                .align_y(Alignment::Center)
                .padding(4),
        )
        .on_press(PodcastMessage::PlayEpisode(index))
        .width(Length::Fill)
        .class(list_row_button_class(is_current));

        episode_list = episode_list.push(row);
    }

    if episodes.is_empty() {
        episode_list = episode_list.push(common::empty_state(
            "application-rss+xml-symbolic",
            fl!("no-episodes"),
            fl!("no-episodes-hint"),
        ));
    }

    widget::scrollable(
        widget::Column::new()
            .push(header)
            .push(widget::divider::horizontal::default())
            .push(episode_list)
            .spacing(16)
            .padding(16),
    )
    .height(Length::Fill)
    .into()
}

/// Format an episode's `pub_date` (epoch seconds) as `YYYY-MM-DD`, or an
/// empty string when unset.
fn format_pub_date(epoch_secs: i64) -> String {
    if epoch_secs <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(epoch_secs, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}
