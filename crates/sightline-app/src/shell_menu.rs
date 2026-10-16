//! "Preview with Sightline" in the Explorer context menu for supported files.
//! Written under the current user's registry hive only, so no administrator rights are needed.

use std::io;
use std::path::Path;

use crate::registry;

/// File types the menu is added to; the readers in `sightline-core` open these.
pub use sightline_core::EXTENSIONS;

/// Adds the menu entry for every supported extension, labelled with `verb`.
pub fn register(exe: &Path, verb: &str) -> io::Result<()> {
    let exe_text = exe.display().to_string();
    for extension in EXTENSIONS {
        let menu = menu_key(extension);
        registry::set_value(&menu, "MUIVerb", verb)?;
        registry::set_value(&menu, "Icon", &exe_text)?;
        registry::set_value(&command_key(&menu), "", &command_line(exe))?;
    }
    Ok(())
}

/// Removes only the keys `register` creates; a missing key is not an error.
pub fn unregister() -> io::Result<()> {
    for extension in EXTENSIONS {
        let menu = menu_key(extension);
        registry::delete_key(&command_key(&menu))?;
        registry::delete_key(&menu)?;
    }
    Ok(())
}

fn command_key(menu_key: &str) -> String {
    format!(r"{menu_key}\command")
}

/// Registry key of the menu entry for one extension, relative to `HKEY_CURRENT_USER`.
fn menu_key(extension: &str) -> String {
    format!(r"Software\Classes\SystemFileAssociations\.{extension}\shell\Sightline")
}

/// Command Explorer runs; `%1` is the selected file.
fn command_line(exe: &Path) -> String {
    format!(r#""{}" "%1""#, exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_key_is_per_extension_under_the_user_classes() {
        assert_eq!(
            menu_key("csv"),
            r"Software\Classes\SystemFileAssociations\.csv\shell\Sightline"
        );
    }

    #[test]
    fn menu_is_added_for_every_readable_extension() {
        let keys: Vec<String> = EXTENSIONS.iter().map(|ext| menu_key(ext)).collect();

        assert_eq!(
            keys,
            [
                r"Software\Classes\SystemFileAssociations\.csv\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.parquet\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.safetensors\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.onnx\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.npy\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.pkl\shell\Sightline",
                r"Software\Classes\SystemFileAssociations\.pickle\shell\Sightline",
            ]
        );
    }

    #[test]
    fn command_quotes_the_program_and_the_file() {
        let exe = Path::new(r"C:\Program Files\Sightline\sightline-app.exe");

        assert_eq!(
            command_line(exe),
            r#""C:\Program Files\Sightline\sightline-app.exe" "%1""#
        );
    }
}
