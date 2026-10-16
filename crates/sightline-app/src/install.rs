//! `--install` and `--uninstall`: a per-user installation without an installer program.
//! Everything goes under the current user (no administrator rights): the program in
//! `%LOCALAPPDATA%\Programs\Sightline`, a Start menu shortcut, the Explorer menu entry and
//! an entry in Windows' list of installed apps. Uninstalling removes only those.

use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, FOLDERID_UserProgramFiles, IShellLinkW, ShellLink,
};
use windows::core::{HSTRING, Interface};

use crate::cli::UNINSTALL_FLAG;
use crate::{autostart, icon, known_folder, language, registry, shell_menu};

const FOLDER_NAME: &str = "Sightline";
const EXE_NAME: &str = "sightline-app.exe";
const ICON_NAME: &str = "sightline.ico";
const SHORTCUT_NAME: &str = "Sightline.lnk";
const PUBLISHER: &str = "Alican Kaya";
/// Windows lists the apps found under this key in Settings > Apps.
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sightline";
/// `CREATE_NO_WINDOW`: the cleanup command runs without a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where an installation puts its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub folder: PathBuf,
    pub exe: PathBuf,
    pub icon: PathBuf,
    pub shortcut: PathBuf,
}

/// The layout under the user's program folder and Start menu programs folder.
pub fn layout(user_programs: &Path, start_menu_programs: &Path) -> Layout {
    let folder = user_programs.join(FOLDER_NAME);
    Layout {
        exe: folder.join(EXE_NAME),
        icon: folder.join(ICON_NAME),
        folder,
        shortcut: start_menu_programs.join(SHORTCUT_NAME),
    }
}

/// Values of the installed-apps entry, as (name, text) pairs.
pub fn uninstall_entries(layout: &Layout, version: &str) -> Vec<(&'static str, String)> {
    let exe = layout.exe.display();
    vec![
        ("DisplayName", FOLDER_NAME.to_string()),
        ("DisplayVersion", version.to_string()),
        ("Publisher", PUBLISHER.to_string()),
        ("InstallLocation", layout.folder.display().to_string()),
        ("DisplayIcon", layout.icon.display().to_string()),
        ("UninstallString", format!(r#""{exe}" {UNINSTALL_FLAG}"#)),
    ]
}

/// A command for `cmd.exe` that removes the installed program after this process exits:
/// the program and icon files by their full paths, then their folder only if it is empty.
pub fn cleanup_command(layout: &Layout) -> String {
    format!(
        r#"timeout /T 2 /NOBREAK >NUL & del /F /Q "{}" "{}" & rmdir "{}""#,
        layout.exe.display(),
        layout.icon.display(),
        layout.folder.display()
    )
}

/// Installs the running program for the current user; `verb` labels the Explorer menu.
pub fn install(verb: &str) -> io::Result<Layout> {
    let layout = user_layout()?;
    fs::create_dir_all(&layout.folder)?;
    let current = std::env::current_exe()?;
    if !same_file(&current, &layout.exe) {
        fs::copy(&current, &layout.exe)?;
    }
    fs::write(&layout.icon, icon::ICO)?;
    shell_menu::register(&layout.exe, verb)?;
    create_shortcut(&layout)?;
    for (name, value) in uninstall_entries(&layout, env!("CARGO_PKG_VERSION")) {
        registry::set_value(UNINSTALL_KEY, name, &value)?;
    }
    // "Start with Windows" keeps working, now from the installed copy.
    if autostart::is_enabled() {
        autostart::set_enabled(&layout.exe, true)?;
    }
    Ok(layout)
}

/// Removes what `install` added. The running program cannot delete itself, so a short
/// command deletes it a moment after this process exits.
pub fn uninstall() -> io::Result<()> {
    let layout = user_layout()?;
    shell_menu::unregister()?;
    autostart::set_enabled(&layout.exe, false)?;
    remove_if_present(&layout.shortcut)?;
    registry::delete_key(UNINSTALL_KEY)?;
    registry::delete_key(language::SETTINGS_KEY)?;
    if same_file(&std::env::current_exe()?, &layout.exe) {
        Command::new("cmd.exe")
            .arg("/C")
            .raw_arg(cleanup_command(&layout))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    } else {
        remove_if_present(&layout.exe)?;
        remove_if_present(&layout.icon)?;
        // Left in place when something else is in the folder.
        let _ = fs::remove_dir(&layout.folder);
    }
    Ok(())
}

fn user_layout() -> io::Result<Layout> {
    Ok(layout(
        &known_folder::path(&FOLDERID_UserProgramFiles)?,
        &known_folder::path(&FOLDERID_Programs)?,
    ))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// A Start menu shortcut to the installed program.
fn create_shortcut(layout: &Layout) -> io::Result<()> {
    // SAFETY: COM is initialized for this thread around the calls and released after;
    // the interfaces are dropped before CoUninitialize.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(to_io)?;
        let result = save_shortcut(layout);
        CoUninitialize();
        result.map_err(to_io)
    }
}

/// # Safety
/// COM must be initialized on the calling thread.
unsafe fn save_shortcut(layout: &Layout) -> windows::core::Result<()> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(layout.exe.as_path()))?;
        link.SetWorkingDirectory(&HSTRING::from(layout.folder.as_path()))?;
        link.SetIconLocation(&HSTRING::from(layout.icon.as_path()), 0)?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(layout.shortcut.as_path()), true)
    }
}

fn to_io(error: windows::core::Error) -> io::Error {
    io::Error::other(error.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Layout {
        layout(
            Path::new(r"C:\Users\ali\AppData\Local\Programs"),
            Path::new(r"C:\Users\ali\AppData\Roaming\Microsoft\Windows\Start Menu\Programs"),
        )
    }

    #[test]
    fn the_program_goes_into_its_own_folder_under_the_user_programs() {
        assert_eq!(
            sample(),
            Layout {
                folder: PathBuf::from(r"C:\Users\ali\AppData\Local\Programs\Sightline"),
                exe: PathBuf::from(
                    r"C:\Users\ali\AppData\Local\Programs\Sightline\sightline-app.exe"
                ),
                icon: PathBuf::from(r"C:\Users\ali\AppData\Local\Programs\Sightline\sightline.ico"),
                shortcut: PathBuf::from(
                    r"C:\Users\ali\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Sightline.lnk"
                ),
            }
        );
    }

    #[test]
    fn the_installed_apps_entry_names_the_program_and_how_to_remove_it() {
        let entries = uninstall_entries(&sample(), "1.0.0");
        let value = |name: &str| {
            entries
                .iter()
                .find(|(entry, _)| *entry == name)
                .map(|(_, value)| value.as_str())
        };

        assert_eq!(value("DisplayName"), Some("Sightline"));
        assert_eq!(value("DisplayVersion"), Some("1.0.0"));
        assert_eq!(value("Publisher"), Some("Alican Kaya"));
        assert_eq!(
            value("InstallLocation"),
            Some(r"C:\Users\ali\AppData\Local\Programs\Sightline")
        );
        assert_eq!(
            value("DisplayIcon"),
            Some(r"C:\Users\ali\AppData\Local\Programs\Sightline\sightline.ico")
        );
        assert_eq!(
            value("UninstallString"),
            Some(
                r#""C:\Users\ali\AppData\Local\Programs\Sightline\sightline-app.exe" --uninstall"#
            )
        );
    }

    #[test]
    fn cleanup_deletes_only_the_program_file_and_an_empty_folder() {
        let command = cleanup_command(&sample());

        assert!(
            command.contains(
                r#"del /F /Q "C:\Users\ali\AppData\Local\Programs\Sightline\sightline-app.exe" "C:\Users\ali\AppData\Local\Programs\Sightline\sightline.ico""#
            ),
            "{command}"
        );
        assert!(
            command.ends_with(r#"rmdir "C:\Users\ali\AppData\Local\Programs\Sightline""#),
            "{command}"
        );
        assert!(!command.to_ascii_uppercase().contains("/S"), "{command}");
    }
}
