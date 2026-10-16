//! Plain text to the Windows clipboard, as UTF-16 so Turkish letters survive the paste.

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

/// `CF_UNICODETEXT` from WinUser.h: NUL-terminated UTF-16 text.
const UNICODE_TEXT: u32 = 13;

pub fn set_text(text: &str) -> io::Result<()> {
    // CR LF is the Windows line end; some programs show text with bare LF on one line.
    let wide: Vec<u16> = text
        .replace('\n', "\r\n")
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: the clipboard is opened and always closed here; the memory handed to it is
    // filled before the hand-over, and freed here only when the hand-over fails.
    unsafe {
        if OpenClipboard(ptr::null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = put(&wide);
        CloseClipboard();
        result
    }
}

/// Must run while the clipboard is open.
unsafe fn put(wide: &[u16]) -> io::Result<()> {
    // SAFETY: the caller has the clipboard open; `memory` is written within its size.
    unsafe {
        if EmptyClipboard() == 0 {
            return Err(io::Error::last_os_error());
        }
        let memory = GlobalAlloc(GMEM_MOVEABLE, size_of_val(wide));
        if memory.is_null() {
            return Err(io::Error::last_os_error());
        }
        let target = GlobalLock(memory).cast::<u16>();
        if target.is_null() {
            GlobalFree(memory);
            return Err(io::Error::last_os_error());
        }
        ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(memory);
        // On success the clipboard owns the memory.
        if SetClipboardData(UNICODE_TEXT, memory).is_null() {
            GlobalFree(memory);
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
