//! The window language the user picked on the About screen, kept under the current user's
//! registry hive. The `--lang` flag still wins over it.

use std::io;
use std::ptr;

use slint::SelectBundledTranslationError;
use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

use crate::{LanguageChoice, registry};

/// Settings key of Sightline, relative to `HKEY_CURRENT_USER`; uninstalling removes it.
pub const SETTINGS_KEY: &str = r"Software\Sightline";
const VALUE_NAME: &str = "Language";
const TURKISH: &str = "tr";
/// The language the interface is written in; Slint needs no translation for it.
const ENGLISH: &str = "en";

/// The saved choice; the system language when nothing was saved.
pub fn saved() -> LanguageChoice {
    choice_from_stored(registry::get_value(SETTINGS_KEY, VALUE_NAME).as_deref())
}

pub fn save(choice: LanguageChoice) -> io::Result<()> {
    match stored_value(choice) {
        Some(code) => registry::set_value(SETTINGS_KEY, VALUE_NAME, code),
        None => registry::delete_value(SETTINGS_KEY, VALUE_NAME),
    }
}

/// Switches the language of every window of this process.
pub fn select(choice: LanguageChoice) -> Result<(), SelectBundledTranslationError> {
    slint::select_bundled_translation(translation(choice, &windows_language()))
}

fn stored_value(choice: LanguageChoice) -> Option<&'static str> {
    match choice {
        LanguageChoice::Turkish => Some(TURKISH),
        LanguageChoice::English => Some(ENGLISH),
        LanguageChoice::System => None,
    }
}

fn choice_from_stored(value: Option<&str>) -> LanguageChoice {
    match value {
        Some(TURKISH) => LanguageChoice::Turkish,
        Some(ENGLISH) => LanguageChoice::English,
        _ => LanguageChoice::System,
    }
}

/// The bundled translation to use; Windows languages other than Turkish get English.
fn translation(choice: LanguageChoice, windows_language: &str) -> &'static str {
    match choice {
        LanguageChoice::Turkish => TURKISH,
        LanguageChoice::English => ENGLISH,
        LanguageChoice::System => {
            let base = windows_language
                .split(['-', '_'])
                .next()
                .unwrap_or_default();
            if base.eq_ignore_ascii_case(TURKISH) {
                TURKISH
            } else {
                ENGLISH
            }
        }
    }
}

/// The user's first display language, such as "tr-TR"; the same source Slint reads at
/// start, so "System language" gives what the window showed before any choice.
fn windows_language() -> String {
    let mut count = 0;
    let mut size = 0;
    // SAFETY: the first call only reports the buffer size the second call fills.
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, ptr::null_mut(), &mut size)
            == 0
        {
            return String::new();
        }
        let mut buffer = vec![0u16; size as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            buffer.as_mut_ptr(),
            &mut size,
        ) == 0
        {
            return String::new();
        }
        let first = buffer.split(|&unit| unit == 0).next().unwrap_or_default();
        String::from_utf16_lossy(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_stores_nothing() {
        assert_eq!(stored_value(LanguageChoice::System), None);
    }

    #[test]
    fn languages_store_their_code() {
        assert_eq!(stored_value(LanguageChoice::Turkish), Some("tr"));
        assert_eq!(stored_value(LanguageChoice::English), Some("en"));
    }

    #[test]
    fn stored_codes_read_back_as_the_same_choice() {
        for choice in [
            LanguageChoice::System,
            LanguageChoice::Turkish,
            LanguageChoice::English,
        ] {
            assert_eq!(choice_from_stored(stored_value(choice)), choice);
        }
    }

    #[test]
    fn missing_or_unknown_value_means_the_system_language() {
        assert_eq!(choice_from_stored(None), LanguageChoice::System);
        assert_eq!(choice_from_stored(Some("de")), LanguageChoice::System);
    }

    #[test]
    fn system_choice_follows_the_windows_language() {
        assert_eq!(translation(LanguageChoice::System, "tr-TR"), "tr");
        assert_eq!(translation(LanguageChoice::System, "en-US"), "en");
        assert_eq!(translation(LanguageChoice::System, "de-DE"), "en");
        assert_eq!(translation(LanguageChoice::System, ""), "en");
    }

    #[test]
    fn picked_language_ignores_the_windows_language() {
        assert_eq!(translation(LanguageChoice::English, "tr-TR"), "en");
        assert_eq!(translation(LanguageChoice::Turkish, "en-US"), "tr");
    }
}
