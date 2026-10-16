//! Settings picked on the Settings screen, kept next to the language under the current
//! user's registry key: the theme and how many rows the summaries sample.

use std::io;

use crate::language::SETTINGS_KEY;
use crate::{SampleSize, ThemeChoice, registry};

const THEME_VALUE: &str = "Theme";
const SAMPLE_VALUE: &str = "SampleRows";
/// Where Windows keeps whether apps use the light theme.
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const LIGHT_APPS_VALUE: &str = "AppsUseLightTheme";

pub fn theme() -> ThemeChoice {
    theme_from_stored(registry::get_value(SETTINGS_KEY, THEME_VALUE).as_deref())
}

pub fn save_theme(choice: ThemeChoice) -> io::Result<()> {
    match theme_stored(choice) {
        Some(value) => registry::set_value(SETTINGS_KEY, THEME_VALUE, value),
        None => registry::delete_value(SETTINGS_KEY, THEME_VALUE),
    }
}

pub fn sample_size() -> SampleSize {
    sample_from_stored(registry::get_value(SETTINGS_KEY, SAMPLE_VALUE).as_deref())
}

pub fn save_sample_size(size: SampleSize) -> io::Result<()> {
    match sample_stored(size) {
        Some(value) => registry::set_value(SETTINGS_KEY, SAMPLE_VALUE, value),
        None => registry::delete_value(SETTINGS_KEY, SAMPLE_VALUE),
    }
}

/// Whether Windows uses its dark theme for apps; light when it does not say.
pub fn windows_is_dark() -> bool {
    registry::get_dword(PERSONALIZE_KEY, LIGHT_APPS_VALUE) == Some(0)
}

/// Rows the summaries sample at most.
pub fn sample_rows(size: SampleSize) -> usize {
    match size {
        SampleSize::Small => 10_000,
        SampleSize::Medium => 100_000,
        SampleSize::Large => 1_000_000,
    }
}

fn theme_stored(choice: ThemeChoice) -> Option<&'static str> {
    match choice {
        ThemeChoice::Light => Some("light"),
        ThemeChoice::Dark => Some("dark"),
        ThemeChoice::System => None,
    }
}

fn theme_from_stored(value: Option<&str>) -> ThemeChoice {
    match value {
        Some("light") => ThemeChoice::Light,
        Some("dark") => ThemeChoice::Dark,
        _ => ThemeChoice::System,
    }
}

/// The default size is not stored, so a missing value keeps today's behaviour.
fn sample_stored(size: SampleSize) -> Option<&'static str> {
    match size {
        SampleSize::Small => None,
        SampleSize::Medium => Some("100000"),
        SampleSize::Large => Some("1000000"),
    }
}

fn sample_from_stored(value: Option<&str>) -> SampleSize {
    match value {
        Some("100000") => SampleSize::Medium,
        Some("1000000") => SampleSize::Large,
        _ => SampleSize::Small,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_theme_and_default_sample_store_nothing() {
        assert_eq!(theme_stored(ThemeChoice::System), None);
        assert_eq!(sample_stored(SampleSize::Small), None);
    }

    #[test]
    fn stored_values_read_back_as_the_same_choice() {
        for choice in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(theme_from_stored(theme_stored(choice)), choice);
        }
        for size in [SampleSize::Small, SampleSize::Medium, SampleSize::Large] {
            assert_eq!(sample_from_stored(sample_stored(size)), size);
        }
    }

    #[test]
    fn unknown_values_fall_back_to_the_defaults() {
        assert_eq!(theme_from_stored(Some("blue")), ThemeChoice::System);
        assert_eq!(sample_from_stored(Some("12")), SampleSize::Small);
    }

    #[test]
    fn sample_sizes_grow_tenfold() {
        assert_eq!(
            [SampleSize::Small, SampleSize::Medium, SampleSize::Large].map(sample_rows),
            [10_000, 100_000, 1_000_000]
        );
    }
}
