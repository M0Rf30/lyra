// SPDX-License-Identifier: GPL-3.0

//! Equalizer view with 10-band vertical sliders and preset management.
//!
//! The preset dropdown contains built-in and custom presets. AutoEQ headphone
//! profiles are searched and selected via a separate text input + scrollable
//! results list below the preset controls.

use crate::autoeq::AutoEQProfileMetadata;
use crate::player::equalizer::{BAND_LABELS, EqPresetData, PresetSource};
use crate::views::spacing;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget;

/// Messages from the equalizer view.
#[derive(Debug, Clone)]
pub enum EqualizerMessage {
    SetBand(usize, f32),
    ToggleEnabled(bool),
    SetPreamp(f32),
    /// Select a regular preset by name.
    SelectPreset(String),
    /// Select an AutoEQ profile by its repository path.
    SelectAutoEQ(String),
    /// Save (overwrite) the current custom preset.
    SavePreset,
    /// User typed a name in the "Save As" input.
    SaveAsNameChanged(String),
    /// Confirm "Save As" with the typed name.
    SavePresetAs,
    /// Delete the active custom preset.
    DeletePreset,
    /// Reset to Flat (all bands 0, preamp 0).
    ResetPreset,
    /// Fetch AutoEQ profile index from GitHub.
    FetchAutoEQ,
    /// User typed in the AutoEQ search field.
    AutoEQSearchChanged(String),
}

/// Render the equalizer panel (shown in the context drawer).
#[allow(clippy::too_many_arguments)]
pub fn equalizer_view<'a>(
    bands: &'a [f32],
    enabled: bool,
    preamp: f32,
    all_presets: &'a [EqPresetData],
    active_preset_name: Option<&'a str>,
    dirty: bool,
    save_as_name: &'a str,
    autoeq_profiles: &'a [AutoEQProfileMetadata],
    autoeq_loading: bool,
    autoeq_search: &'a str,
) -> cosmic::Element<'a, EqualizerMessage> {
    let toggle = cosmic::widget::settings::item::builder("Equalizer Enabled")
        .control(widget::toggler(enabled).on_toggle(EqualizerMessage::ToggleEnabled));

    // --- Preset dropdown (built-in + custom only) ---
    let preset_names_display: Vec<String> = all_presets
        .iter()
        .map(|p| match p.source {
            PresetSource::Builtin => p.name.clone(),
            _ => format!("{} *", p.name),
        })
        .collect();

    let preset_names: Vec<String> = all_presets.iter().map(|p| p.name.clone()).collect();

    let selected_index =
        active_preset_name.and_then(|active| all_presets.iter().position(|p| p.name == active));

    let preset_dropdown = widget::dropdown(preset_names_display, selected_index, move |idx| {
        if let Some(name) = preset_names.get(idx) {
            EqualizerMessage::SelectPreset(name.clone())
        } else {
            EqualizerMessage::ResetPreset
        }
    })
    .width(Length::Fill);

    // --- Toolbar buttons ---
    let is_custom_selected = active_preset_name
        .map(|name| {
            all_presets
                .iter()
                .any(|p| p.name == name && p.source != PresetSource::Builtin)
        })
        .unwrap_or(false);

    let save_btn = widget::button::text("Save").on_press_maybe(if is_custom_selected && dirty {
        Some(EqualizerMessage::SavePreset)
    } else {
        None
    });

    let delete_btn = widget::button::destructive("Delete").on_press_maybe(if is_custom_selected {
        Some(EqualizerMessage::DeletePreset)
    } else {
        None
    });

    let reset_btn = widget::button::text("Reset").on_press(EqualizerMessage::ResetPreset);

    let toolbar_row = widget::row()
        .push(save_btn)
        .push(delete_btn)
        .push(reset_btn)
        .spacing(spacing::XXXS);

    // --- Save As inline input ---
    let save_as_input = widget::text_input("Preset name...", save_as_name)
        .on_input(EqualizerMessage::SaveAsNameChanged);

    let save_as_btn = if !save_as_name.trim().is_empty() {
        widget::button::standard("Save As").on_press(EqualizerMessage::SavePresetAs)
    } else {
        widget::button::standard("Save As")
    };

    let save_as_row = widget::row()
        .push(save_as_input)
        .push(save_as_btn)
        .spacing(spacing::XXXS)
        .align_y(Alignment::Center);

    // --- AutoEQ section: search input + scrollable results list ---
    let autoeq_section: cosmic::Element<'a, EqualizerMessage> = if !autoeq_profiles.is_empty() {
        let query = autoeq_search.trim().to_lowercase();

        let search_input = widget::text_input("Search headphones...", autoeq_search)
            .on_input(EqualizerMessage::AutoEQSearchChanged);

        if query.len() >= 2 {
            let filtered: Vec<&AutoEQProfileMetadata> = autoeq_profiles
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&query))
                .take(50)
                .collect();

            let count = filtered.len();
            let count_text: cosmic::Element<'_, EqualizerMessage> = if count == 0 {
                widget::text::caption("No matches").into()
            } else if count >= 50 {
                widget::text::caption("50+ matches \u{2014} refine your search").into()
            } else {
                widget::text::caption(format!("{count} matches")).into()
            };

            let mut result_list = widget::column().spacing(spacing::XXXS);
            for profile in &filtered {
                let path = profile.path.clone();
                let subtitle = format!("{} \u{00b7} {}", profile.type_, profile.source);

                let row_content = widget::column()
                    .push(widget::text::body(&profile.name))
                    .push(widget::text::caption(subtitle))
                    .spacing(spacing::XXXS);

                let row = widget::button::custom(
                    widget::container(row_content).padding([spacing::XXXS, spacing::XXS]),
                )
                .on_press(EqualizerMessage::SelectAutoEQ(path))
                .width(Length::Fill)
                .class(cosmic::theme::Button::Text);

                result_list = result_list.push(row);
            }

            let scrollable_results =
                widget::scrollable(widget::container(result_list).width(Length::Fill))
                    .height(Length::Fixed(200.0));

            let refresh_btn = widget::button::text(format!("{} profiles", autoeq_profiles.len()))
                .on_press(EqualizerMessage::FetchAutoEQ);

            widget::column()
                .push(search_input)
                .push(count_text)
                .push(scrollable_results)
                .push(refresh_btn)
                .spacing(spacing::XXXS)
                .into()
        } else {
            let hint: cosmic::Element<'_, EqualizerMessage> =
                widget::text::caption("Type 2+ chars to search").into();

            let refresh_btn =
                widget::button::text(format!("{} profiles loaded", autoeq_profiles.len()))
                    .on_press(EqualizerMessage::FetchAutoEQ);

            widget::column()
                .push(search_input)
                .push(hint)
                .push(refresh_btn)
                .spacing(spacing::XXXS)
                .into()
        }
    } else {
        // Profiles not yet loaded — show fetch button
        let fetch_btn = if autoeq_loading {
            widget::button::text("Loading...")
        } else {
            widget::button::text("Load AutoEQ Profiles").on_press(EqualizerMessage::FetchAutoEQ)
        };
        widget::column()
            .push(fetch_btn)
            .spacing(spacing::XXXS)
            .into()
    };

    // --- Preamp slider ---
    let preamp_label = widget::text::body(format!("{:+.1} dB", preamp));
    let preamp_slider =
        widget::slider(-20.0..=10.0, preamp, EqualizerMessage::SetPreamp).width(Length::Fill);

    let preamp_control = widget::column()
        .push(preamp_label)
        .push(preamp_slider)
        .spacing(spacing::XXXS);

    let preamp_flex = cosmic::widget::settings::flex_item("Preamp", preamp_control);

    // --- 10-band vertical sliders ---
    let mut band_row = widget::row().spacing(spacing::XXXS).width(Length::Fill);

    for (i, &gain) in bands.iter().enumerate().take(10) {
        let label = if i < BAND_LABELS.len() {
            BAND_LABELS[i]
        } else {
            "?"
        };

        let slider_col = widget::column()
            .push(widget::text::caption(format!("{:+.1}", gain)).size(10))
            .push(
                widget::vertical_slider(-12.0..=12.0, gain, move |v| {
                    EqualizerMessage::SetBand(i, v)
                })
                .height(150.0),
            )
            .push(widget::text::caption(label).size(10))
            .spacing(spacing::XXXS)
            .width(Length::Fill)
            .align_x(Alignment::Center);

        band_row = band_row.push(slider_col);
    }

    // --- Assemble layout ---
    let preset_section = cosmic::widget::settings::section()
        .title("Preset")
        .add(preset_dropdown)
        .add(toolbar_row)
        .add(save_as_row);

    let autoeq_section_widget = cosmic::widget::settings::section()
        .title("AutoEQ")
        .add(autoeq_section);

    let preamp_section = cosmic::widget::settings::section()
        .title("Preamp")
        .add(preamp_flex);

    let bands_section = cosmic::widget::settings::section()
        .title("Bands")
        .add(band_row);

    widget::column()
        .push(toggle)
        .push(preset_section)
        .push(autoeq_section_widget)
        .push(preamp_section)
        .push(bands_section)
        .spacing(spacing::XXS)
        .padding(spacing::S)
        .into()
}
