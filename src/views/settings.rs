// SPDX-License-Identifier: GPL-3.0

//! Settings page view — playback settings (crossfade, replay gain).

use crate::fl;
use crate::views::spacing;
use cosmic::iced::Alignment;
use cosmic::prelude::*;
use cosmic::widget;

/// Messages emitted by the settings view.
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    /// Crossfade duration changed (seconds, 0 = disabled).
    SetCrossfade(f32),
    /// Replay gain mode changed.
    SetReplayGainMode(crate::config::ReplayGainMode),
}

/// Render the Settings page.
pub fn settings_view<'a>(
    crossfade_secs: f32,
    replay_gain_mode: crate::config::ReplayGainMode,
    active_provider_type: Option<crate::provider::ProviderType>,
) -> cosmic::Element<'a, SettingsMessage> {
    let show_crossfade = active_provider_type
        .map(|pt| {
            matches!(
                pt,
                crate::provider::ProviderType::Local | crate::provider::ProviderType::Mpd
            )
        })
        .unwrap_or(true);
    let show_replay_gain = active_provider_type
        .map(|pt| {
            matches!(
                pt,
                crate::provider::ProviderType::Local | crate::provider::ProviderType::Mpd
            )
        })
        .unwrap_or(true);

    let mut col = widget::column().spacing(spacing::XXS).padding(spacing::S);

    if show_crossfade || show_replay_gain {
        let mut playback_section =
            cosmic::widget::settings::section().title(fl!("playback-settings"));

        if show_crossfade {
            let crossfade_label = if crossfade_secs < 0.1 {
                fl!("crossfade-disabled")
            } else {
                fl!("crossfade-seconds", secs = format!("{:.0}", crossfade_secs))
            };

            let crossfade_control = widget::column()
                .push(widget::text::caption(crossfade_label))
                .push(
                    widget::slider(0.0..=12.0, crossfade_secs, SettingsMessage::SetCrossfade)
                        .step(0.5),
                )
                .spacing(spacing::XXXS);

            playback_section = playback_section.add(cosmic::widget::settings::flex_item(
                fl!("crossfade-duration"),
                crossfade_control,
            ));
        }

        if show_replay_gain {
            use crate::config::ReplayGainMode;

            let modes = [
                (ReplayGainMode::Off, fl!("replay-gain-off")),
                (ReplayGainMode::Track, fl!("replay-gain-track")),
                (ReplayGainMode::Album, fl!("replay-gain-album")),
                (ReplayGainMode::Auto, fl!("replay-gain-auto")),
            ];

            let mut mode_row = widget::row()
                .spacing(spacing::XXXS)
                .align_y(Alignment::Center);
            for (mode, label) in modes {
                let btn = if mode == replay_gain_mode {
                    widget::button::standard(label)
                } else {
                    widget::button::text(label)
                };
                mode_row = mode_row.push(btn.on_press(SettingsMessage::SetReplayGainMode(mode)));
            }

            playback_section = playback_section.add(cosmic::widget::settings::flex_item(
                fl!("replay-gain"),
                mode_row,
            ));
        }

        col = col.push(playback_section);
    }

    widget::scrollable(col)
        .width(cosmic::iced::Length::Fill)
        .height(cosmic::iced::Length::Fill)
        .into()
}
