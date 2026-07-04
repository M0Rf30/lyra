// SPDX-License-Identifier: GPL-3.0

//! Equalizer view with 10-band vertical sliders and preset management.
//!
//! The preset dropdown contains built-in and custom presets. AutoEQ headphone
//! profiles are searched and selected via a separate text input + scrollable
//! results list below the preset controls.

use crate::autoeq::AutoEQProfileMetadata;
use crate::player::equalizer::{BAND_LABELS, EqPresetData, PresetSource};
use crate::views::list_row_button_class;
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

    let toolbar_row = widget::Row::new()
        .push(save_btn)
        .push(delete_btn)
        .push(reset_btn)
        .spacing(4);

    // --- Save As inline input ---
    let save_as_input = widget::text_input("Preset name...", save_as_name)
        .on_input(EqualizerMessage::SaveAsNameChanged);

    let save_as_btn = if !save_as_name.trim().is_empty() {
        widget::button::standard("Save As").on_press(EqualizerMessage::SavePresetAs)
    } else {
        widget::button::standard("Save As")
    };

    let save_as_row = widget::Row::new()
        .push(save_as_input)
        .push(save_as_btn)
        .spacing(4)
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
                .take(50) // cap results for performance
                .collect();

            let count = filtered.len();
            let count_text: cosmic::Element<'_, EqualizerMessage> = if count == 0 {
                widget::text::caption("No matches").into()
            } else if count >= 50 {
                widget::text::caption("50+ matches — refine your search").into()
            } else {
                widget::text::caption(format!("{count} matches")).into()
            };

            // Build scrollable clickable list of matching profiles
            let mut result_list = widget::Column::new().spacing(1);
            for profile in &filtered {
                let path = profile.path.clone();
                let subtitle = format!("{} · {}", profile.type_, profile.source);

                let row_content = widget::Column::new()
                    .push(widget::text::body(&profile.name))
                    .push(widget::text::caption(subtitle))
                    .spacing(1);

                let row = widget::button::custom(widget::container(row_content).padding([4, 8]))
                    .on_press(EqualizerMessage::SelectAutoEQ(path))
                    .width(Length::Fill)
                    .class(list_row_button_class(false));

                result_list = result_list.push(row);
            }

            let scrollable_results =
                widget::scrollable(widget::container(result_list).width(Length::Fill))
                    .height(Length::Fixed(200.0));

            let refresh_btn = widget::button::text(format!("{} profiles", autoeq_profiles.len()))
                .on_press(EqualizerMessage::FetchAutoEQ);

            widget::Column::new()
                .push(search_input)
                .push(count_text)
                .push(scrollable_results)
                .push(refresh_btn)
                .spacing(4)
                .into()
        } else {
            let hint: cosmic::Element<'_, EqualizerMessage> =
                widget::text::caption("Type 2+ chars to search").into();

            let refresh_btn =
                widget::button::text(format!("{} profiles loaded", autoeq_profiles.len()))
                    .on_press(EqualizerMessage::FetchAutoEQ);

            widget::Column::new()
                .push(search_input)
                .push(hint)
                .push(refresh_btn)
                .spacing(4)
                .into()
        }
    } else {
        // Profiles not yet loaded — show fetch button
        let fetch_btn = if autoeq_loading {
            widget::button::text("Loading...")
        } else {
            widget::button::text("Load AutoEQ Profiles").on_press(EqualizerMessage::FetchAutoEQ)
        };
        widget::Column::new().push(fetch_btn).spacing(4).into()
    };

    // --- Preamp slider ---
    let preamp_row = widget::Row::new()
        .push(widget::text::body("Preamp:"))
        .push(widget::space::horizontal())
        .push(widget::text::body(format!("{:+.1} dB", preamp)))
        .spacing(8)
        .align_y(Alignment::Center);

    let preamp_slider =
        widget::slider(-20.0..=10.0, preamp, EqualizerMessage::SetPreamp).width(Length::Fill);

    let preamp_control = widget::Column::new()
        .push(preamp_row)
        .push(preamp_slider)
        .spacing(4);

    // --- 10-band vertical sliders ---
    // Each band column gets equal width via Length::Fill so they spread
    // evenly across the full panel width.
    let mut band_row = widget::Row::new().spacing(2).width(Length::Fill);

    for (i, &gain) in bands.iter().enumerate().take(10) {
        let label = if i < BAND_LABELS.len() {
            BAND_LABELS[i]
        } else {
            "?"
        };

        let slider_col = widget::Column::new()
            .push(widget::text::caption(format!("{:+.1}", gain)).size(10))
            .push(
                widget::vertical_slider(-12.0..=12.0, gain, move |v| {
                    EqualizerMessage::SetBand(i, v)
                })
                .height(150.0),
            )
            .push(widget::text::caption(label).size(10))
            .spacing(2)
            .width(Length::Fill)
            .align_x(Alignment::Center);

        band_row = band_row.push(slider_col);
    }

    // --- Assemble layout ---
    widget::Column::new()
        .push(toggle)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("Preset"))
        .push(preset_dropdown)
        .push(toolbar_row)
        .push(save_as_row)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("AutoEQ"))
        .push(autoeq_section)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("Preamp"))
        .push(preamp_control)
        .push(widget::divider::horizontal::default())
        .push(widget::text::title4("Bands"))
        .push(band_row)
        .spacing(8)
        .padding(16)
        .into()
}
