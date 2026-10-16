//! Turns reader results into the plain values the window shows. No Slint models here,
//! and no user text: the window translates every label itself.

use std::error::Error;
use std::io;

use sightline_core::{
    Basis, ColumnProfile, ColumnSummary, ColumnType, Insight, InsightKind, NullCount, ReadError,
    RowCount, Shape, TableFormat, TableSummary, TextEncoding,
};

use crate::{
    Cell, ColumnKind, FailureInfo, FailureKind, FileFormat, FileInfo, SchemaRow, WarningKind,
    WarningRow,
};

const KILOBYTE: u64 = 1024;
const SIZE_UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];

/// Number separators of the window's language, read from its translations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberFormat {
    pub decimal: char,
    pub thousands: char,
}

impl NumberFormat {
    pub const ENGLISH: Self = Self {
        decimal: '.',
        thousands: ',',
    };

    /// The first character of each separator; English where one is empty.
    pub fn from_separators(decimal: &str, thousands: &str) -> Self {
        Self {
            decimal: decimal.chars().next().unwrap_or(Self::ENGLISH.decimal),
            thousands: thousands.chars().next().unwrap_or(Self::ENGLISH.thousands),
        }
    }

    /// A whole number with grouped thousands, e.g. `32,000,000`.
    pub fn count(&self, number: u64) -> String {
        group_digits(&number.to_string(), self.thousands)
    }

    /// A number rounded to one decimal place, e.g. `6.7`.
    pub fn one_decimal(&self, number: f64) -> String {
        let text = format!("{number:.1}");
        let (whole, fraction) = text.split_once('.').unwrap_or((&text, "0"));
        format!(
            "{}{}{fraction}",
            group_digits(whole, self.thousands),
            self.decimal
        )
    }
}

/// Puts `separator` between groups of three digits, counted from the right.
fn group_digits(digits: &str, separator: char) -> String {
    let (sign, digits) = digits
        .strip_prefix('-')
        .map_or(("", digits), |rest| ("-", rest));
    let mut grouped = String::from(sign);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// Header line values for a file that was read.
pub fn file_info(
    name: &str,
    size_bytes: u64,
    summary: &TableSummary,
    format: &NumberFormat,
) -> FileInfo {
    let (rows, rows_exact) = match summary.rows {
        RowCount::Exact(count) => (count, true),
        RowCount::AtLeast(count) => (count, false),
    };
    let mut info = FileInfo {
        name: name.into(),
        size: size_text(size_bytes, format).into(),
        rows: format.count(rows).into(),
        rows_exact,
        columns: to_int(summary.columns.len()),
        ..FileInfo::default()
    };
    set_sampled(&mut info, summary.sampled_rows, format);
    match summary.format {
        TableFormat::Parquet { row_groups } => {
            info.format = FileFormat::Parquet;
            info.row_groups = to_int(row_groups);
        }
        TableFormat::Csv {
            delimiter,
            encoding,
        } => {
            info.format = FileFormat::Csv;
            info.delimiter = char::from(delimiter).to_string().into();
            info.encoding = encoding_name(encoding).into();
        }
    }
    info
}

/// One schema line per column.
pub fn schema_rows(columns: &[ColumnSummary], format: &NumberFormat) -> Vec<SchemaRow> {
    columns
        .iter()
        .map(|column| schema_row(column, format))
        .collect()
}

/// Bar heights from 0 to 1, relative to each column's tallest bin; empty without a histogram.
pub fn histograms(columns: &[ColumnSummary]) -> Vec<Vec<f32>> {
    columns
        .iter()
        .map(|column| match column.profile.as_ref().map(|p| &p.shape) {
            Some(Shape::Histogram(bins)) => relative_heights(bins),
            _ => Vec::new(),
        })
        .collect()
}

/// One line per insight; the window writes the sentence around the values.
pub fn warning_rows(insights: &[Insight], format: &NumberFormat) -> Vec<WarningRow> {
    insights
        .iter()
        .map(|insight| warning_row(insight, format))
        .collect()
}

/// The first rows, with missing values marked instead of shown as text.
pub fn head_rows(head: &[Vec<Option<String>>]) -> Vec<Vec<Cell>> {
    head.iter()
        .map(|row| row.iter().map(|value| cell(value.as_deref())).collect())
        .collect()
}

/// What went wrong, in terms the window has a translated message for.
pub fn failure_info(file_name: &str, error: &ReadError) -> FailureInfo {
    FailureInfo {
        kind: failure_kind(error),
        file_name: file_name.into(),
        detail: error.to_string().into(),
    }
}

/// File size with a binary unit, e.g. `1.5 KB`.
pub fn size_text(bytes: u64, format: &NumberFormat) -> String {
    if bytes < KILOBYTE {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / KILOBYTE as f64;
    let mut unit = 0;
    while size >= KILOBYTE as f64 && unit + 1 < SIZE_UNITS.len() {
        size /= KILOBYTE as f64;
        unit += 1;
    }
    format!("{} {}", format.one_decimal(size), SIZE_UNITS[unit])
}

/// Slint numbers are `i32`; counts that large are shown as the largest value.
pub fn to_int(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn encoding_name(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::Utf8 => "UTF-8",
        TextEncoding::Windows1254 => "Windows-1254",
    }
}

fn schema_row(column: &ColumnSummary, format: &NumberFormat) -> SchemaRow {
    let mut row = SchemaRow {
        name: column.name.as_str().into(),
        kind: column_kind(column.column_type),
        ..SchemaRow::default()
    };
    if let Some(nulls) = column.nulls {
        set_nulls(&mut row, nulls, format);
    }
    if let Some(bounds) = &column.bounds {
        row.bounds_known = true;
        row.min = bounds.min.as_str().into();
        row.max = bounds.max.as_str().into();
    }
    if let Some(profile) = &column.profile {
        set_profile(&mut row, profile, format);
    }
    row
}

fn set_nulls(row: &mut SchemaRow, nulls: NullCount, format: &NumberFormat) {
    row.nulls_known = true;
    row.nulls_ratio = nulls.ratio() as f32;
    row.nulls_percent = format.one_decimal(nulls.ratio() * 100.0).into();
}

fn set_profile(row: &mut SchemaRow, profile: &ColumnProfile, format: &NumberFormat) {
    row.distinct_known = true;
    row.distinct = format.count(profile.distinct).into();
    match &profile.shape {
        Shape::Histogram(_) => row.has_histogram = true,
        Shape::TopValues(top) => {
            // A value seen once says nothing about the column.
            if let Some((value, count)) = top.first().filter(|(_, count)| *count > 1) {
                let share = *count as f64 / profile.values.max(1) as f64 * 100.0;
                row.top_value = value.as_str().into();
                row.top_share = format.one_decimal(share).into();
            }
        }
        Shape::None => {}
    }
}

fn set_sampled(info: &mut FileInfo, sampled_rows: Option<u64>, format: &NumberFormat) {
    if let Some(rows) = sampled_rows {
        info.sampled = true;
        info.sampled_rows = format.count(rows).into();
    }
}

pub fn relative_heights(bins: &[u64]) -> Vec<f32> {
    let tallest = bins.iter().copied().max().unwrap_or(0).max(1) as f32;
    bins.iter().map(|&count| count as f32 / tallest).collect()
}

fn warning_row(insight: &Insight, format: &NumberFormat) -> WarningRow {
    let (kind, first, second) = match &insight.kind {
        InsightKind::HighNulls { ratio } => (
            WarningKind::HighNulls,
            format.one_decimal(ratio * 100.0),
            String::new(),
        ),
        InsightKind::LooksLikeId => (WarningKind::LooksLikeId, String::new(), String::new()),
        InsightKind::SingleValue { value } => {
            (WarningKind::SingleValue, value.clone(), String::new())
        }
        InsightKind::Skewed { skewness } => (
            WarningKind::Skewed,
            format.one_decimal(*skewness),
            String::new(),
        ),
        InsightKind::DateRange { from, to } => (WarningKind::DateRange, from.clone(), to.clone()),
    };
    WarningRow {
        kind,
        column: insight.column.as_str().into(),
        first: first.into(),
        second: second.into(),
        from_sample: insight.basis == Basis::Sample,
    }
}

fn column_kind(column_type: ColumnType) -> ColumnKind {
    match column_type {
        ColumnType::Boolean => ColumnKind::Boolean,
        ColumnType::Integer => ColumnKind::Integer,
        ColumnType::Float => ColumnKind::Float,
        ColumnType::Decimal => ColumnKind::Decimal,
        ColumnType::Date => ColumnKind::Date,
        ColumnType::Time => ColumnKind::Time,
        ColumnType::DateTime => ColumnKind::DateTime,
        ColumnType::Text => ColumnKind::Text,
        ColumnType::Binary => ColumnKind::Binary,
        ColumnType::Nested => ColumnKind::Nested,
    }
}

fn cell(value: Option<&str>) -> Cell {
    Cell {
        text: value.unwrap_or_default().into(),
        missing: value.is_none(),
    }
}

fn failure_kind(error: &ReadError) -> FailureKind {
    if let ReadError::UnsupportedFormat(_) = error {
        return FailureKind::UnsupportedFormat;
    }
    match io_error_in(error).map(io::Error::kind) {
        Some(io::ErrorKind::NotFound) => FailureKind::NotFound,
        Some(io::ErrorKind::PermissionDenied) => FailureKind::PermissionDenied,
        Some(io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof) | None => {
            FailureKind::UnreadableContent
        }
        Some(_) => FailureKind::Other,
    }
}

/// The I/O error behind `error`, if any; the CSV and Parquet errors can wrap one.
fn io_error_in(error: &ReadError) -> Option<&io::Error> {
    let mut current: Option<&(dyn Error + 'static)> = error.source();
    while let Some(error) = current {
        if let Some(io_error) = error.downcast_ref::<io::Error>() {
            return Some(io_error);
        }
        current = error.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use sightline_core::{
        Basis, Bounds, ColumnType, InsightKind, Limits, NullCount, RowCount, Shape, TableFormat,
        TextEncoding, summarize,
    };

    use super::*;
    use crate::{ColumnKind, FileFormat, WarningKind};

    const ENGLISH: NumberFormat = NumberFormat::ENGLISH;
    const TURKISH: NumberFormat = NumberFormat {
        decimal: ',',
        thousands: '.',
    };

    fn table(format: TableFormat, rows: RowCount) -> TableSummary {
        TableSummary {
            format,
            rows,
            columns: vec![
                column("id", ColumnType::Integer),
                column("ad", ColumnType::Text),
            ],
            head: vec![],
            insights: vec![],
            sampled_rows: None,
        }
    }

    fn column(name: &str, column_type: ColumnType) -> ColumnSummary {
        ColumnSummary {
            name: name.into(),
            column_type,
            nulls: None,
            bounds: None,
            profile: None,
        }
    }

    fn profile(values: u64, distinct: u64, shape: Shape) -> Option<ColumnProfile> {
        Some(ColumnProfile {
            values,
            distinct,
            basis: Basis::WholeFile,
            shape,
            skewness: None,
            single_value: None,
        })
    }

    fn insight(kind: InsightKind, basis: Basis) -> Insight {
        Insight {
            column: "c".into(),
            kind,
            basis,
        }
    }

    #[test]
    fn parquet_info_has_row_groups_and_exact_rows() {
        let summary = table(
            TableFormat::Parquet { row_groups: 3 },
            RowCount::Exact(1200),
        );

        let info = file_info("sales.parquet", 2048, &summary, &ENGLISH);

        assert_eq!(
            info,
            FileInfo {
                name: "sales.parquet".into(),
                size: "2.0 KB".into(),
                format: FileFormat::Parquet,
                row_groups: 3,
                rows: "1,200".into(),
                rows_exact: true,
                columns: 2,
                ..FileInfo::default()
            }
        );
    }

    #[test]
    fn csv_info_has_delimiter_and_encoding() {
        let format = TableFormat::Csv {
            delimiter: b';',
            encoding: TextEncoding::Windows1254,
        };

        let info = file_info("a.csv", 10, &table(format, RowCount::Exact(4)), &ENGLISH);

        assert_eq!(info.format, FileFormat::Csv);
        assert_eq!(info.delimiter, ";");
        assert_eq!(info.encoding, "Windows-1254");
    }

    #[test]
    fn row_count_lower_bound_is_not_exact() {
        let summary = table(
            TableFormat::Parquet { row_groups: 1 },
            RowCount::AtLeast(500),
        );

        let info = file_info("a.parquet", 10, &summary, &ENGLISH);

        assert_eq!((info.rows.as_str(), info.rows_exact), ("500", false));
    }

    #[test]
    fn size_uses_bytes_below_one_kilobyte() {
        assert_eq!(size_text(0, &ENGLISH), "0 B");
        assert_eq!(size_text(1023, &ENGLISH), "1023 B");
    }

    #[test]
    fn size_uses_the_largest_whole_unit() {
        assert_eq!(size_text(1536, &ENGLISH), "1.5 KB");
        assert_eq!(size_text(5 * 1024 * 1024, &ENGLISH), "5.0 MB");
        assert_eq!(size_text(1_082_190_668, &ENGLISH), "1.0 GB");
    }

    #[test]
    fn size_uses_the_language_decimal_separator() {
        assert_eq!(size_text(1536, &TURKISH), "1,5 KB");
    }

    #[test]
    fn counts_group_thousands_in_the_language_style() {
        assert_eq!(ENGLISH.count(32_000_000), "32,000,000");
        assert_eq!(TURKISH.count(32_000_000), "32.000.000");
        assert_eq!(TURKISH.count(1000), "1.000");
        assert_eq!(TURKISH.count(999), "999");
        assert_eq!(TURKISH.count(0), "0");
    }

    #[test]
    fn decimals_are_rounded_to_one_place() {
        assert_eq!(TURKISH.one_decimal(6.66), "6,7");
        assert_eq!(ENGLISH.one_decimal(0.0), "0.0");
        assert_eq!(TURKISH.one_decimal(1234.5), "1.234,5");
    }

    #[test]
    fn separators_come_from_the_translation() {
        assert_eq!(NumberFormat::from_separators(",", "."), TURKISH);
        assert_eq!(NumberFormat::from_separators("", ""), ENGLISH);
    }

    #[test]
    fn schema_row_shows_whole_file_nulls_and_bounds() {
        let mut id = column("id", ColumnType::Integer);
        id.nulls = Some(NullCount {
            count: 1,
            of_rows: 4,
            basis: Basis::WholeFile,
        });
        id.bounds = Some(Bounds {
            min: "1".into(),
            max: "9".into(),
        });

        let rows = schema_rows(&[id], &ENGLISH);

        assert_eq!(
            rows,
            [SchemaRow {
                name: "id".into(),
                kind: ColumnKind::Integer,
                nulls_known: true,
                nulls_ratio: 0.25,
                nulls_percent: "25.0".into(),
                bounds_known: true,
                min: "1".into(),
                max: "9".into(),
                ..SchemaRow::default()
            }]
        );
    }

    #[test]
    fn a_sampled_file_names_its_sample_size() {
        let mut summary = table(
            TableFormat::Parquet { row_groups: 1 },
            RowCount::Exact(50_000),
        );
        summary.sampled_rows = Some(10_000);

        let info = file_info("a.parquet", 10, &summary, &ENGLISH);

        assert!(info.sampled);
        assert_eq!(info.sampled_rows, "10,000");
    }

    #[test]
    fn a_whole_file_is_not_marked_as_sampled() {
        let summary = table(TableFormat::Parquet { row_groups: 1 }, RowCount::Exact(5));

        let info = file_info("a.parquet", 10, &summary, &ENGLISH);

        assert!(!info.sampled);
    }

    #[test]
    fn schema_row_shows_distinct_count_and_top_value() {
        let mut city = column("city", ColumnType::Text);
        city.profile = profile(
            1000,
            1200,
            Shape::TopValues(vec![("Ankara".into(), 340), ("İzmir".into(), 10)]),
        );

        let row = &schema_rows(&[city], &ENGLISH)[0];

        assert!(row.distinct_known);
        assert_eq!(row.distinct, "1,200");
        assert_eq!(
            (row.top_value.as_str(), row.top_share.as_str()),
            ("Ankara", "34.0")
        );
        assert!(!row.has_histogram);
    }

    #[test]
    fn top_value_is_left_out_when_no_value_repeats() {
        let mut note = column("note", ColumnType::Text);
        note.profile = profile(3, 3, Shape::TopValues(vec![("a".into(), 1)]));

        let row = &schema_rows(&[note], &ENGLISH)[0];

        assert_eq!(row.top_value, "");
    }

    #[test]
    fn schema_row_marks_a_histogram() {
        let mut amount = column("amount", ColumnType::Float);
        amount.profile = profile(3, 3, Shape::Histogram(vec![1, 2]));

        let row = &schema_rows(&[amount], &ENGLISH)[0];

        assert!(row.has_histogram);
        assert_eq!(row.top_value, "");
    }

    #[test]
    fn histogram_bars_are_relative_to_the_tallest_bin() {
        let mut amount = column("amount", ColumnType::Float);
        amount.profile = profile(7, 7, Shape::Histogram(vec![1, 2, 4, 0]));
        let mut name = column("name", ColumnType::Text);
        name.profile = profile(1, 1, Shape::TopValues(vec![("a".into(), 1)]));

        let bars = histograms(&[amount, name, column("x", ColumnType::Text)]);

        assert_eq!(bars, [vec![0.25, 0.5, 1.0, 0.0], vec![], vec![]]);
    }

    #[test]
    fn warning_rows_carry_formatted_values() {
        let found = [
            insight(InsightKind::HighNulls { ratio: 0.25 }, Basis::WholeFile),
            insight(InsightKind::Skewed { skewness: -2.54 }, Basis::WholeFile),
            insight(
                InsightKind::DateRange {
                    from: "2026-09-01".into(),
                    to: "2026-09-12".into(),
                },
                Basis::WholeFile,
            ),
        ];

        let rows = warning_rows(&found, &TURKISH);

        let summary: Vec<(WarningKind, &str, &str)> = rows
            .iter()
            .map(|r| (r.kind, r.first.as_str(), r.second.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (WarningKind::HighNulls, "25,0", ""),
                (WarningKind::Skewed, "-2,5", ""),
                (WarningKind::DateRange, "2026-09-01", "2026-09-12"),
            ]
        );
        assert_eq!(rows[0].column, "c");
    }

    #[test]
    fn warning_rows_name_single_values_and_sampled_findings() {
        let found = [
            insight(
                InsightKind::SingleValue { value: "TR".into() },
                Basis::WholeFile,
            ),
            insight(InsightKind::LooksLikeId, Basis::Sample),
        ];

        let rows = warning_rows(&found, &ENGLISH);

        assert_eq!(
            (rows[0].kind, rows[0].first.as_str()),
            (WarningKind::SingleValue, "TR")
        );
        assert!(!rows[0].from_sample);
        assert_eq!(rows[1].kind, WarningKind::LooksLikeId);
        assert!(rows[1].from_sample);
    }

    #[test]
    fn schema_row_without_statistics_is_marked_unknown() {
        let row = &schema_rows(&[column("ad", ColumnType::Text)], &ENGLISH)[0];

        assert!(!row.nulls_known);
        assert!(!row.bounds_known);
    }

    #[test]
    fn every_column_type_has_its_own_kind() {
        let types = [
            (ColumnType::Boolean, ColumnKind::Boolean),
            (ColumnType::Integer, ColumnKind::Integer),
            (ColumnType::Float, ColumnKind::Float),
            (ColumnType::Decimal, ColumnKind::Decimal),
            (ColumnType::Date, ColumnKind::Date),
            (ColumnType::Time, ColumnKind::Time),
            (ColumnType::DateTime, ColumnKind::DateTime),
            (ColumnType::Text, ColumnKind::Text),
            (ColumnType::Binary, ColumnKind::Binary),
            (ColumnType::Nested, ColumnKind::Nested),
        ];
        let columns: Vec<ColumnSummary> = types.iter().map(|(t, _)| column("c", *t)).collect();

        let kinds: Vec<ColumnKind> = schema_rows(&columns, &ENGLISH)
            .iter()
            .map(|r| r.kind)
            .collect();

        assert_eq!(kinds, types.map(|(_, kind)| kind));
    }

    #[test]
    fn head_rows_mark_missing_values() {
        let head = vec![
            vec![Some("1".into()), None],
            vec![Some("".into()), Some("x".into())],
        ];

        let rows = head_rows(&head);

        let cell = |text: &str, missing| Cell {
            text: text.into(),
            missing,
        };
        assert_eq!(
            rows,
            [
                vec![cell("1", false), cell("", true)],
                vec![cell("", false), cell("x", false)]
            ]
        );
    }

    fn scratch_file(name: &str, content: &[u8]) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/app-tests");
        fs::create_dir_all(&dir).expect("create test directory");
        let path = dir.join(name);
        fs::write(&path, content).expect("write test file");
        path
    }

    fn read_error(path: &Path) -> ReadError {
        summarize(path, &Limits::default()).expect_err("reading should fail")
    }

    #[test]
    fn missing_file_is_not_found() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-file.csv");

        let failure = failure_info("no-such-file.csv", &read_error(&path));

        assert_eq!(failure.kind, FailureKind::NotFound);
        assert_eq!(failure.file_name, "no-such-file.csv");
    }

    #[test]
    fn unknown_extension_is_unsupported() {
        let path = scratch_file("notes.txt", b"hello");

        let failure = failure_info("notes.txt", &read_error(&path));

        assert_eq!(failure.kind, FailureKind::UnsupportedFormat);
    }

    #[test]
    fn broken_parquet_is_unreadable_content() {
        let path = scratch_file("broken.parquet", b"this is not parquet");

        let failure = failure_info("broken.parquet", &read_error(&path));

        assert_eq!(failure.kind, FailureKind::UnreadableContent);
        assert!(!failure.detail.is_empty());
    }
}
