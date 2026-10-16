//! The files selected in the foreground Explorer window, read through the shell's COM interfaces.
//! Windows 11 keeps several tabs in one Explorer window; only the active tab counts.
//! The calling thread must have joined a single-threaded COM apartment (`CoInitializeEx`).

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    IFolderView2, IShellBrowser, IShellWindows, SID_STopLevelBrowser, SIGDN_FILESYSPATH,
    ShellWindows,
};
use windows::core::{Interface, Result};
use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW;

/// One selected item is the only case that opens; two are enough to tell it is not one.
const MAX_ITEMS: u32 = 2;

/// Window class of one Explorer tab; the active tab comes first among its siblings.
const TAB_CLASS: &str = "ShellTabWindowClass";

/// Paths of up to `MAX_ITEMS` items selected in the active tab of `explorer_window`.
/// Empty when the window is not a file browser Explorer knows about.
pub fn selected_paths(explorer_window: HWND) -> Result<Vec<PathBuf>> {
    let tab = active_tab(explorer_window);
    // SAFETY: plain COM calls on interfaces this function owns; the thread is in an STA.
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        for index in 0..windows.Count()? {
            let Ok(window) = windows.Item(&index_variant(index)) else {
                continue;
            };
            let browser: IShellBrowser = window
                .cast::<IServiceProvider>()?
                .QueryService(&SID_STopLevelBrowser)?;
            if browser.GetWindow()? == tab {
                return selection(&browser);
            }
        }
    }
    Ok(Vec::new())
}

/// The window Explorer's shell browser reports: the active tab on Windows 11,
/// the Explorer window itself where there are no tabs.
fn active_tab(explorer_window: HWND) -> HWND {
    let class: Vec<u16> = TAB_CLASS.encode_utf16().chain([0]).collect();
    // SAFETY: `class` is a NUL-terminated UTF-16 buffer that outlives the call.
    let tab = unsafe {
        FindWindowExW(
            explorer_window.0,
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    if tab.is_null() {
        explorer_window
    } else {
        HWND(tab)
    }
}

/// # Safety
/// The thread must be in an STA; `browser` must be a live shell browser.
unsafe fn selection(browser: &IShellBrowser) -> Result<Vec<PathBuf>> {
    // SAFETY: plain COM calls; the path text is freed with the allocator that made it.
    unsafe {
        let view: IFolderView2 = browser.QueryActiveShellView()?.cast()?;
        let Ok(items) = view.GetSelection(false) else {
            // Explorer reports an empty selection as a failure.
            return Ok(Vec::new());
        };
        let count = items.GetCount()?.min(MAX_ITEMS);
        let mut paths = Vec::new();
        for index in 0..count {
            let text = items.GetItemAt(index)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = text.to_string();
            CoTaskMemFree(Some(text.0.cast_const().cast()));
            paths.push(PathBuf::from(path?));
        }
        Ok(paths)
    }
}

fn index_variant(index: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    // SAFETY: sets the type tag and the matching 32-bit integer of a zeroed VARIANT.
    unsafe {
        let value = &mut *variant.Anonymous.Anonymous;
        value.vt = VT_I4;
        value.Anonymous.lVal = index;
    }
    variant
}
