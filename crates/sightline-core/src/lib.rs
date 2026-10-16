//! File readers and summaries for Sightline. Independent of any user interface.

pub mod analysis;
pub mod insights;
pub mod layers;
pub mod notebook;
pub mod profile;
pub mod readers;
pub mod summary;

pub use analysis::analyze;
pub use insights::insights;
pub use layers::{LayerNode, ParameterTotals};
pub use profile::profile;
pub use readers::{
    CsvReader, EXTENSIONS, Limits, NpyReader, OnnxReader, ParquetReader, PickleReader, ReadError,
    Reader, SafetensorsReader, XlsxReader, summarize,
};
pub use summary::{
    Analysis, ArrayStats, ArraySummary, Basis, Bounds, ColumnProfile, ColumnStats, ColumnSummary,
    ColumnType, Correlation, DType, Dimension, FileWarning, Insight, InsightKind, MissingMap,
    ModelFormat, ModelSummary, NullCount, OnnxInfo, PickleImport, PickleSummary, RowCount, Shape,
    SizeCheck, Summary, TableFormat, TableSummary, TensorInfo, TextEncoding, ValueInfo,
};
