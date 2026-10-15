//! "Preview with Sightline" in the Explorer context menu for supported files.
//! Written under the current user's registry hive only, so no administrator rights are needed.

use std::io;
use std::path::Path;

/// File types the menu is added to; the readers in `sightline-core` open these.
const EXTENSIONS: [&str; 2] = ["csv", "parquet"];

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

/// Thin wrapper over the Win32 registry calls, limited to `HKEY_CURRENT_USER`.
mod registry {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyW, RegSetKeyValueW,
    };

    /// Sets one text value, creating `key` if needed; `""` is the key's default value.
    pub fn set_value(key: &str, name: &str, value: &str) -> io::Result<()> {
        let data = wide(value);
        let byte_len = u32::try_from(data.len() * size_of::<u16>())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "registry value too long"))?;
        // SAFETY: every pointer is a NUL-terminated UTF-16 buffer that outlives the call,
        // and `data` is `byte_len` bytes long.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(name).as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                byte_len,
            )
        };
        check(status)
    }

    /// Deletes `key` if it has no subkeys left; a key that does not exist is fine.
    pub fn delete_key(key: &str) -> io::Result<()> {
        // SAFETY: the path is a NUL-terminated UTF-16 buffer that outlives the call.
        let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(status)
    }

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain([0]).collect()
    }

    fn check(status: WIN32_ERROR) -> io::Result<()> {
        if status == ERROR_SUCCESS {
            return Ok(());
        }
        Err(io::Error::from_raw_os_error(status as i32))
    }
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
