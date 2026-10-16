//! Tray mode: Sightline waits in the notification area and previews the file selected in
//! Explorer when Space is pressed. The preview opens in this process, so no program has to
//! start; closing the window only hides it.
//!
//! A shell thread owns the keyboard hook, the tray icon and their message loop, and asks
//! Explorer for its selection; the Slint event loop stays on the main thread.

use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, MSG,
    PostMessageW, SetForegroundWindow, TranslateMessage,
};

use crate::hook::KeyboardHook;
use crate::preview::Previewer;
use crate::tray::{self, Tray, TrayEvent, TrayText};
use crate::{AppWindow, autostart, change_language, explorer, trigger};

/// Runs until the user picks Quit in the tray menu. The window stays hidden until needed.
pub fn run(window: &AppWindow, previewer: Previewer) -> Result<(), Box<dyn Error>> {
    let text = Arc::new(Mutex::new(tray_text(window)));
    // Set by the shell thread once the tray window exists.
    let tray_window = Arc::new(AtomicPtr::new(ptr::null_mut()));
    retranslate_tray(window, previewer.clone(), text.clone(), tray_window.clone());
    let ui = Ui {
        window: window.as_weak(),
        previewer,
    };
    let shell = thread::spawn(move || {
        let result = run_shell(text, &tray_window, &ui);
        // Fails only when the event loop has already stopped.
        let _ = slint::quit_event_loop();
        result
    });
    slint::run_event_loop_until_quit()?;
    shell.join().map_err(|_| "the tray thread panicked")??;
    Ok(())
}

fn tray_text(window: &AppWindow) -> TrayText {
    TrayText {
        open: window.get_tray_open().into(),
        about: window.get_tray_about().into(),
        start_with_windows: window.get_tray_start_with_windows().into(),
        quit: window.get_tray_quit().into(),
        tip: window.get_tray_tip().into(),
    }
}

/// A language picked on the About screen also renames the tray menu and tooltip.
fn retranslate_tray(
    window: &AppWindow,
    previewer: Previewer,
    text: Arc<Mutex<TrayText>>,
    tray_window: Arc<AtomicPtr<std::ffi::c_void>>,
) {
    let app = window.as_weak();
    window.on_language_chosen(move |choice| {
        let Some(window) = app.upgrade() else {
            return;
        };
        change_language(&window, &previewer, choice);
        *text.lock().unwrap_or_else(PoisonError::into_inner) = tray_text(&window);
        let tray = tray_window.load(Ordering::SeqCst);
        if !tray.is_null() {
            // SAFETY: posting to a window handle is safe even if the window is gone.
            unsafe { PostMessageW(tray, tray::TEXT_CHANGED, 0, 0) };
        }
    });
}

/// Hands work to the Slint event loop from the shell thread.
struct Ui {
    window: slint::Weak<AppWindow>,
    previewer: Previewer,
}

impl Ui {
    /// Shows the window, first opening `file` in it when there is one.
    fn show(&self, file: Option<PathBuf>) {
        let window = self.window.clone();
        let previewer = self.previewer.clone();
        // Fails only when the event loop has already stopped.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(file) = file {
                previewer.open(file);
            }
            if let Some(window) = window.upgrade() {
                // Nothing to report to when it fails; the next Space press tries again.
                let _ = window.show();
                bring_to_front(&window);
            }
        });
    }
}

impl Ui {
    fn show_about(&self) {
        let window = self.window.clone();
        // Fails only when the event loop has already stopped.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                window.set_about_open(true);
                let _ = window.show();
                bring_to_front(&window);
            }
        });
    }
}

/// Windows shows a window from a background process behind the foreground one.
/// Joining the foreground thread's input for a moment lets this thread take the foreground,
/// without sending any fake key presses.
fn bring_to_front(window: &AppWindow) {
    window.window().with_winit_window(|winit| {
        let Ok(handle) = winit.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        // SAFETY: plain calls on live window and thread handles; the input is detached again.
        unsafe {
            let foreground_thread =
                GetWindowThreadProcessId(GetForegroundWindow(), ptr::null_mut());
            let this_thread = GetCurrentThreadId();
            let attached = foreground_thread != 0
                && foreground_thread != this_thread
                && AttachThreadInput(this_thread, foreground_thread, 1) != 0;
            SetForegroundWindow(handle.hwnd.get() as _);
            if attached {
                AttachThreadInput(this_thread, foreground_thread, 0);
            }
        }
    });
}

fn run_shell(
    text: Arc<Mutex<TrayText>>,
    tray_window: &AtomicPtr<std::ffi::c_void>,
    ui: &Ui,
) -> Result<(), io::Error> {
    // Explorer's shell windows are reached through COM from a single-threaded apartment.
    // SAFETY: called once on this thread, matched by `CoUninitialize` below.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let result = pump_messages(text, tray_window, ui);
    // SAFETY: every COM object of this thread is released inside `pump_messages`.
    unsafe { CoUninitialize() };
    result
}

fn pump_messages(
    text: Arc<Mutex<TrayText>>,
    tray_window: &AtomicPtr<std::ffi::c_void>,
    ui: &Ui,
) -> Result<(), io::Error> {
    let exe = std::env::current_exe()?;
    let tray = Tray::create(text)?;
    tray_window.store(tray.window(), Ordering::SeqCst);
    let _hook = KeyboardHook::install(tray.window(), tray::SPACE_PRESSED)?;
    let mut message = MSG::default();
    // SAFETY: `message` is a writable `MSG`; the loop only passes it back to the system.
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        while let Some(event) = tray.next_event() {
            match event {
                TrayEvent::SpacePressed { explorer } => preview_selection(explorer, ui),
                TrayEvent::Open => ui.show(None),
                TrayEvent::About => ui.show_about(),
                TrayEvent::ToggleStartWithWindows => {
                    // The menu reads the real state again next time, so a failure shows there.
                    let _ = autostart::set_enabled(&exe, !autostart::is_enabled());
                }
                TrayEvent::Quit => return Ok(()),
            }
        }
    }
    Ok(())
}

fn preview_selection(explorer: windows_sys::Win32::Foundation::HWND, ui: &Ui) {
    // Explorer may refuse while it is busy; Space then simply does what it does in Explorer.
    let Ok(selected) = explorer::selected_paths(HWND(explorer)) else {
        return;
    };
    if let Some(file) = trigger::file_to_open(&selected) {
        ui.show(Some(file.to_path_buf()));
    }
}
