// SPDX-License-Identifier: GPL-3.0

//! Equalizer view with 10-band sliders and presets.

use crate::player::equalizer::{EqPreset, BAND_LABELS};
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the equalizer view.
#[derive(Debug, Clone)]
pub enum EqualizerMessage {
    SetBand(usize, f32),
    SetPreset(EqPreset),
    ToggleEnabled(bool),
}

/// Render the equalizer panel (shown in the context drawer).
pub fn equalizer_view<'a>(
    bands: &'a [f32],
    enabled: bool,
    _current_preset: Option<EqPreset>,
) -> cosmic::Element<'a, EqualizerMessage> {
    let toggle = cosmic::widget::settings::item::builder("Equalizer Enabled")
        .control(widget::toggler(enabled).on_toggle(EqualizerMessage::ToggleEnabled));

    // Preset buttons in rows
    let mut presets = widget::column().spacing(8);
    let mut preset_row = widget::row().spacing(8);
    for (i, preset) in EqPreset::ALL.iter().enumerate() {
        preset_row = preset_row.push(
            widget::button::standard(preset.label()).on_press(EqualizerMessage::SetPreset(*preset)),
        );
        if (i + 1) % 4 == 0 {
            presets = presets.push(preset_row);
            preset_row = widget::row().spacing(8);
        }
    }
    // Push remaining
    presets = presets.push(preset_row);

    // 10-band sliders (horizontal layout)
    let mut band_columns = widget::row().spacing(12).align_y(Alignment::End);

    for (i, &gain) in bands.iter().enumerate().take(10) {
        let label = if i < BAND_LABELS.len() {
            BAND_LABELS[i]
        } else {
            "?"
        };

        let slider_col = widget::column()
            .push(widget::text::caption(format!("{:+.0}", gain)))
            .push(
                widget::slider(-12.0..=12.0, gain, move |v| EqualizerMessage::SetBand(i, v))
                    .width(120),
            )
            .push(widget::text::caption(label))
            .spacing(4)
            .align_x(Alignment::Center);

        band_columns = band_columns.push(slider_col);
    }

    // dB scale labels
    let db_labels = widget::column()
        .push(widget::text::caption("+12 dB"))
        .push(widget::text::caption("0 dB").height(Length::Shrink))
        .push(widget::text::caption("-12 dB"))
        .spacing(4)
        .height(140);

    let sliders_with_labels = widget::row().push(db_labels).push(band_columns).spacing(8);

    widget::column()
        .push(toggle)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("Presets"))
        .push(presets)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("Bands"))
        .push(sliders_with_labels)
        .spacing(12)
        .padding(16)
        .into()
}
