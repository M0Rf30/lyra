// SPDX-License-Identifier: GPL-3.0

//! Provider settings view — configure MPD and Subsonic servers.

use crate::fl;
use cosmic::iced::Alignment;
use cosmic::iced_core::Color;
use cosmic::prelude::*;
use cosmic::widget;

// ── MPD editing state ──────────────────────────────────────────────────────

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

// ── Subsonic editing state ─────────────────────────────────────────────────

/// Editing state for a single Subsonic/Navidrome server entry.
#[derive(Debug, Clone)]
pub struct SubsonicEditState {
    pub id: String,
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub accept_invalid_certs: bool,
}

impl SubsonicEditState {
    /// Create from a config entry.
    pub fn from_config(entry: &crate::config::SubsonicConfigEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            url: entry.url.clone(),
            username: entry.username.clone(),
            password: entry.password.clone().unwrap_or_default(),
            accept_invalid_certs: entry.accept_invalid_certs,
        }
    }

    /// Convert back to a config entry.
    pub fn to_config(&self) -> crate::config::SubsonicConfigEntry {
        crate::config::SubsonicConfigEntry {
            id: self.id.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            username: self.username.clone(),
            password: if self.password.is_empty() {
                None
            } else {
                Some(self.password.clone())
            },
            accept_invalid_certs: self.accept_invalid_certs,
        }
    }

    /// Create a new empty entry with a generated id.
    pub fn new_default(index: usize) -> Self {
        Self {
            id: format!("subsonic-{index}"),
            name: "Subsonic Server".to_string(),
            url: "https://".to_string(),
            username: String::new(),
            password: String::new(),
            accept_invalid_certs: false,
        }
    }
}

// ── Messages ───────────────────────────────────────────────────────────────

/// Messages emitted by the providers settings view.
#[derive(Debug, Clone)]
pub enum ProvidersMessage {
    // MPD
    AddMpd,
    EditName(usize, String),
    EditHost(usize, String),
    EditPort(usize, String),
    EditPassword(usize, String),
    Save(usize),
    Remove(usize),
    TestConnection(usize),

    // Subsonic
    AddSubsonic,
    SubsonicEditName(usize, String),
    SubsonicEditUrl(usize, String),
    SubsonicEditUsername(usize, String),
    SubsonicEditPassword(usize, String),
    SubsonicToggleCerts(usize, bool),
    SubsonicSave(usize),
    SubsonicRemove(usize),
    SubsonicTestConnection(usize),
}

// ── View ───────────────────────────────────────────────────────────────────

/// Render the providers settings panel (shown in the context drawer).
pub fn providers_view<'a>(
    mpd_servers: &'a [MpdEditState],
    mpd_connection_status: &'a [Option<String>],
    subsonic_servers: &'a [SubsonicEditState],
    subsonic_connection_status: &'a [Option<String>],
) -> cosmic::Element<'a, ProvidersMessage> {
    let mut col = widget::column().spacing(16).padding(16);

    let has_any = !mpd_servers.is_empty() || !subsonic_servers.is_empty();

    if !has_any {
        col = col.push(widget::text::body(fl!("no-providers")));
    }

    // MPD servers
    for (i, server) in mpd_servers.iter().enumerate() {
        let status = mpd_connection_status.get(i).and_then(|s| s.as_deref());
        col = col.push(mpd_server_card(i, server, status));
    }

    // Subsonic servers
    for (i, server) in subsonic_servers.iter().enumerate() {
        let status = subsonic_connection_status.get(i).and_then(|s| s.as_deref());
        col = col.push(subsonic_server_card(i, server, status));
    }

    // Add buttons
    col = col.push(
        widget::row()
            .push(
                widget::button::standard(fl!("add-mpd-server")).on_press(ProvidersMessage::AddMpd),
            )
            .push(
                widget::button::standard(fl!("add-subsonic-server"))
                    .on_press(ProvidersMessage::AddSubsonic),
            )
            .spacing(8),
    );

    col.into()
}

// ── MPD card ───────────────────────────────────────────────────────────────

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

    if let Some(status) = connection_status {
        buttons = buttons.push(status_label(status));
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

// ── Subsonic card ──────────────────────────────────────────────────────────

fn subsonic_server_card<'a>(
    index: usize,
    server: &'a SubsonicEditState,
    connection_status: Option<&'a str>,
) -> cosmic::Element<'a, ProvidersMessage> {
    let name_input = widget::text_input(fl!("subsonic-name"), &server.name)
        .on_input(move |v| ProvidersMessage::SubsonicEditName(index, v));

    let url_input = widget::text_input(fl!("subsonic-url"), &server.url)
        .on_input(move |v| ProvidersMessage::SubsonicEditUrl(index, v));

    let username_input = widget::text_input(fl!("subsonic-username"), &server.username)
        .on_input(move |v| ProvidersMessage::SubsonicEditUsername(index, v));

    let password_input = widget::text_input(fl!("subsonic-password"), &server.password)
        .on_input(move |v| ProvidersMessage::SubsonicEditPassword(index, v));

    let tls_toggle = widget::toggler(server.accept_invalid_certs)
        .label(fl!("subsonic-accept-invalid-certs"))
        .on_toggle(move |v| ProvidersMessage::SubsonicToggleCerts(index, v));

    let mut buttons = widget::row().spacing(8).align_y(Alignment::Center);
    buttons = buttons.push(
        widget::button::standard(fl!("save")).on_press(ProvidersMessage::SubsonicSave(index)),
    );
    buttons = buttons.push(
        widget::button::standard(fl!("test-connection"))
            .on_press(ProvidersMessage::SubsonicTestConnection(index)),
    );
    buttons = buttons.push(
        widget::button::destructive(fl!("remove"))
            .on_press(ProvidersMessage::SubsonicRemove(index)),
    );

    if let Some(status) = connection_status {
        buttons = buttons.push(status_label(status));
    }

    widget::column()
        .push(widget::text::title4(format!("Subsonic: {}", &server.name)))
        .push(name_input)
        .push(url_input)
        .push(
            widget::row()
                .push(username_input)
                .push(password_input)
                .spacing(8),
        )
        .push(tls_toggle)
        .push(buttons)
        .push(widget::divider::horizontal::default())
        .spacing(8)
        .into()
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Render a connection status label with color coding.
///
/// Green for "Connected", red for anything else (connection failed + error).
fn status_label<'a, M: 'a>(status: &str) -> cosmic::Element<'a, M> {
    let connected_text = crate::fl!("connected");
    let is_connected = status == connected_text;

    let color = if is_connected {
        Color::from_rgb(0.2, 0.8, 0.2) // green
    } else {
        Color::from_rgb(0.9, 0.2, 0.2) // red
    };

    let dot = "● ";

    widget::text::caption(format!("{dot}{status}"))
        .class(cosmic::theme::Text::Color(color))
        .into()
}
