//! One reader per file format. Every reader only opens files for reading.

mod csv;
mod npy;
mod onnx;
mod parquet;
mod pickle_inspect;
mod protobuf;
mod py_literal;
mod safetensors;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::summary::Summary;

pub use self::csv::CsvReader;
pub use self::npy::NpyReader;
pub use self::onnx::OnnxReader;
pub use self::parquet::ParquetReader;
pub use self::pickle_inspect::PickleReader;
pub use self::safetensors::SafetensorsReader;

/// File name extensions the readers open, for the shell menu and the Space key.
pub const EXTENSIONS: [&str; 7] = [
    "csv",
    "parquet",
    "safetensors",
    "onnx",
    "npy",
    "pkl",
    "pickle",
];

pub trait Reader {
    /// Decides by the file name only; does not open the file.
    fn can_read(&self, path: &Path) -> bool;
    fn summarize(&self, path: &Path, limits: &Limits) -> Result<Summary, ReadError>;
}

/// How much of a file a reader may look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Rows returned in `TableSummary::head`.
    pub head_rows: usize,
    /// Rows read when a number is not in the file's metadata.
    pub sample_rows: usize,
    /// Bytes read from the start of a text file.
    pub sample_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            head_rows: 20,
            sample_rows: 10_000,
            sample_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Parquet(::parquet::errors::ParquetError),
    Csv(::csv::Error),
    /// The file does not follow its format; `reason` says where.
    Malformed {
        format: &'static str,
        reason: String,
    },
    UnsupportedFormat(PathBuf),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read the file: {error}"),
            Self::Parquet(error) => write!(f, "not a readable Parquet file: {error}"),
            Self::Csv(error) => write!(f, "not a readable CSV file: {error}"),
            Self::Malformed { format, reason } => {
                write!(f, "not a readable {format} file: {reason}")
            }
            Self::UnsupportedFormat(path) => {
                write!(f, "unsupported file type: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parquet(error) => Some(error),
            Self::Csv(error) => Some(error),
            Self::Malformed { .. } | Self::UnsupportedFormat(_) => None,
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<::parquet::errors::ParquetError> for ReadError {
    fn from(error: ::parquet::errors::ParquetError) -> Self {
        Self::Parquet(error)
    }
}

impl From<::csv::Error> for ReadError {
    fn from(error: ::csv::Error) -> Self {
        Self::Csv(error)
    }
}

/// Summarizes `path` with the first reader that accepts its file name.
pub fn summarize(path: &Path, limits: &Limits) -> Result<Summary, ReadError> {
    let readers: [&dyn Reader; 6] = [
        &ParquetReader,
        &CsvReader,
        &SafetensorsReader,
        &OnnxReader,
        &NpyReader,
        &PickleReader,
    ];
    let reader = readers
        .into_iter()
        .find(|reader| reader.can_read(path))
        .ok_or_else(|| ReadError::UnsupportedFormat(path.to_path_buf()))?;
    reader.summarize(path, limits)
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|found| found.to_str())
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}
