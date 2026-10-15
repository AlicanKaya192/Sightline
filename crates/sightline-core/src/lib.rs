//! File readers and summaries for Sightline. Independent of any user interface.

pub mod readers;
pub mod summary;

pub use readers::{CsvReader, Limits, ParquetReader, ReadError, Reader, summarize};
pub use summary::{
    Basis, Bounds, ColumnSummary, ColumnType, NullCount, RowCount, Summary, TableFormat,
    TableSummary, TextEncoding,
};
