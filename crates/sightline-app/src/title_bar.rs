//! The window draws its own title bar; this moves, resizes and maximizes the borderless
//! window for it and asks Windows for the shadow it gives framed windows.

use std::cell::Cell;

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::winit::window::ResizeDirection;
use slint::{ComponentHandle, Weak};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

use crate::{AppWindow, ResizeEdge};

pub fn connect(window: &AppWindow) -> FrameEvents {
    let app = window.as_weak();
    window.on_drag_window(move || {
        // The title bar may still believe the button is down after a move ended outside
        // it; a move started then would follow the pointer with no button held.
        if !left_button_down() {
            return;
        }
        if let Some(window) = app.upgrade() {
            window
                .window()
                .with_winit_window(|winit| winit.drag_window());
        }
    });
    let app = window.as_weak();
    window.on_resize_window(move |edge| {
        if let Some(window) = app.upgrade() {
            window
                .window()
                .with_winit_window(|winit| winit.drag_resize_window(direction(edge)));
        }
    });
    let app = window.as_weak();
    window.on_minimize_window(move || {
        if let Some(window) = app.upgrade() {
            window.window().set_minimized(true);
        }
    });
    let app = window.as_weak();
    window.on_toggle_maximized(move || {
        if let Some(window) = app.upgrade() {
            let maximized = window.window().is_maximized();
            window.window().set_maximized(!maximized);
        }
    });
    FrameEvents {
        app: window.as_weak(),
        shadowed: Cell::new(false),
    }
}

/// The title bar's part of the window's winit events; Slint keeps one event filter per
/// window, so the filter passes every event on to `handle`.
pub struct FrameEvents {
    app: Weak<AppWindow>,
    shadowed: Cell<bool>,
}

impl FrameEvents {
    pub fn handle(&self, slint_window: &slint::Window, event: &WindowEvent) {
        // The winit window exists only once the window is shown, so the shadow is asked
        // for on its first event.
        if !self.shadowed.replace(true) {
            slint_window.with_winit_window(|winit| winit.set_undecorated_shadow(true));
        }
        // Also covers maximizing by Win+Up or a snap.
        if matches!(event, WindowEvent::Resized(_))
            && let Some(window) = self.app.upgrade()
        {
            window.set_is_maximized(slint_window.is_maximized());
        }
    }
}

fn left_button_down() -> bool {
    // SAFETY: plain query of the key state; the high bit says the key is down now.
    unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) < 0 }
}

fn direction(edge: ResizeEdge) -> ResizeDirection {
    match edge {
        ResizeEdge::North => ResizeDirection::North,
        ResizeEdge::South => ResizeDirection::South,
        ResizeEdge::East => ResizeDirection::East,
        ResizeEdge::West => ResizeDirection::West,
        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edge_resizes_towards_itself() {
        let pairs = [
            (ResizeEdge::North, ResizeDirection::North),
            (ResizeEdge::South, ResizeDirection::South),
            (ResizeEdge::East, ResizeDirection::East),
            (ResizeEdge::West, ResizeDirection::West),
            (ResizeEdge::NorthEast, ResizeDirection::NorthEast),
            (ResizeEdge::NorthWest, ResizeDirection::NorthWest),
            (ResizeEdge::SouthEast, ResizeDirection::SouthEast),
            (ResizeEdge::SouthWest, ResizeDirection::SouthWest),
        ];
        for (edge, expected) in pairs {
            assert_eq!(direction(edge), expected);
        }
    }
}
