//! The notification area icon, its menu, and the hidden window that receives their messages.
//! The window procedure only queues events; the thread's message loop handles them through
//! `Tray::next_event`, so slow work never runs inside the window procedure.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, IDI_APPLICATION, LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING, PostMessageW,
    RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_APP, WM_CLOSE, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

use crate::autostart;

/// Message the keyboard hook posts to the tray window; `WPARAM` is the Explorer window.
pub const SPACE_PRESSED: u32 = WM_APP + 1;
/// Message the notification icon sends on mouse events; `LPARAM` is the mouse message.
const ICON_MESSAGE: u32 = WM_APP + 2;

const ICON_ID: u32 = 1;
const OPEN_ID: usize = 1;
const START_WITH_WINDOWS_ID: usize = 2;
const QUIT_ID: usize = 3;

/// What the shell thread has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    SpacePressed { explorer: HWND },
    Open,
    ToggleStartWithWindows,
    Quit,
}

/// Menu and tooltip text, already translated.
pub struct TrayText {
    pub open: String,
    pub start_with_windows: String,
    pub quit: String,
    pub tip: String,
}

/// Messages as the window procedure saw them, before `next_event` turns them into events.
enum Raw {
    Event(TrayEvent),
    MenuRequested,
    TaskbarRestarted,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Raw>> = const { RefCell::new(VecDeque::new()) };
    /// Explorer broadcasts this after it restarts; the icon has to be added again.
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// The hidden window with its icon; dropping it removes both.
/// Lives on the thread that created it, which must run a message loop.
pub struct Tray {
    window: HWND,
    text: TrayText,
}

impl Tray {
    pub fn create(text: TrayText) -> io::Result<Self> {
        let class = wide("SightlineTray");
        // SAFETY: `class` outlives the calls; `window_proc` matches `WNDPROC`.
        let window = unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&window_class) == 0 {
                return Err(io::Error::last_os_error());
            }
            TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
            // A plain top-level window that is never shown, so it still gets broadcasts.
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        let tray = Self { window, text };
        tray.add_icon()?;
        Ok(tray)
    }

    pub fn window(&self) -> HWND {
        self.window
    }

    /// The next queued event; shows the menu or restores the icon on the way.
    pub fn next_event(&self) -> Option<TrayEvent> {
        while let Some(raw) = QUEUE.with_borrow_mut(VecDeque::pop_front) {
            match raw {
                Raw::Event(event) => return Some(event),
                Raw::MenuRequested => {
                    if let Some(event) = self.show_menu() {
                        return Some(event);
                    }
                }
                // Nothing else can be done when it fails; the program still runs without it.
                Raw::TaskbarRestarted => _ = self.add_icon(),
            }
        }
        None
    }

    fn add_icon(&self) -> io::Result<()> {
        let mut data = self.icon_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = ICON_MESSAGE;
        // SAFETY: a null instance with `IDI_APPLICATION` loads the stock application icon.
        data.hIcon = unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) };
        let tip: Vec<u16> = self.text.tip.encode_utf16().collect();
        let len = tip.len().min(data.szTip.len() - 1);
        data.szTip[..len].copy_from_slice(&tip[..len]);
        // SAFETY: `data` is a fully initialised `NOTIFYICONDATAW`.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(io::Error::other("the notification icon could not be added"));
        }
        Ok(())
    }

    fn icon_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    fn show_menu(&self) -> Option<TrayEvent> {
        let start_flag = if autostart::is_enabled() {
            MF_CHECKED
        } else {
            0
        };
        let open = wide(&self.text.open);
        let start = wide(&self.text.start_with_windows);
        let quit = wide(&self.text.quit);
        // SAFETY: the menu is created, shown and destroyed here; the texts outlive it.
        let chosen = unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            AppendMenuW(menu, MF_STRING, OPEN_ID, open.as_ptr());
            AppendMenuW(
                menu,
                MF_STRING | start_flag,
                START_WITH_WINDOWS_ID,
                start.as_ptr(),
            );
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(menu, MF_STRING, QUIT_ID, quit.as_ptr());
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor);
            // Without these two calls the menu does not close when the user clicks elsewhere.
            SetForegroundWindow(self.window);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                cursor.x,
                cursor.y,
                0,
                self.window,
                ptr::null(),
            );
            PostMessageW(self.window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            chosen
        };
        match chosen as usize {
            OPEN_ID => Some(TrayEvent::Open),
            START_WITH_WINDOWS_ID => Some(TrayEvent::ToggleStartWithWindows),
            QUIT_ID => Some(TrayEvent::Quit),
            _ => None,
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.icon_data();
        // SAFETY: removes the icon and destroys the window this value created.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            DestroyWindow(self.window);
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let raw = match message {
        SPACE_PRESSED => Some(Raw::Event(TrayEvent::SpacePressed {
            explorer: wparam as HWND,
        })),
        ICON_MESSAGE => match lparam as u32 {
            WM_LBUTTONUP => Some(Raw::Event(TrayEvent::Open)),
            WM_RBUTTONUP => Some(Raw::MenuRequested),
            _ => None,
        },
        // Lets another program close Sightline the normal way, as with any window.
        WM_CLOSE => Some(Raw::Event(TrayEvent::Quit)),
        _ if message == TASKBAR_CREATED.get() => Some(Raw::TaskbarRestarted),
        _ => None,
    };
    match raw {
        Some(raw) => {
            QUEUE.with_borrow_mut(|queue| queue.push_back(raw));
            0
        }
        // SAFETY: every other message gets the default handling.
        None => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
