// SPDX-License-Identifier: GPL-3.0

//! Provider settings view — configure MPD (and future Subsonic) servers.

use crate::fl;
use cosmic::iced::Alignment;
use cosmic::prelude::*;
use cosmic::widget;

/// Editing state for a single MPD server entry.
///
/// Kept as strings so text inputs can bind directly without
/// lifetime issues around temporary conversions.
#[derive(Debug, Clone)]
pub struct MpdEditState {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: String,
    pub password: String,
}

impl MpdEditState {
    /// Create from a config entry.
    pub fn from_config(entry: &crate::config::MpdConfigEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            host: entry.host.clone(),
            port: entry.port.to_string(),
            password: entry.password.clone().unwrap_or_default(),
        }
    }

    /// Convert back to a config entry (port defaults to 6600 on parse failure).
    pub fn to_config(&self) -> crate::config::MpdConfigEntry {
        crate::config::MpdConfigEntry {
            id: self.id.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port.parse().unwrap_or(6600),
            password: if self.password.is_empty() {
                None
            } else {
                Some(self.password.clone())
            },
        }
    }

    /// Create a new empty entry with a generated id.
    pub fn new_default(index: usize) -> Self {
        Self {
            id: format!("mpd-{index}"),
            name: "MPD Server".to_string(),
            host: "localhost".to_string(),
            port: "6600".to_string(),
            password: String::new(),
        }
    }
}

/// Messages emitted by the providers settings view.
#[derive(Debug, Clone)]
pub enum ProvidersMessage {
    /// User wants to add a new MPD server (shows the form with defaults).
    AddMpd,
    /// Editing field changed for the server at the given index.
    EditName(usize, String),
    EditHost(usize, String),
    EditPort(usize, String),
    EditPassword(usize, String),
    /// Save changes for the server at the given index.
    Save(usize),
    /// Remove the server at the given index.
    Remove(usize),
    /// Test connection for the server at the given index.
    TestConnection(usize),
}

/// Render the providers settings panel (shown in the context drawer).
///
/// `servers` is the current list of MPD edit states.
/// `connection_status` maps server index to an optional status string.
pub fn providers_view<'a>(
    servers: &'a [MpdEditState],
    connection_status: &'a [Option<String>],
) -> cosmic::Element<'a, ProvidersMessage> {
    let mut col = widget::column().spacing(16).padding(16);

    if servers.is_empty() {
        col = col.push(widget::text::body(fl!("no-providers")));
    }

    for (i, server) in servers.iter().enumerate() {
        let status = connection_status.get(i).and_then(|s| s.as_deref());
        let card = mpd_server_card(i, server, status);
        col = col.push(card);
    }

    // "Add MPD Server" button
    col = col
        .push(widget::button::standard(fl!("add-mpd-server")).on_press(ProvidersMessage::AddMpd));

    col.into()
}

/// Render a single MPD server configuration card.
fn mpd_server_card<'a>(
    index: usize,
    server: &'a MpdEditState,
    connection_status: Option<&'a str>,
) -> cosmic::Element<'a, ProvidersMessage> {
    let name_input = widget::text_input(fl!("mpd-name"), &server.name)
        .on_input(move |v| ProvidersMessage::EditName(index, v));

    let host_input = widget::text_input(fl!("mpd-host"), &server.host)
        .on_input(move |v| ProvidersMessage::EditHost(index, v));

    let port_input = widget::text_input(fl!("mpd-port"), &server.port)
        .on_input(move |v| ProvidersMessage::EditPort(index, v));

    let password_input = widget::text_input(fl!("mpd-password"), &server.password)
        .on_input(move |v| ProvidersMessage::EditPassword(index, v));

    let mut buttons = widget::row().spacing(8).align_y(Alignment::Center);

    buttons =
        buttons.push(widget::button::standard(fl!("save")).on_press(ProvidersMessage::Save(index)));

    buttons = buttons.push(
        widget::button::standard(fl!("test-connection"))
            .on_press(ProvidersMessage::TestConnection(index)),
    );

    buttons = buttons
        .push(widget::button::destructive(fl!("remove")).on_press(ProvidersMessage::Remove(index)));

    // Status indicator
    if let Some(status) = connection_status {
        buttons = buttons.push(widget::text::caption(status.to_string()));
    }

    widget::column()
        .push(widget::text::title4(format!("MPD: {}", &server.name)))
        .push(name_input)
        .push(host_input)
        .push(
            widget::row()
                .push(port_input)
                .push(password_input)
                .spacing(8),
        )
        .push(buttons)
        .push(widget::divider::horizontal::default())
        .spacing(8)
        .into()
}
