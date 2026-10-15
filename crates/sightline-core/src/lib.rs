//! File readers and summaries for Sightline. Independent of any user interface.

pub mod insights;
pub mod profile;
pub mod readers;
pub mod summary;

pub use insights::insights;
pub use profile::profile;
pub use readers::{CsvReader, Limits, ParquetReader, ReadError, Reader, summarize};
pub use summary::{
    Basis, Bounds, ColumnProfile, ColumnSummary, ColumnType, Insight, InsightKind, NullCount,
    RowCount, Shape, Summary, TableFormat, TableSummary, TextEncoding,
};
