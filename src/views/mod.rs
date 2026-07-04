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
pub mod tag_editor;

use cosmic::iced::core::Background;
use cosmic::widget::button::Style as ButtonStyle;

pub fn card_button_class() -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            ButtonStyle {
                background: None,
                text_color: Some(cosmic.background(false).component.on.into()),
                icon_color: Some(cosmic.background(false).component.on.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
        hovered: Box::new(|_focused, theme| {
            let cosmic = theme.cosmic();
            let comp = &cosmic.background(false).component;
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
            let comp = &cosmic.background(false).component;
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
                text_color: Some(cosmic.background(false).component.on_disabled.into()),
                icon_color: Some(cosmic.background(false).component.on_disabled.into()),
                border_radius: cosmic.corner_radii.radius_m.into(),
                ..ButtonStyle::new()
            }
        }),
    }
}
