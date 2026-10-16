//! Plain data returned by the readers. No file access and no formatting here.

/// What a reader found in a file.
#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    Table(TableSummary),
    Model(ModelSummary),
    Array(ArraySummary),
    Pickle(PickleSummary),
}

/// Summary of a model file (safetensors, ONNX). Weights are never read.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSummary {
    pub format: ModelFormat,
    /// Weight tensors in the order they are stored.
    pub tensors: Vec<TensorInfo>,
    pub size_check: SizeCheck,
    pub warnings: Vec<FileWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelFormat {
    Safetensors {
        /// The free-form `__metadata__` entries, sorted by key.
        metadata: Vec<(String, String)>,
    },
    Onnx(OnnxInfo),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnnxInfo {
    pub ir_version: i64,
    /// Operator set versions by domain; the default domain is "".
    pub opsets: Vec<(String, i64)>,
    pub producer: String,
    pub inputs: Vec<ValueInfo>,
    pub outputs: Vec<ValueInfo>,
    /// Number of nodes per operator type, most frequent first.
    pub operators: Vec<(String, u64)>,
}

/// A graph input or output.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueInfo {
    pub name: String,
    pub dtype: Option<DType>,
    pub shape: Vec<Dimension>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dimension {
    Fixed(u64),
    /// A size chosen when the model runs, such as `batch`.
    Named(String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TensorInfo {
    pub name: String,
    pub dtype: DType,
    pub shape: Vec<u64>,
}

impl TensorInfo {
    /// Number of values; `None` when it does not fit in 64 bits.
    pub fn elements(&self) -> Option<u64> {
        self.shape
            .iter()
            .try_fold(1u64, |count, &size| count.checked_mul(size))
    }

    /// Bytes the values take; `None` when the size of the type is not known.
    pub fn bytes(&self) -> Option<u64> {
        self.elements()?.checked_mul(self.dtype.bytes()?)
    }
}

/// Element type of a tensor or array.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DType {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F16,
    BF16,
    F32,
    F64,
    F8E4M3,
    F8E5M2,
    Complex64,
    Complex128,
    Text,
    /// A type this version does not know, by the name the file uses.
    Other(String),
}

impl DType {
    /// Bytes per value; `None` for text and unknown types.
    pub fn bytes(&self) -> Option<u64> {
        match self {
            Self::Bool | Self::U8 | Self::I8 | Self::F8E4M3 | Self::F8E5M2 => Some(1),
            Self::U16 | Self::I16 | Self::F16 | Self::BF16 => Some(2),
            Self::U32 | Self::I32 | Self::F32 => Some(4),
            Self::U64 | Self::I64 | Self::F64 | Self::Complex64 => Some(8),
            Self::Complex128 => Some(16),
            Self::Text | Self::Other(_) => None,
        }
    }
}

/// Whether the file is as long as its header says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizeCheck {
    Consistent,
    /// File size in bytes that the header describes, and the real one.
    Mismatch {
        expected: u64,
        actual: u64,
    },
    /// The stored bytes of this tensor do not match its shape and type.
    BadOffsets {
        tensor: String,
    },
    /// A type of unknown size, or a size beyond 64 bits.
    Unknown,
}

/// Summary of a NumPy `.npy` array.
#[derive(Debug, Clone, PartialEq)]
pub struct ArraySummary {
    /// The type as the file writes it, such as `<f4`.
    pub descr: String,
    pub dtype: DType,
    pub shape: Vec<u64>,
    pub fortran_order: bool,
    pub size_check: SizeCheck,
    /// `None` for types without numbers, such as text or objects.
    pub stats: Option<ArrayStats>,
    pub warnings: Vec<FileWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayStats {
    /// Values the numbers below were computed from.
    pub values: u64,
    pub basis: Basis,
    /// Smallest and largest finite value; `None` when there is none.
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub nan: u64,
    pub infinite: u64,
    /// Counts of equal-width bins of the finite values.
    pub histogram: Vec<u64>,
}

/// What a pickle would do when loaded, read from its opcodes without loading it.
#[derive(Debug, Clone, PartialEq)]
pub struct PickleSummary {
    /// From the `PROTO` opcode; `None` for protocols 0 and 1, which have none.
    pub protocol: Option<u8>,
    pub opcodes: u64,
    /// Functions and classes the pickle imports, each once, in order of appearance.
    pub imports: Vec<PickleImport>,
    /// Opcodes that call an imported function or class.
    pub calls: u64,
    pub warnings: Vec<FileWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickleImport {
    pub module: String,
    pub name: String,
    pub dangerous: bool,
}

/// Something worth knowing about a model, array or pickle file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileWarning {
    /// ONNX weights kept in other files, which are not read.
    ExternalData { tensors: u64 },
    /// An object array holds pickled Python objects; its values are not read.
    PickleInside,
    /// No statistics for this element type (text, structured or unknown).
    NoStatistics,
    /// Loading a pickle can run code.
    RunsCodeWhenLoaded,
    /// The pickle imports a function that can harm the computer.
    DangerousImport { module: String, name: String },
    /// Reading stopped at a limit before the end of the file.
    StoppedEarly,
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
    pub analysis: Analysis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableFormat {
    Parquet {
        row_groups: usize,
    },
    Csv {
        delimiter: u8,
        encoding: TextEncoding,
    },
    /// Sheet names in workbook order; the first one is summarized.
    Excel {
        sheets: Vec<String>,
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
    HighNulls {
        ratio: f64,
    },
    LooksLikeId,
    SingleValue {
        value: String,
    },
    Skewed {
        skewness: f64,
    },
    DateRange {
        from: String,
        to: String,
    },
    /// Many zeros far below every other value: zero may stand for "not measured".
    ZerosAsMissing {
        ratio: f64,
    },
    /// Values beyond 1.5 interquartile ranges from the quartiles.
    Outliers {
        count: u64,
        ratio: f64,
        low: f64,
        high: f64,
    },
    /// A column with a few values, nearly all of them one value: a skewed class.
    Imbalanced {
        value: String,
        ratio: f64,
    },
    /// Rows of the sample equal to an earlier row; reported for the table, not a column.
    DuplicateRows {
        count: u64,
        of_rows: u64,
    },
    /// A text column whose values are nearly all numbers; `examples` are the others.
    NumbersAsText {
        ratio: f64,
        examples: Vec<String>,
    },
    /// One category written in more than one way (letter case, spaces at the ends).
    InconsistentSpelling {
        variants: Vec<String>,
    },
    /// Not a single value, but one value nearly everywhere.
    DominantValue {
        value: String,
        ratio: f64,
    },
    /// Nearly the same information as another numeric column.
    HighCorrelation {
        other: String,
        r: f64,
    },
}

/// What the row sample of a table shows beyond each column on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Analysis {
    /// Sample rows behind these numbers.
    pub rows: u64,
    /// Per column, in column order: statistics of numeric columns.
    pub stats: Vec<Option<ColumnStats>>,
    /// Per column: the most frequent values of text and true/false columns.
    pub categories: Vec<Vec<(String, u64)>>,
    pub correlation: Option<Correlation>,
    pub missing: MissingMap,
    /// Sample rows equal to an earlier sample row.
    pub duplicates: u64,
}

/// What pandas' `describe()` gives for a numeric column; the standard deviation is the
/// sample one (n - 1) and quartiles are linearly interpolated, as there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnStats {
    pub count: u64,
    pub mean: f64,
    pub std: f64,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
}

/// Pearson correlations between numeric columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Correlation {
    /// Indexes of the columns, in column order.
    pub columns: Vec<usize>,
    /// Row-major square matrix; `NaN` where a column has no spread.
    pub values: Vec<f64>,
}

/// Which cells of the first sample rows are missing, row by row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissingMap {
    pub rows: usize,
    pub columns: usize,
    pub missing: Vec<bool>,
}
