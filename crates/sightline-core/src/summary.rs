//! Plain data returned by the readers. No file access and no formatting here.

/// What a reader found in a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    Table(TableSummary),
}

/// Summary of a tabular file (CSV, Parquet).
#[derive(Debug, Clone, PartialEq)]
pub struct TableSummary {
    pub format: TableFormat,
    pub rows: RowCount,
    pub columns: Vec<ColumnSummary>,
    /// First rows as text, one entry per column; `None` is a missing value.
    pub head: Vec<Vec<Option<String>>>,
    pub insights: Vec<Insight>,
    /// Rows behind the sample-based numbers, when the sample is not the whole file.
    pub sampled_rows: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFormat {
    Parquet {
        row_groups: usize,
    },
    Csv {
        delimiter: u8,
        encoding: TextEncoding,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Windows1254,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowCount {
    Exact(u64),
    /// The reader stopped before the end of the file.
    AtLeast(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSummary {
    pub name: String,
    pub column_type: ColumnType,
    pub nulls: Option<NullCount>,
    pub bounds: Option<Bounds>,
    pub profile: Option<ColumnProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Boolean,
    Integer,
    Float,
    Decimal,
    Date,
    Time,
    DateTime,
    Text,
    Binary,
    Nested,
}

/// Number of missing values among `of_rows` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NullCount {
    pub count: u64,
    pub of_rows: u64,
    pub basis: Basis,
}

impl NullCount {
    pub fn ratio(&self) -> f64 {
        if self.of_rows == 0 {
            return 0.0;
        }
        self.count as f64 / self.of_rows as f64
    }
}

/// Whether a number covers the whole file or only the rows that were read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    WholeFile,
    Sample,
}

/// Smallest and largest value of a column, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounds {
    pub min: String,
    pub max: String,
}

/// What the non-empty values of one column look like.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnProfile {
    /// Non-empty values the profile was computed from.
    pub values: u64,
    pub distinct: u64,
    pub basis: Basis,
    pub shape: Shape,
    /// Fisher-Pearson skewness of numeric columns with a spread.
    pub skewness: Option<f64>,
    /// The value, when every value is the same.
    pub single_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    None,
    /// Counts of equal-width bins from the smallest to the largest value.
    Histogram(Vec<u64>),
    /// Most frequent values with their counts, most frequent first.
    TopValues(Vec<(String, u64)>),
}

/// Something worth knowing about a column before using it.
#[derive(Debug, Clone, PartialEq)]
pub struct Insight {
    pub column: String,
    pub kind: InsightKind,
    pub basis: Basis,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InsightKind {
    HighNulls { ratio: f64 },
    LooksLikeId,
    SingleValue { value: String },
    Skewed { skewness: f64 },
    DateRange { from: String, to: String },
}
