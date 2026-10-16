//! Low-level keyboard hook that watches for Space in Explorer.
//! It never swallows a key: every event goes on to `CallNextHookEx` unchanged.
//! The callback only gathers cheap facts for `trigger` and posts a message; the slow
//! work (asking Explorer for its selection) happens outside the hook.

use std::cell::{Cell, RefCell};
use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GUITHREADINFO, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowThreadProcessId, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::trigger::{self, Context, KeyStroke, SpaceWatch};

const MODIFIERS: [u16; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];

/// Window class names are at most 256 characters long.
const CLASS_NAME_LEN: usize = 256;

thread_local! {
    /// Where the hook reports a Space press: the window and the message number.
    static TARGET: Cell<Option<(HWND, u32)>> = const { Cell::new(None) };
    static WATCH: RefCell<SpaceWatch> = RefCell::new(SpaceWatch::default());
}

/// The installed hook; dropping it removes the hook.
/// Lives on the thread that installed it, which must run a message loop.
pub struct KeyboardHook(HHOOK);

impl KeyboardHook {
    /// On a Space press that `trigger` accepts, posts `message` to `target`
    /// with the Explorer window in `WPARAM`.
    pub fn install(target: HWND, message: u32) -> io::Result<Self> {
        TARGET.set(Some((target, message)));
        // SAFETY: `on_key` matches `HOOKPROC`; a low-level hook needs no module handle.
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), ptr::null_mut(), 0) };
        if hook.is_null() {
            TARGET.set(None);
            return Err(io::Error::last_os_error());
        }
        Ok(Self(hook))
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `SetWindowsHookExW` and is removed only here.
        unsafe { UnhookWindowsHookEx(self.0) };
        TARGET.set(None);
    }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for `HC_ACTION`, `lparam` points to the event's `KBDLLHOOKSTRUCT`.
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let stroke = KeyStroke {
            key: event.vkCode,
            is_down: matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN),
        };
        if WATCH.with_borrow_mut(|watch| watch.is_fresh_press(stroke)) {
            report_if_explorer();
        }
    }
    // SAFETY: passes the event on unchanged, as every hook must.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn report_if_explorer() {
    let Some((target, message)) = TARGET.get() else {
        return;
    };
    // SAFETY: these calls only read window state; every buffer outlives its call.
    unsafe {
        let foreground = GetForegroundWindow();
        let context = Context {
            modifier_held: MODIFIERS
                .iter()
                .any(|&key| GetAsyncKeyState(key.into()) < 0),
            foreground_class: &class_name(foreground),
            focus_class: &class_name(focused_control(foreground)),
        };
        if trigger::should_look(context) {
            PostMessageW(target, message, foreground as WPARAM, 0);
        }
    }
}

/// The control with keyboard focus in `window`'s thread; null if there is none.
fn focused_control(window: HWND) -> HWND {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a writable `GUITHREADINFO` with `cbSize` set.
    unsafe {
        let thread = GetWindowThreadProcessId(window, ptr::null_mut());
        if thread == 0 || GetGUIThreadInfo(thread, &mut info) == 0 {
            return ptr::null_mut();
        }
    }
    info.hwndFocus
}

fn class_name(window: HWND) -> String {
    if window.is_null() {
        return String::new();
    }
    let mut buffer = [0u16; CLASS_NAME_LEN];
    // SAFETY: `buffer` holds `CLASS_NAME_LEN` characters.
    let len = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), CLASS_NAME_LEN as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}
