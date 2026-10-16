//! Saves the notebook for "Open in notebook" under Documents\Sightline and opens it with
//! the program Windows associates with `.ipynb`. An existing file is never overwritten;
//! the data file itself is never touched.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::known_folder;

const FOLDER_NAME: &str = "Sightline";
const EXTENSION: &str = "ipynb";
/// Enough numbered names for any real use; past it, something is wrong with the folder.
const MAX_ATTEMPTS: u32 = 1_000;

/// Writes `text` as a new notebook named after `source` in `folder`, created if missing:
/// `sales.ipynb`, then `sales (2).ipynb` and so on when the name is taken.
pub fn save_new(folder: &Path, source: &Path, text: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let stem = source
        .file_stem()
        .map_or_else(|| "notebook".into(), |stem| stem.to_string_lossy());
    for attempt in 1..=MAX_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stem}.{EXTENSION}")
        } else {
            format!("{stem} ({attempt}).{EXTENSION}")
        };
        let path = folder.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{MAX_ATTEMPTS} notebooks named after {stem} already exist"),
    ))
}

/// `Documents\Sightline` of the current user, wherever Documents has been moved to.
pub fn notebook_folder() -> io::Result<PathBuf> {
    known_folder::path(&windows::Win32::UI::Shell::FOLDERID_Documents)
        .map(|documents| documents.join(FOLDER_NAME))
}

/// Opens `path` with its associated program; without one, shows its folder instead.
pub fn open(path: &Path) -> io::Result<()> {
    match shell_open(path) {
        Err(error) if error.raw_os_error() == Some(NO_ASSOCIATION) => {
            shell_open(path.parent().unwrap_or(path))
        }
        result => result,
    }
}

/// `SE_ERR_NOASSOC`: no program is registered for the file type.
const NO_ASSOCIATION: i32 = 31;
/// `ShellExecuteW` returns a value above this on success.
const SHELL_EXECUTE_OK_ABOVE: isize = 32;

fn shell_open(path: &Path) -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb = wide("open");
    let file = wide(&path.display().to_string());
    // SAFETY: both strings are NUL-terminated and outlive the call; no window handle.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > SHELL_EXECUTE_OK_ABOVE {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    /// A new folder under the workspace's `target\tmp`, so runs never meet each other's files.
    fn new_folder(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after 1970")
            .as_nanos();
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/notebook-tests")
            .join(format!("{name}-{nanos}"))
    }

    #[test]
    fn the_notebook_is_named_after_the_data_file() {
        let folder = new_folder("named");

        let path = save_new(&folder, Path::new(r"C:\data\sales.csv"), "{}").expect("saved");

        assert_eq!(path, folder.join("sales.ipynb"));
        assert_eq!(fs::read_to_string(&path).expect("read"), "{}");
    }

    #[test]
    fn an_existing_notebook_is_never_overwritten() {
        let folder = new_folder("existing");
        let source = Path::new(r"C:\data\sales.csv");
        let first = save_new(&folder, source, "first").expect("first");

        let second = save_new(&folder, source, "second").expect("second");

        assert_eq!(second, folder.join("sales (2).ipynb"));
        assert_eq!(fs::read_to_string(first).expect("read"), "first");
        assert_eq!(fs::read_to_string(second).expect("read"), "second");
    }

    #[test]
    fn turkish_names_are_kept() {
        let folder = new_folder("turkish");

        let path =
            save_new(&folder, Path::new(r"C:\veri\şehir_nüfus.parquet"), "{}").expect("saved");

        assert_eq!(path.file_name().expect("name"), "şehir_nüfus.ipynb");
    }
}
