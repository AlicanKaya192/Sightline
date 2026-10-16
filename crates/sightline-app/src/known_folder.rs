//! Paths of the current user's Windows folders (Documents, Start menu…), wherever they live.

use std::io;
use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::GUID;

/// The folder with the known-folder id `id`, such as `FOLDERID_Documents`.
pub fn path(id: &GUID) -> io::Result<PathBuf> {
    // SAFETY: the returned string is read once and then freed with the COM allocator,
    // as SHGetKnownFolderPath requires.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)
            .map_err(|error| io::Error::other(error.message()))?;
        let text = path.to_string();
        CoTaskMemFree(Some(path.as_ptr() as *const _));
        text.map(PathBuf::from)
            .map_err(|_| io::Error::other("a known folder path is not valid text"))
    }
}
