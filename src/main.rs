// SPDX-License-Identifier: GPL-3.0

#![allow(dead_code, unused_imports)]

mod app;
mod config;
mod i18n;
mod library;
mod player;
mod provider;
mod views;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Settings for configuring the application window and iced runtime.
    let settings = cosmic::app::Settings::default().size_limits(
        cosmic::iced::Limits::NONE
            .min_width(900.0)
            .min_height(600.0),
    );

    // Starts the application's event loop.
    cosmic::app::run::<app::AppModel>(settings, ())
}
