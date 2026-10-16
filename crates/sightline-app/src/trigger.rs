//! Decides, from what the keyboard hook sees, whether Space should open a preview.
//! Pure functions only: the hook gathers the facts, this module judges them.

use std::path::{Path, PathBuf};

use crate::shell_menu::EXTENSIONS;

/// Virtual-key code of the Space bar.
pub const VK_SPACE: u32 = 0x20;

/// Window class of an Explorer window (each tab lives inside one).
const EXPLORER_CLASS: &str = "CabinetWClass";

/// Window class of Explorer's file list. Space anywhere else in Explorer (the box shown
/// while renaming, the address bar, the search box, the toolbar) is left alone.
const FILE_LIST_CLASS: &str = "DirectUIHWND";

/// One key event as the low-level keyboard hook reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    pub key: u32,
    pub is_down: bool,
}

/// The low-level hook has no repeat flag, so held keys are told apart by remembering Space.
#[derive(Debug, Default)]
pub struct SpaceWatch {
    space_is_down: bool,
}

impl SpaceWatch {
    /// True only for the first press of Space; repeats while it is held are not.
    pub fn is_fresh_press(&mut self, stroke: KeyStroke) -> bool {
        if stroke.key != VK_SPACE {
            return false;
        }
        let was_down = self.space_is_down;
        self.space_is_down = stroke.is_down;
        stroke.is_down && !was_down
    }
}

/// The window state at the moment Space was pressed.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// Ctrl, Shift, Alt or a Windows key is held.
    pub modifier_held: bool,
    pub foreground_class: &'a str,
    /// Class of the control that has keyboard focus in the foreground window.
    pub focus_class: &'a str,
}

/// True when a fresh Space press in this context should look for a file to preview.
pub fn should_look(context: Context<'_>) -> bool {
    !context.modifier_held
        && context.foreground_class == EXPLORER_CLASS
        && context.focus_class == FILE_LIST_CLASS
}

/// The file to preview: exactly one selected item with a supported extension.
pub fn file_to_open(selected: &[PathBuf]) -> Option<&Path> {
    let [path] = selected else {
        return None;
    };
    let extension = path.extension()?;
    EXTENSIONS
        .iter()
        .any(|supported| extension.eq_ignore_ascii_case(supported))
        .then_some(path.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;

    fn down(key: u32) -> KeyStroke {
        KeyStroke { key, is_down: true }
    }

    fn up(key: u32) -> KeyStroke {
        KeyStroke {
            key,
            is_down: false,
        }
    }

    fn explorer_list() -> Context<'static> {
        Context {
            modifier_held: false,
            foreground_class: "CabinetWClass",
            focus_class: "DirectUIHWND",
        }
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn first_space_press_is_fresh() {
        let mut watch = SpaceWatch::default();

        assert!(watch.is_fresh_press(down(VK_SPACE)));
    }

    #[test]
    fn held_space_repeats_are_not_fresh() {
        let mut watch = SpaceWatch::default();
        watch.is_fresh_press(down(VK_SPACE));

        assert!(!watch.is_fresh_press(down(VK_SPACE)));
        assert!(!watch.is_fresh_press(down(VK_SPACE)));
    }

    #[test]
    fn space_is_fresh_again_after_release() {
        let mut watch = SpaceWatch::default();
        watch.is_fresh_press(down(VK_SPACE));

        assert!(!watch.is_fresh_press(up(VK_SPACE)));
        assert!(watch.is_fresh_press(down(VK_SPACE)));
    }

    #[test]
    fn other_keys_are_never_fresh_space_presses() {
        let mut watch = SpaceWatch::default();

        assert!(!watch.is_fresh_press(down(VK_A)));
        assert!(watch.is_fresh_press(down(VK_SPACE)));
    }

    #[test]
    fn other_keys_do_not_end_a_held_space() {
        let mut watch = SpaceWatch::default();
        watch.is_fresh_press(down(VK_SPACE));
        watch.is_fresh_press(up(VK_A));

        assert!(!watch.is_fresh_press(down(VK_SPACE)));
    }

    #[test]
    fn space_in_the_explorer_file_list_looks_for_a_file() {
        assert!(should_look(explorer_list()));
    }

    #[test]
    fn space_with_a_modifier_does_nothing() {
        let context = Context {
            modifier_held: true,
            ..explorer_list()
        };

        assert!(!should_look(context));
    }

    #[test]
    fn space_while_renaming_a_file_does_nothing() {
        let context = Context {
            focus_class: "Edit",
            ..explorer_list()
        };

        assert!(!should_look(context));
    }

    #[test]
    fn space_in_the_address_bar_search_box_or_toolbar_does_nothing() {
        // Windows 11 draws these parts of Explorer with XAML, all in this window class.
        let context = Context {
            focus_class: "Windows.UI.Input.InputSite.WindowClass",
            ..explorer_list()
        };

        assert!(!should_look(context));
    }

    #[test]
    fn space_in_other_programs_does_nothing() {
        for foreground_class in ["OpusApp", "Chrome_WidgetWin_1", "MozillaWindowClass", ""] {
            let context = Context {
                foreground_class,
                ..explorer_list()
            };

            assert!(!should_look(context), "{foreground_class}");
        }
    }

    #[test]
    fn one_selected_csv_or_parquet_file_is_opened() {
        let csv = paths(&[r"C:\data\sales.csv"]);
        let parquet = paths(&[r"C:\data\trips.parquet"]);

        assert_eq!(file_to_open(&csv), Some(Path::new(r"C:\data\sales.csv")));
        assert_eq!(
            file_to_open(&parquet),
            Some(Path::new(r"C:\data\trips.parquet"))
        );
    }

    #[test]
    fn array_model_and_pickle_files_are_opened() {
        for name in [
            r"C:\models\model.safetensors",
            r"C:\models\model.onnx",
            r"C:\data\embeddings.npy",
            r"C:\models\forest.pkl",
            r"C:\models\forest.pickle",
        ] {
            assert_eq!(
                file_to_open(&paths(&[name])),
                Some(Path::new(name)),
                "{name}"
            );
        }
    }

    #[test]
    fn extension_case_does_not_matter() {
        let selected = paths(&[r"C:\data\SALES.CSV"]);

        assert_eq!(
            file_to_open(&selected),
            Some(Path::new(r"C:\data\SALES.CSV"))
        );
    }

    #[test]
    fn several_selected_files_open_nothing() {
        let selected = paths(&[r"C:\data\a.csv", r"C:\data\b.csv"]);

        assert_eq!(file_to_open(&selected), None);
    }

    #[test]
    fn no_selection_opens_nothing() {
        assert_eq!(file_to_open(&[]), None);
    }

    #[test]
    fn unsupported_or_missing_extensions_open_nothing() {
        for name in [
            r"C:\data\notes.txt",
            r"C:\data\csv",
            r"C:\data\report.csv.zip",
        ] {
            assert_eq!(file_to_open(&paths(&[name])), None, "{name}");
        }
    }
}
