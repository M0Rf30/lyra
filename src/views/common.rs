// SPDX-License-Identifier: GPL-3.0

//! Shared UI helpers for library views.
//!
//! Every view renders durations, single-line table cells, star ratings and
//! favorite toggles through these helpers so the whole app stays visually
//! consistent.

use std::borrow::Cow;

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::core::text::Wrapping;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::widget::tooltip::Position as TooltipPosition;

/// Concrete text widget type returned by `cosmic::widget::text::*` helpers.
pub type Text<'a> = cosmic::iced::widget::Text<'a, cosmic::Theme, cosmic::Renderer>;

/// Fixed width for right-aligned duration cells (fits `H:MM:SS`).
pub const DURATION_WIDTH: f32 = 64.0;

/// Format a duration in whole seconds as `H:MM:SS` when at least an hour,
/// otherwise `M:SS`.
#[must_use]
pub fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Coarse human-readable duration for headers: `2h 44m`, `44m`, `12s`.
#[must_use]
pub fn format_duration_coarse(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{total_secs}s")
    }
}

/// Truncate on a `char` boundary, appending `…` when the input exceeds
/// `max_chars`.
#[must_use]
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", truncated.trim_end())
    }
}

/// Body text pinned to a single line: clipped at the cell edge, never wraps.
pub fn cell_text<'a>(content: impl Into<Cow<'a, str>> + 'a) -> Text<'a> {
    widget::text::body(content).wrapping(Wrapping::None)
}

/// Caption (small, dim) text pinned to a single line.
pub fn cell_caption<'a>(content: impl Into<Cow<'a, str>> + 'a) -> Text<'a> {
    widget::text::caption(content).wrapping(Wrapping::None)
}

/// Right-aligned, fixed-width duration cell for track rows.
pub fn duration_cell<'a, M: 'a>(total_secs: u64) -> cosmic::Element<'a, M> {
    widget::container(cell_caption(format_duration(total_secs)))
        .width(Length::Fixed(DURATION_WIDTH))
        .align_x(Horizontal::Right)
        .into()
}

/// Compact interactive five-star rating (roughly 100px wide).
///
/// Clicking a star sets the rating; clicking the current rating clears it
/// (sends `0`). Filled stars are tinted with the accent color.
pub fn star_rating<'a, M: Clone + 'static>(
    rating: Option<u8>,
    on_rate: impl Fn(u8) -> M,
) -> cosmic::Element<'a, M> {
    let current = rating.unwrap_or(0);
    let mut row = widget::Row::new().align_y(Alignment::Center);
    for star in 1u8..=5 {
        let icon_name = if star <= current {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        };
        let new_rating = if star == current { 0 } else { star };
        row = row.push(
            widget::button::icon(widget::icon::from_name(icon_name).size(14))
                .padding(2)
                .selected(star <= current)
                .on_press(on_rate(new_rating)),
        );
    }
    row.into()
}

/// Heart-shaped favorite toggle with a tooltip.
///
/// Accent-tinted when favorited, dim otherwise — visually distinct from the
/// star rating so the two can never be confused.
pub fn favorite_button<'a, M: Clone + 'static>(
    is_favorite: bool,
    on_toggle: M,
) -> cosmic::Element<'a, M> {
    let button = widget::button::icon(widget::icon::from_name("emblem-favorite-symbolic").size(16))
        .padding(4)
        .selected(is_favorite)
        .on_press(on_toggle);
    widget::tooltip(
        button,
        widget::text::caption(if is_favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        }),
        TooltipPosition::Top,
    )
    .into()
}

/// Icon button wrapped in a caption tooltip — for transport/utility controls.
pub fn icon_button<'a, M: Clone + 'static>(
    icon_name: &'static str,
    icon_size: u16,
    label: &'a str,
    on_press: M,
) -> cosmic::Element<'a, M> {
    widget::tooltip(
        widget::button::icon(widget::icon::from_name(icon_name).size(icon_size)).on_press(on_press),
        widget::text::caption(label),
        TooltipPosition::Top,
    )
    .into()
}

/// Centered empty-state placeholder with an icon, title and hint.
pub fn empty_state<'a, M: 'static>(
    icon_name: &'static str,
    title: impl Into<Cow<'a, str>> + 'a,
    subtitle: impl Into<Cow<'a, str>> + 'a,
) -> cosmic::Element<'a, M> {
    widget::container(
        widget::Column::new()
            .push(widget::icon::from_name(icon_name).size(64))
            .push(widget::text::title3(title))
            .push(widget::text::body(subtitle))
            .spacing(8)
            .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::{format_duration, format_duration_coarse, truncate_str};

    #[test]
    fn format_duration_sub_minute() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59), "0:59");
    }

    #[test]
    fn format_duration_minutes_only() {
        assert_eq!(format_duration(60), "1:00");
        assert_eq!(format_duration(3599), "59:59");
    }

    #[test]
    fn format_duration_hour_boundary() {
        assert_eq!(format_duration(3600), "1:00:00");
    }

    #[test]
    fn format_duration_regression_164_minutes() {
        // 9860s = 2h 44m 20s. Previously rendered as the bogus "164:20"
        // because minutes were never rolled over into hours.
        assert_eq!(format_duration(9860), "2:44:20");
    }

    #[test]
    fn format_duration_coarse_seconds_only() {
        assert_eq!(format_duration_coarse(12), "12s");
    }

    #[test]
    fn format_duration_coarse_minutes_only() {
        assert_eq!(format_duration_coarse(300), "5m");
    }

    #[test]
    fn format_duration_coarse_hours_and_minutes() {
        assert_eq!(format_duration_coarse(9860), "2h 44m");
    }

    #[test]
    fn format_duration_coarse_hour_boundary_zero_minutes() {
        assert_eq!(format_duration_coarse(3600), "1h 00m");
    }

    #[test]
    fn truncate_str_shorter_than_max_is_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exact_max_is_unchanged() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_over_max_ends_with_ellipsis_and_respects_length() {
        let result = truncate_str("hello world", 8);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 8);
    }

    #[test]
    fn truncate_str_trims_trailing_whitespace_before_ellipsis() {
        // Cutting right after "hello" lands on trailing spaces; they must be
        // trimmed so the ellipsis doesn't float after visible whitespace.
        let result = truncate_str("hello   world", 8);
        assert_eq!(result, "hello…");
    }

    #[test]
    fn truncate_str_multibyte_safe_no_panic() {
        // Multibyte (non-ASCII) chars must be counted and sliced by `char`,
        // never by byte, or this would panic on a non-boundary split.
        let cjk = "日本語のテスト";
        let result = truncate_str(cjk, 4);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 4);

        let accented = "ééééééééé";
        let result = truncate_str(accented, 5);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 5);
    }
}
