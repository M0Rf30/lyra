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
    /// Gapless playback toggled.
    SetGaplessPlayback(bool),
    /// Replay gain fallback dB changed.
    SetReplayGainFallback(f32),
}

/// Render the Settings page.
pub fn settings_view<'a>(
    crossfade_secs: f32,
    replay_gain_mode: crate::config::ReplayGainMode,
    gapless_playback: bool,
    replay_gain_fallback_db: f32,
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

            // Gapless playback — mutually exclusive with crossfade
            let gapless_control: cosmic::Element<SettingsMessage> = if crossfade_secs > 0.0 {
                widget::text::caption("Disabled when crossfade is active").into()
            } else {
                widget::toggler(gapless_playback)
                    .on_toggle(SettingsMessage::SetGaplessPlayback)
                    .into()
            };

            playback_section = playback_section.add(
                cosmic::widget::settings::item::builder("Gapless Playback")
                    .description("Pre-queue next track for seamless transitions")
                    .control(gapless_control),
            );
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

            // Fallback gain — only shown when replay gain is not Off
            if replay_gain_mode != ReplayGainMode::Off {
                let fallback_label = format!("{:.1} dB", replay_gain_fallback_db);
                let fallback_control = widget::column()
                    .push(widget::text::caption(fallback_label))
                    .push(
                        widget::slider(
                            -12.0..=0.0_f32,
                            replay_gain_fallback_db,
                            SettingsMessage::SetReplayGainFallback,
                        )
                        .step(0.5_f32),
                    )
                    .spacing(spacing::XXXS);

                playback_section = playback_section.add(cosmic::widget::settings::flex_item(
                    "Fallback Gain",
                    fallback_control,
                ));
            }
        }

        col = col.push(playback_section);
    }

    col.into()
}
