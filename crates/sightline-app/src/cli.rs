use std::fmt;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub language: Option<String>,
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

/// Parses the command line arguments after the program name.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, ArgsError> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg != LANG_FLAG {
            return Err(ArgsError::UnknownArgument(arg));
        }
        let language = args.next().ok_or(ArgsError::MissingValue(LANG_FLAG))?;
        options.language = Some(language);
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
        assert_eq!(parse(&[]), Ok(Options { language: None }));
    }

    #[test]
    fn lang_flag_selects_language() {
        assert_eq!(
            parse(&["--lang", "tr"]),
            Ok(Options {
                language: Some("tr".into())
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
}
