//! The computer the game is running on, the equivalent of `love.system`.

use crate::context;
pub use smitten_core::{PowerInfo, PowerState};

/// The operating system's name, using LÖVE's spellings: `"Windows"`,
/// `"Linux"`, `"OS X"`, `"Android"` or `"iOS"`. Anything else is named as
/// Rust names it.
pub fn os() -> &'static str {
    os_name(std::env::consts::OS)
}

fn os_name(rust_name: &'static str) -> &'static str {
    match rust_name {
        "windows" => "Windows",
        "linux" => "Linux",
        "macos" => "OS X",
        "android" => "Android",
        "ios" => "iOS",
        other => other,
    }
}

/// How many threads the processor can run at once. At least 1.
pub fn processor_count() -> usize {
    std::thread::available_parallelism().map_or(1, |count| count.get())
}

/// Puts `text` on the system clipboard, for pasting elsewhere.
pub fn set_clipboard_text(text: &str) {
    context::with(|ctx| {
        if let Err(message) = ctx.backend.set_clipboard_text(text) {
            eprintln!("smitten: could not set the clipboard: {message}");
        }
    });
}

/// The text on the system clipboard. Empty if there is none.
pub fn clipboard_text() -> String {
    context::with(|ctx| ctx.backend.clipboard_text())
}

/// Opens a web address in the player's browser, or a file with the
/// program the system uses for it. Returns whether it could be opened.
pub fn open_url(url: &str) -> bool {
    context::with(|ctx| ctx.backend.open_url(url).is_ok())
}

/// The state of the battery, if the computer has one.
pub fn power_info() -> PowerInfo {
    context::with(|ctx| ctx.backend.power_info())
}

/// The player's preferred languages, most preferred first, the
/// equivalent of `love.system.getPreferredLocales`. Each is a language
/// code with an optional country code, such as `"en_US"`, `"pt_BR"` or
/// `"fr"`. Empty if the system does not say.
///
/// Pick the first one the game has a translation for, falling back to
/// matching the language alone:
///
/// ```no_run
/// use smitten::system;
///
/// let translations = ["en", "fr", "de"];
/// let language = system::preferred_locales()
///     .iter()
///     .map(|locale| locale.split('_').next().unwrap_or(locale).to_string())
///     .find(|language| translations.contains(&language.as_str()))
///     .unwrap_or_else(|| "en".to_string());
/// ```
pub fn preferred_locales() -> Vec<String> {
    context::with(|ctx| ctx.backend.preferred_locales())
}

/// Vibrates the phone or tablet the game runs on for `seconds`, the
/// equivalent of `love.system.vibrate`. Returns whether it could.
///
/// This works on Android, with the `VIBRATE` permission in the app's
/// manifest. Elsewhere it does nothing and returns `false`; to shake a
/// gamepad, use [`crate::Joystick::set_vibration`].
pub fn vibrate(seconds: f32) -> bool {
    context::with(|ctx| ctx.backend.vibrate(seconds))
}

/// Whether the player is listening to music from another app, the
/// equivalent of `love.system.hasBackgroundMusic`. A game can check this
/// to leave its own music off.
///
/// LÖVE can only tell on iOS and Android. Smitten cannot tell anywhere
/// yet, so this is always `false`.
pub fn has_background_music() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operating_systems_are_named_as_in_love() {
        assert_eq!(os_name("windows"), "Windows");
        assert_eq!(os_name("linux"), "Linux");
        assert_eq!(os_name("macos"), "OS X");
        assert_eq!(os_name("freebsd"), "freebsd");
        assert!(!os().is_empty());
    }

    #[test]
    fn there_is_always_at_least_one_processor() {
        assert!(processor_count() >= 1);
    }
}
