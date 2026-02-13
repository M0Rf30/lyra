// SPDX-License-Identifier: GPL-3.0

fn main() -> cosmic::iced::Result {
    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    #[cfg(not(feature = "tokio-console"))]
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    cosmic_music_player::i18n::init(&requested_languages);

    // Settings for configuring the application window and iced runtime.
    let settings = cosmic::app::Settings::default().size_limits(
        cosmic::iced::Limits::NONE
            .min_width(900.0)
            .min_height(600.0),
    );

    // Starts the application's event loop.
    cosmic::app::run::<cosmic_music_player::app::AppModel>(settings, ())
}
