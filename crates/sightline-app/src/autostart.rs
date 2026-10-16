//! "Start with Windows" for the tray mode, through the current user's Run key.
//! Off unless the user turns it on; no administrator rights are needed.

use std::io;
use std::path::Path;

use crate::cli::TRAY_FLAG;
use crate::registry;

/// Programs listed here start when the current user signs in.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Sightline";

pub fn is_enabled() -> bool {
    registry::get_value(RUN_KEY, VALUE_NAME).is_some()
}

pub fn set_enabled(exe: &Path, enabled: bool) -> io::Result<()> {
    if enabled {
        registry::set_value(RUN_KEY, VALUE_NAME, &command_line(exe))
    } else {
        registry::delete_value(RUN_KEY, VALUE_NAME)
    }
}

/// Command Windows runs at sign-in: the program in tray mode.
fn command_line(exe: &Path) -> String {
    format!(r#""{}" {TRAY_FLAG}"#, exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_starts_the_quoted_program_in_tray_mode() {
        let exe = Path::new(r"C:\Program Files\Sightline\sightline-app.exe");

        assert_eq!(
            command_line(exe),
            r#""C:\Program Files\Sightline\sightline-app.exe" --tray"#
        );
    }
}
