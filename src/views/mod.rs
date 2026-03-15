// SPDX-License-Identifier: GPL-3.0

pub mod albums;
pub mod artists;
pub mod equalizer;
pub mod genres;
pub mod lyrics;
pub mod now_playing;
pub mod playlists;
pub mod providers;
pub mod songs;

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Length};
use cosmic::iced_core::Background;
use cosmic::prelude::*;
use cosmic::widget;
use cosmic::widget::button::Style as ButtonStyle;

/// COSMIC spacing scale (from cosmic_theme::Spacing defaults).
///
/// Use these constants for all spacing/padding values to ensure consistency
/// with the COSMIC desktop design language.
pub mod spacing {
    /// 4px — tightest spacing (between related items like stars, track numbers).
    pub const XXXS: u16 = 4;
    /// 8px — extra-extra-small (track list row gaps, inner element spacing).
    pub const XXS: u16 = 8;
    /// 12px — extra-small (list item padding, small section gaps).
    pub const XS: u16 = 12;
    /// 16px — small (page padding, section spacing, card padding).
    pub const S: u16 = 16;
    /// 24px — medium (major section gaps, header spacing).
    pub const M: u16 = 24;
    /// 32px — large (page-level gaps, expanded view padding).
    pub const L: u16 = 32;
    /// 48px — extra-large.
    pub const XL: u16 = 48;
    /// 64px — extra-extra-large.
    pub const XXL: u16 = 64;
}

/// Render a consistent empty state placeholder.
///
/// Shows a large icon, a title, and a description centered in the available space.
pub fn empty_state<'a, M: 'static>(
    icon_name: &'static str,
    title: &'a str,
    description: &'a str,
) -> cosmic::Element<'a, M> {
    widget::container(
        widget::column()
            .push(widget::icon::from_name(icon_name).size(64))
            .push(widget::text::title3(title))
            .push(widget::text::body(description))
            .spacing(spacing::XS)
            .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .into()
}

/// Custom button class for grid cards (albums, genres).
///
/// Transparent background by default, subtle hover/pressed states using
/// component colors from the active COSMIC theme.
pub fn card_button_class() -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            ButtonStyle {
                background: None,
                text_color: Some(cosmic.background.component.on.into()),
                icon_color: Some(cosmic.background.component.on.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
        hovered: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            let comp = &cosmic.background.component;
            ButtonStyle {
                background: Some(Background::Color(comp.hover.into())),
                text_color: Some(comp.on.into()),
                icon_color: Some(comp.on.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
        pressed: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            let comp = &cosmic.background.component;
            ButtonStyle {
                background: Some(Background::Color(comp.pressed.into())),
                text_color: Some(comp.on.into()),
                icon_color: Some(comp.on.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
        disabled: Box::new(|theme| {
            let cosmic = theme.cosmic();
            ButtonStyle {
                background: None,
                text_color: Some(cosmic.background.component.on_disabled.into()),
                icon_color: Some(cosmic.background.component.on_disabled.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
    }
}

/// Truncate a string to `max_chars` characters, appending an ellipsis if truncated.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}
