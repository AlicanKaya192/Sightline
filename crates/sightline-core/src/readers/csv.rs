//! CSV: reads the start of the file, detects encoding and delimiter, infers types.

use std::cmp::Ordering;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use encoding_rs::WINDOWS_1254;

use super::{Limits, ReadError, Reader, has_extension};
use crate::insights::insights;
use crate::profile::profile;
use crate::summary::{
    Basis, Bounds, ColumnSummary, ColumnType, NullCount, RowCount, Summary, TableFormat,
    TableSummary, TextEncoding,
};

/// Ties go to the first candidate.
const DELIMITERS: [u8; 2] = *b",;";
/// Records parsed per candidate delimiter.
const DETECTION_ROWS: usize = 20;
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

pub struct CsvReader;

impl Reader for CsvReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "csv")
    }

    fn summarize(&self, path: &Path, limits: &Limits) -> Result<Summary, ReadError> {
        let start = read_start(path, limits.sample_bytes)?;
        let (text, encoding) = decode(&start);
        let text = if start.is_whole_file {
            text.as_str()
        } else {
            without_partial_last_line(&text)
        };
        let delimiter = detect_delimiter(text);
        let table = parse(text, delimiter, limits.sample_rows.max(limits.head_rows))?;
        let is_complete = start.is_whole_file && table.reached_end;
        let sample = &table.rows[..table.rows.len().min(limits.sample_rows)];
        let basis = if is_complete && sample.len() == table.rows.len() {
            Basis::WholeFile
        } else {
            Basis::Sample
        };

        let columns = summarize_columns(&table.header, sample, delimiter, basis);
        Ok(Summary::Table(TableSummary {
            format: TableFormat::Csv {
                delimiter,
                encoding,
            },
            rows: row_count(table.rows.len(), is_complete),
            insights: insights(&columns),
            columns,
            head: table.rows.iter().take(limits.head_rows).cloned().collect(),
            sampled_rows: (basis == Basis::Sample).then_some(sample.len() as u64),
        }))
    }
}

struct FileStart {
    bytes: Vec<u8>,
    is_whole_file: bool,
}

fn read_start(path: &Path, limit: usize) -> Result<FileStart, ReadError> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let limit = limit as u64;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    Ok(FileStart {
        bytes,
        is_whole_file: file_size <= limit,
    })
}

/// UTF-8 if the bytes are valid UTF-8, otherwise Windows-1254 (Turkish Windows).
fn decode(start: &FileStart) -> (String, TextEncoding) {
    let bytes = start.bytes.strip_prefix(UTF8_BOM).unwrap_or(&start.bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), TextEncoding::Utf8),
        // The byte limit can split the last character of a valid UTF-8 file.
        Err(error) if error.error_len().is_none() && !start.is_whole_file => (
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned(),
            TextEncoding::Utf8,
        ),
        Err(_) => (
            WINDOWS_1254
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
            TextEncoding::Windows1254,
        ),
    }
}

fn without_partial_last_line(text: &str) -> &str {
    text.rfind('\n').map_or("", |end| &text[..=end])
}

fn detect_delimiter(text: &str) -> u8 {
    DELIMITERS
        .into_iter()
        .rev()
        .max_by_key(|&delimiter| delimiter_score(text, delimiter))
        .unwrap_or(DELIMITERS[0])
}

/// (rows with as many fields as the first row, fields in the first row);
/// a first row with a single field means the delimiter does not occur.
fn delimiter_score(text: &str, delimiter: u8) -> (usize, usize) {
    let lengths: Vec<usize> = csv_reader(text, delimiter)
        .into_records()
        .take(DETECTION_ROWS)
        .map_while(Result::ok)
        .map(|record| record.len())
        .collect();
    match lengths.first() {
        Some(&fields) if fields > 1 => {
            let agreeing = lengths.iter().filter(|&&n| n == fields).count();
            (agreeing, fields)
        }
        _ => (0, 0),
    }
}

fn csv_reader(text: &str, delimiter: u8) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes())
}

struct ParsedTable {
    header: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
    reached_end: bool,
}

/// The first record is the header; then at most `max_rows` rows.
fn parse(text: &str, delimiter: u8, max_rows: usize) -> Result<ParsedTable, ReadError> {
    let mut records = csv_reader(text, delimiter).into_records();
    let header: Vec<String> = match records.next() {
        Some(record) => record?.iter().map(str::to_string).collect(),
        None => Vec::new(),
    };
    let mut rows = Vec::new();
    for record in records.by_ref().take(max_rows) {
        rows.push(row_values(&record?, header.len()));
    }
    Ok(ParsedTable {
        header,
        rows,
        reached_end: records.next().is_none(),
    })
}

/// One value per header column: empty fields and missing trailing fields are `None`.
fn row_values(record: &csv::StringRecord, columns: usize) -> Vec<Option<String>> {
    (0..columns)
        .map(|index| {
            record
                .get(index)
                .filter(|field| !field.trim().is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn row_count(rows: usize, is_complete: bool) -> RowCount {
    if is_complete {
        RowCount::Exact(rows as u64)
    } else {
        RowCount::AtLeast(rows as u64)
    }
}

fn summarize_columns(
    header: &[String],
    sample: &[Vec<Option<String>>],
    delimiter: u8,
    basis: Basis,
) -> Vec<ColumnSummary> {
    // Turkish and other European CSVs use `;` between fields and `,` in numbers.
    let decimal_comma = delimiter == b';';
    header
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let values: Vec<&str> = sample
                .iter()
                .filter_map(|row| row[index].as_deref())
                .map(str::trim)
                .collect();
            let column_type = infer_type(&values, decimal_comma);
            ColumnSummary {
                name: name.clone(),
                column_type,
                nulls: Some(NullCount {
                    count: (sample.len() - values.len()) as u64,
                    of_rows: sample.len() as u64,
                    basis,
                }),
                // `Bounds` cannot say "from a sample", so a sample gives none.
                bounds: if basis == Basis::WholeFile {
                    value_bounds(&values, column_type, decimal_comma)
                } else {
                    None
                },
                profile: Some(profile(&values, column_type, basis, &|value| {
                    parse_float(value, decimal_comma)
                })),
            }
        })
        .collect()
}

/// The narrowest type that fits every non-empty value; `Text` when there are none.
fn infer_type(values: &[&str], decimal_comma: bool) -> ColumnType {
    let all =
        |matches: &dyn Fn(&str) -> bool| !values.is_empty() && values.iter().all(|v| matches(v));
    if all(&is_boolean) {
        ColumnType::Boolean
    } else if all(&is_integer) {
        ColumnType::Integer
    } else if all(&|value| is_float(value, decimal_comma)) {
        ColumnType::Float
    } else if all(&is_date) {
        ColumnType::Date
    } else if all(&is_datetime) {
        ColumnType::DateTime
    } else {
        ColumnType::Text
    }
}

/// Smallest and largest value as written in the file, in the column type's order.
/// Text has none: byte order is not alphabetical order in Turkish.
fn value_bounds(values: &[&str], column_type: ColumnType, decimal_comma: bool) -> Option<Bounds> {
    match column_type {
        ColumnType::Boolean => bounds_by(values, |v| v.eq_ignore_ascii_case("true")),
        ColumnType::Integer => bounds_by(values, |v| v.parse::<i64>().ok()),
        ColumnType::Float => bounds_by(values, |v| parse_float(v, decimal_comma)),
        ColumnType::Date => bounds_by(values, str::to_string),
        // `T` and a space may both separate date and time.
        ColumnType::DateTime => bounds_by(values, |v| v.replacen('T', " ", 1)),
        _ => None,
    }
}

fn bounds_by<K: PartialOrd>(values: &[&str], key: impl Fn(&str) -> K) -> Option<Bounds> {
    let keyed: Vec<(K, &str)> = values.iter().map(|&value| (key(value), value)).collect();
    let order = |a: &&(K, &str), b: &&(K, &str)| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal);
    let (_, min) = keyed.iter().min_by(order)?;
    let (_, max) = keyed.iter().max_by(order)?;
    Some(Bounds {
        min: min.to_string(),
        max: max.to_string(),
    })
}

fn is_boolean(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false")
}

fn is_integer(value: &str) -> bool {
    value.parse::<i64>().is_ok()
}

fn is_float(value: &str, decimal_comma: bool) -> bool {
    parse_float(value, decimal_comma).is_some()
}

fn parse_float(value: &str, decimal_comma: bool) -> Option<f64> {
    if decimal_comma {
        value.replacen(',', ".", 1).parse().ok()
    } else {
        value.parse().ok()
    }
}

/// `YYYY-MM-DD`
fn is_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    year.len() == 4
        && year.bytes().all(|b| b.is_ascii_digit())
        && two_digits_in(month, 1, 12)
        && two_digits_in(day, 1, 31)
}

/// `YYYY-MM-DD HH:MM[:SS[.fraction]]`, with a space or `T` in between.
fn is_datetime(value: &str) -> bool {
    value
        .split_once(['T', ' '])
        .is_some_and(|(date, time)| is_date(date) && is_time(time))
}

fn is_time(value: &str) -> bool {
    let mut parts = value.split(':');
    let (Some(hours), Some(minutes)) = (parts.next(), parts.next()) else {
        return false;
    };
    let seconds_valid = match (parts.next(), parts.next()) {
        (None, _) => true,
        (Some(seconds), None) => {
            let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, "0"));
            two_digits_in(whole, 0, 59)
                && !fraction.is_empty()
                && fraction.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    };
    two_digits_in(hours, 0, 23) && two_digits_in(minutes, 0, 59) && seconds_valid
}

fn two_digits_in(text: &str, min: u8, max: u8) -> bool {
    text.len() == 2
        && text.bytes().all(|b| b.is_ascii_digit())
        && text.parse::<u8>().is_ok_and(|n| (min..=max).contains(&n))
}
