use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub language: Option<String>,
    /// File to preview right after the window opens.
    pub file: Option<PathBuf>,
    pub menu: Option<MenuChange>,
    /// Stay in the notification area and preview files on Space in Explorer.
    pub tray: bool,
    pub setup: Option<Setup>,
}

/// Installs or removes Sightline for the current user instead of opening the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    Install,
    Uninstall,
}

/// Adds or removes the Explorer context menu entry instead of opening the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuChange {
    Register,
    Unregister,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArgsError {
    MissingValue(&'static str),
    UnknownArgument(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(flag) => write!(f, "{flag} needs a value, e.g. {flag} tr"),
            Self::UnknownArgument(arg) => write!(f, "unknown argument: {arg}"),
        }
    }
}

impl std::error::Error for ArgsError {}

const LANG_FLAG: &str = "--lang";
const REGISTER_MENU_FLAG: &str = "--register-menu";
const UNREGISTER_MENU_FLAG: &str = "--unregister-menu";
pub const TRAY_FLAG: &str = "--tray";
const INSTALL_FLAG: &str = "--install";
pub const UNINSTALL_FLAG: &str = "--uninstall";

/// Parses the command line arguments after the program name.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, ArgsError> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            LANG_FLAG => {
                let language = args.next().ok_or(ArgsError::MissingValue(LANG_FLAG))?;
                options.language = Some(language);
            }
            REGISTER_MENU_FLAG => options.menu = Some(MenuChange::Register),
            UNREGISTER_MENU_FLAG => options.menu = Some(MenuChange::Unregister),
            TRAY_FLAG => options.tray = true,
            INSTALL_FLAG => options.setup = Some(Setup::Install),
            UNINSTALL_FLAG => options.setup = Some(Setup::Uninstall),
            _ if arg.starts_with("--") || options.file.is_some() => {
                return Err(ArgsError::UnknownArgument(arg));
            }
            _ => options.file = Some(PathBuf::from(arg)),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, ArgsError> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn no_arguments_keeps_system_language() {
        assert_eq!(parse(&[]), Ok(Options::default()));
    }

    #[test]
    fn lang_flag_selects_language() {
        assert_eq!(
            parse(&["--lang", "tr"]),
            Ok(Options {
                language: Some("tr".into()),
                ..Options::default()
            })
        );
    }

    #[test]
    fn lang_flag_without_value_is_an_error() {
        assert_eq!(parse(&["--lang"]), Err(ArgsError::MissingValue(LANG_FLAG)));
    }

    #[test]
    fn unknown_argument_is_an_error() {
        assert_eq!(
            parse(&["--verbose"]),
            Err(ArgsError::UnknownArgument("--verbose".into()))
        );
    }

    #[test]
    fn file_argument_is_opened() {
        assert_eq!(
            parse(&[r"C:\data\sales.csv"]),
            Ok(Options {
                file: Some(PathBuf::from(r"C:\data\sales.csv")),
                ..Options::default()
            })
        );
    }

    #[test]
    fn file_argument_works_with_the_lang_flag() {
        let options = parse(&["--lang", "en", "a.parquet"]).expect("valid arguments");

        assert_eq!(options.language.as_deref(), Some("en"));
        assert_eq!(options.file, Some(PathBuf::from("a.parquet")));
    }

    #[test]
    fn second_file_argument_is_an_error() {
        assert_eq!(
            parse(&["a.csv", "b.csv"]),
            Err(ArgsError::UnknownArgument("b.csv".into()))
        );
    }

    #[test]
    fn menu_flags_choose_the_menu_change() {
        let register = parse(&["--register-menu"]).expect("valid arguments");
        let unregister = parse(&["--unregister-menu"]).expect("valid arguments");

        assert_eq!(register.menu, Some(MenuChange::Register));
        assert_eq!(unregister.menu, Some(MenuChange::Unregister));
    }

    #[test]
    fn tray_flag_starts_the_tray_mode() {
        let tray = parse(&["--tray"]).expect("valid arguments");

        assert!(tray.tray);
        assert!(!parse(&[]).expect("valid arguments").tray);
    }

    #[test]
    fn setup_flags_install_or_uninstall() {
        let install = parse(&["--install"]).expect("valid arguments");
        let uninstall = parse(&["--uninstall"]).expect("valid arguments");

        assert_eq!(install.setup, Some(Setup::Install));
        assert_eq!(uninstall.setup, Some(Setup::Uninstall));
        assert_eq!(parse(&[]).expect("valid arguments").setup, None);
    }
}
