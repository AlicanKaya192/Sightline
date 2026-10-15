use std::fmt;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `ozet <file>`: print the summary of one file.
    Summary(PathBuf),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArgsError {
    MissingCommand,
    UnknownCommand(String),
    MissingPath,
    UnexpectedArgument(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => write!(f, "usage: sightline {SUMMARY_COMMAND} <file>"),
            Self::UnknownCommand(command) => {
                write!(f, "unknown command: {command} (try `{SUMMARY_COMMAND}`)")
            }
            Self::MissingPath => write!(f, "{SUMMARY_COMMAND} needs a file path"),
            Self::UnexpectedArgument(arg) => write!(f, "unexpected argument: {arg}"),
        }
    }
}

impl std::error::Error for ArgsError {}

const SUMMARY_COMMAND: &str = "ozet";

/// Parses the command line arguments after the program name.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, ArgsError> {
    let mut args = args.into_iter();
    let command = args.next().ok_or(ArgsError::MissingCommand)?;
    if command != SUMMARY_COMMAND {
        return Err(ArgsError::UnknownCommand(command));
    }
    let path = args.next().ok_or(ArgsError::MissingPath)?;
    if let Some(extra) = args.next() {
        return Err(ArgsError::UnexpectedArgument(extra));
    }
    Ok(Command::Summary(PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, ArgsError> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn summary_command_takes_a_path() {
        assert_eq!(
            parse(&["ozet", "data.parquet"]),
            Ok(Command::Summary(PathBuf::from("data.parquet")))
        );
    }

    #[test]
    fn no_arguments_is_an_error() {
        assert_eq!(parse(&[]), Err(ArgsError::MissingCommand));
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert_eq!(
            parse(&["open", "data.csv"]),
            Err(ArgsError::UnknownCommand("open".into()))
        );
    }

    #[test]
    fn summary_without_path_is_an_error() {
        assert_eq!(parse(&["ozet"]), Err(ArgsError::MissingPath));
    }

    #[test]
    fn extra_argument_is_an_error() {
        assert_eq!(
            parse(&["ozet", "a.csv", "b.csv"]),
            Err(ArgsError::UnexpectedArgument("b.csv".into()))
        );
    }
}
