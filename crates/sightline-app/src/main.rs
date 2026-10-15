#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod preview;
mod shell_menu;
mod view;

use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};

use cli::MenuChange;
use preview::Previewer;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = cli::parse_args(std::env::args().skip(1))?;
    let window = AppWindow::new()?;
    // Slint picks the system language on its own; the flag only overrides it.
    if let Some(language) = &options.language {
        slint::select_bundled_translation(language)?;
    }
    if let Some(change) = options.menu {
        return change_menu(change, &window);
    }
    let previewer = Previewer::new(&window);
    open_dropped_files(&window, previewer.clone());
    if let Some(file) = options.file {
        previewer.open(file);
    }
    window.run()?;
    Ok(())
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

fn change_menu(change: MenuChange, window: &AppWindow) -> Result<(), Box<dyn std::error::Error>> {
    match change {
        MenuChange::Register => {
            shell_menu::register(&std::env::current_exe()?, &window.get_menu_verb())?;
        }
        MenuChange::Unregister => shell_menu::unregister()?,
    }
    Ok(())
}
