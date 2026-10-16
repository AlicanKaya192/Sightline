#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod array_view;
mod autostart;
mod cli;
mod clipboard;
mod explorer;
mod file_view;
mod hook;
mod icon;
mod install;
mod known_folder;
mod language;
mod model_view;
mod notebook_file;
mod pickle_view;
mod preview;
mod registry;
mod resident;
mod shell_menu;
mod summary_text;
mod tray;
mod trigger;
mod view;

use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};

use cli::{MenuChange, Setup};
use preview::Previewer;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = cli::parse_args(std::env::args().skip(1))?;
    let window = AppWindow::new()?;
    window.set_version(env!("CARGO_PKG_VERSION").into());
    let saved_language = language::saved();
    window.set_language(saved_language);
    // Slint picks the system language on its own; a saved choice or the flag overrides it.
    match &options.language {
        Some(language) => slint::select_bundled_translation(language)?,
        None if saved_language != LanguageChoice::System => language::select(saved_language)?,
        None => {}
    }
    if let Some(change) = options.menu {
        return change_menu(change, &window);
    }
    if let Some(setup) = options.setup {
        run_setup(setup, &window);
        return Ok(());
    }
    let previewer = Previewer::new(&window);
    open_dropped_files(&window, previewer.clone());
    let notebook_previewer = previewer.clone();
    window.on_open_notebook(move || notebook_previewer.open_notebook());
    close_on_escape(&window);
    copy_summary_on_request(&window);
    let language_previewer = previewer.clone();
    let app = window.as_weak();
    window.on_language_chosen(move |choice| {
        if let Some(window) = app.upgrade() {
            change_language(&window, &language_previewer, choice);
        }
    });
    if let Some(file) = options.file {
        previewer.open(file);
    }
    if options.tray {
        return resident::run(&window, previewer);
    }
    window.run()?;
    Ok(())
}

/// Applies a language picked on the About screen and keeps it for the next start.
pub fn change_language(window: &AppWindow, previewer: &Previewer, choice: LanguageChoice) {
    window.set_language(choice);
    // Failures are not reported: the window still switches; only the next start may not.
    let _ = language::save(choice);
    // Fails only for a language that is not bundled, and every choice is.
    let _ = language::select(choice);
    // Numbers are formatted when a file is shown, so the shown file is shown again.
    previewer.refresh();
    let _ = shell_menu::relabel(&window.get_menu_verb());
}

/// "Copy summary" puts the shown summary on the clipboard; the button says how it went.
fn copy_summary_on_request(window: &AppWindow) {
    let app = window.as_weak();
    window.global::<Copying>().on_copy_summary(move || {
        let Some(window) = app.upgrade() else {
            return;
        };
        let Some(text) = summary_text::shown(&window) else {
            return;
        };
        let state = match clipboard::set_text(&text) {
            Ok(()) => CopyState::Copied,
            Err(_) => CopyState::Failed,
        };
        window.global::<Copying>().set_state(state);
    });
}

/// Hiding is what the close button does: the program ends, or in tray mode it keeps waiting.
fn close_on_escape(window: &AppWindow) {
    let app = window.as_weak();
    window.on_close_window(move || {
        if let Some(window) = app.upgrade() {
            // Nothing to report to when it fails; the window then simply stays open.
            let _ = window.hide();
        }
    });
}

/// Slint does not pass files dragged from Explorer on, so they are taken from winit.
fn open_dropped_files(window: &AppWindow, previewer: Previewer) {
    let app = window.as_weak();
    window
        .window()
        .on_winit_window_event(move |_, event| match event {
            WindowEvent::HoveredFile(_) => {
                set_drop_hover(&app, true);
                EventResult::PreventDefault
            }
            WindowEvent::HoveredFileCancelled => {
                set_drop_hover(&app, false);
                EventResult::PreventDefault
            }
            WindowEvent::DroppedFile(path) => {
                set_drop_hover(&app, false);
                previewer.open(path.clone());
                EventResult::PreventDefault
            }
            _ => EventResult::Propagate,
        });
}

fn set_drop_hover(app: &slint::Weak<AppWindow>, hovering: bool) {
    if let Some(window) = app.upgrade() {
        window.set_drop_hover(hovering);
    }
}

/// Installs or uninstalls and says how it went in a message box, since there is no console.
fn run_setup(setup: Setup, window: &AppWindow) {
    let (result, done, failed) = match setup {
        Setup::Install => (
            install::install(&window.get_menu_verb()).map(|_| ()),
            window.get_install_done(),
            window.get_install_failed(),
        ),
        Setup::Uninstall => (
            install::uninstall(),
            window.get_uninstall_done(),
            window.get_uninstall_failed(),
        ),
    };
    match result {
        Ok(()) => message_box(&done, false),
        Err(error) => message_box(&format!("{failed}\n\n{error}"), true),
    }
}

fn message_box(text: &str, is_error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };

    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain([0]).collect() };
    let icon = if is_error {
        MB_ICONERROR
    } else {
        MB_ICONINFORMATION
    };
    let (text, caption) = (wide(text), wide("Sightline"));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | icon,
        )
    };
}

fn change_menu(change: MenuChange, window: &AppWindow) -> Result<(), Box<dyn std::error::Error>> {
    match change {
        MenuChange::Register => {
            shell_menu::register(&std::env::current_exe()?, &window.get_menu_verb())?;
        }
        MenuChange::Unregister => shell_menu::unregister()?,
    }
    Ok(())
}
