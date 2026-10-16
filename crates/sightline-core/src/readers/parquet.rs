//! Parquet: schema, row count and statistics come from the footer metadata.
//! Flat columns are sampled from column pages spread over the whole file, for the
//! profiles and for null counts missing from the metadata. Whole rows are decoded only
//! for the head and for nested columns.

use std::fs::File;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::path::Path;
use std::thread;

use ::parquet::basic::{ConvertedType, LogicalType, Type as PhysicalType};
use ::parquet::column::reader::{ColumnReader, ColumnReaderImpl, get_column_reader};
use ::parquet::data_type::DataType;
use ::parquet::errors::ParquetError;
use ::parquet::file::metadata::ParquetMetaData;
use ::parquet::file::reader::{FileReader, SerializedFileReader};
use ::parquet::file::statistics::{Statistics, ValueStatistics};
use ::parquet::record::{Field, Row};
use ::parquet::schema::types::{ColumnDescPtr, ColumnDescriptor, SchemaDescriptor};

use super::{Limits, ReadError, Reader, has_extension};
use crate::insights::insights;
use crate::profile::profile;
use crate::summary::{
    Basis, Bounds, ColumnProfile, ColumnSummary, ColumnType, NullCount, RowCount, Summary,
    TableFormat, TableSummary,
};

pub struct ParquetReader;

impl Reader for ParquetReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "parquet")
    }

    fn summarize(&self, path: &Path, limits: &Limits) -> Result<Summary, ReadError> {
        let reader = SerializedFileReader::new(File::open(path)?)?;
        let metadata = reader.metadata();
        let total_rows = row_count(metadata)?;
        let mut columns = columns_from_metadata(metadata, total_rows);

        let head = read_rows(&reader, limits.head_rows)?;
        let flat_sample = sample_flat_columns(path, metadata, &mut columns, limits.sample_rows)?;
        let row_sample = if columns.iter().any(|column| column.profile.is_none()) {
            let rows = read_sample(path, metadata.num_row_groups(), limits.sample_rows)?;
            fill_missing_nulls(&mut columns, &rows, total_rows);
            add_profiles(&mut columns, &rows, total_rows);
            Some(rows.len() as u64)
        } else {
            None
        };
        let sampled = flat_sample.or(row_sample);

        Ok(Summary::Table(TableSummary {
            format: TableFormat::Parquet {
                row_groups: metadata.num_row_groups(),
            },
            rows: RowCount::Exact(total_rows),
            insights: insights(&columns),
            columns,
            head: head.iter().map(row_text).collect(),
            sampled_rows: sampled.filter(|&rows| basis_of(rows, total_rows) == Basis::Sample),
        }))
    }
}

fn row_count(metadata: &ParquetMetaData) -> Result<u64, ReadError> {
    let rows = metadata.file_metadata().num_rows();
    u64::try_from(rows)
        .map_err(|_| ParquetError::General(format!("negative row count: {rows}")).into())
}

/// The leaf column of each top-level field; `None` for a nested field.
fn top_level_leaves(schema: &SchemaDescriptor) -> Vec<Option<usize>> {
    let mut leaf_of_field = vec![None; schema.root_schema().get_fields().len()];
    for leaf in 0..schema.num_columns() {
        let field = schema.get_column_root_idx(leaf);
        if schema.get_column_root(leaf).is_primitive() {
            leaf_of_field[field] = Some(leaf);
        }
    }
    leaf_of_field
}

/// One summary per top-level field. Nested fields get no statistics.
fn columns_from_metadata(metadata: &ParquetMetaData, total_rows: u64) -> Vec<ColumnSummary> {
    let schema = metadata.file_metadata().schema_descr();
    schema
        .root_schema()
        .get_fields()
        .iter()
        .zip(top_level_leaves(schema))
        .map(|(field, leaf)| match leaf {
            Some(leaf) => primitive_column(metadata, leaf, total_rows),
            None => ColumnSummary {
                name: field.name().to_string(),
                column_type: ColumnType::Nested,
                nulls: None,
                bounds: None,
                profile: None,
            },
        })
        .collect()
}

fn primitive_column(metadata: &ParquetMetaData, leaf: usize, total_rows: u64) -> ColumnSummary {
    let descriptor = metadata.file_metadata().schema_descr().column(leaf);
    let column_type = column_type(&descriptor);
    let statistics = leaf_statistics(metadata, leaf);
    ColumnSummary {
        name: descriptor.name().to_string(),
        column_type,
        nulls: statistics
            .as_deref()
            .and_then(|stats| null_count(stats, total_rows)),
        bounds: statistics
            .as_deref()
            .and_then(|stats| bounds(stats, column_type, &descriptor)),
        profile: None,
    }
}

/// Statistics of one column in every row group; `None` if any row group has none.
fn leaf_statistics(metadata: &ParquetMetaData, leaf: usize) -> Option<Vec<&Statistics>> {
    metadata
        .row_groups()
        .iter()
        .map(|row_group| row_group.column(leaf).statistics())
        .collect()
}

fn null_count(statistics: &[&Statistics], total_rows: u64) -> Option<NullCount> {
    let count = statistics
        .iter()
        .map(|stats| stats.null_count_opt())
        .sum::<Option<u64>>()?;
    Some(NullCount {
        count,
        of_rows: total_rows,
        basis: Basis::WholeFile,
    })
}

fn column_type(column: &ColumnDescriptor) -> ColumnType {
    column
        .logical_type_ref()
        .and_then(logical_column_type)
        .or_else(|| converted_column_type(column.converted_type()))
        .unwrap_or_else(|| physical_column_type(column.physical_type()))
}

fn logical_column_type(logical: &LogicalType) -> Option<ColumnType> {
    match logical {
        LogicalType::String | LogicalType::Enum | LogicalType::Json => Some(ColumnType::Text),
        LogicalType::Decimal(_) => Some(ColumnType::Decimal),
        LogicalType::Date => Some(ColumnType::Date),
        LogicalType::Time(_) => Some(ColumnType::Time),
        LogicalType::Timestamp(_) => Some(ColumnType::DateTime),
        LogicalType::Integer(_) => Some(ColumnType::Integer),
        LogicalType::Float16 => Some(ColumnType::Float),
        _ => None,
    }
}

/// Older writers set only the converted type.
fn converted_column_type(converted: ConvertedType) -> Option<ColumnType> {
    match converted {
        ConvertedType::UTF8 | ConvertedType::ENUM | ConvertedType::JSON => Some(ColumnType::Text),
        ConvertedType::DECIMAL => Some(ColumnType::Decimal),
        ConvertedType::DATE => Some(ColumnType::Date),
        ConvertedType::TIME_MILLIS | ConvertedType::TIME_MICROS => Some(ColumnType::Time),
        ConvertedType::TIMESTAMP_MILLIS | ConvertedType::TIMESTAMP_MICROS => {
            Some(ColumnType::DateTime)
        }
        _ => None,
    }
}

fn physical_column_type(physical: PhysicalType) -> ColumnType {
    match physical {
        PhysicalType::BOOLEAN => ColumnType::Boolean,
        PhysicalType::INT32 | PhysicalType::INT64 => ColumnType::Integer,
        // INT96 is only used for legacy timestamps.
        PhysicalType::INT96 => ColumnType::DateTime,
        PhysicalType::FLOAT | PhysicalType::DOUBLE => ColumnType::Float,
        PhysicalType::BYTE_ARRAY | PhysicalType::FIXED_LEN_BYTE_ARRAY => ColumnType::Binary,
    }
}

/// Bounds only for types whose stored order matches their meaning.
fn bounds(
    statistics: &[&Statistics],
    column_type: ColumnType,
    column: &ColumnDescriptor,
) -> Option<Bounds> {
    match (column_type, statistics.first()?) {
        // Unsigned values are stored in signed integers, so their order is wrong.
        (ColumnType::Integer, _) if is_unsigned(column) => None,
        (ColumnType::Integer, Statistics::Int32(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                Statistics::Int32(values) => Some(values),
                _ => None,
            },
            i32::to_string,
        ),
        (ColumnType::Integer, Statistics::Int64(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                Statistics::Int64(values) => Some(values),
                _ => None,
            },
            i64::to_string,
        ),
        (ColumnType::Float, Statistics::Float(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                Statistics::Float(values) => Some(values),
                _ => None,
            },
            f32::to_string,
        ),
        (ColumnType::Float, Statistics::Double(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                Statistics::Double(values) => Some(values),
                _ => None,
            },
            f64::to_string,
        ),
        (ColumnType::Boolean, Statistics::Boolean(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                Statistics::Boolean(values) => Some(values),
                _ => None,
            },
            bool::to_string,
        ),
        (ColumnType::Text, Statistics::ByteArray(_)) => merge_bounds(
            statistics,
            |stats| match stats {
                // Long strings may be cut in the statistics; show only exact ones.
                Statistics::ByteArray(values) if values.min_is_exact() && values.max_is_exact() => {
                    Some(values)
                }
                _ => None,
            },
            |text| String::from_utf8_lossy(text.data()).into_owned(),
        ),
        _ => None,
    }
}

fn is_unsigned(column: &ColumnDescriptor) -> bool {
    let logical_unsigned = matches!(
        column.logical_type_ref(),
        Some(LogicalType::Integer(int)) if !int.is_signed
    );
    let converted_unsigned = matches!(
        column.converted_type(),
        ConvertedType::UINT_8
            | ConvertedType::UINT_16
            | ConvertedType::UINT_32
            | ConvertedType::UINT_64
    );
    logical_unsigned || converted_unsigned
}

/// Smallest min and largest max over all row groups. Row groups with only nulls
/// have no min and max and are skipped.
fn merge_bounds<'a, T: PartialOrd + 'a>(
    statistics: &[&'a Statistics],
    values: impl Fn(&'a Statistics) -> Option<&'a ValueStatistics<T>>,
    to_text: impl Fn(&T) -> String,
) -> Option<Bounds> {
    let mut min: Option<&T> = None;
    let mut max: Option<&T> = None;
    for stats in statistics {
        let values = values(stats)?;
        let (Some(low), Some(high)) = (values.min_opt(), values.max_opt()) else {
            continue;
        };
        if min.is_none_or(|current| low < current) {
            min = Some(low);
        }
        if max.is_none_or(|current| high > current) {
            max = Some(high);
        }
    }
    Some(Bounds {
        min: to_text(min?),
        max: to_text(max?),
    })
}

fn read_rows(reader: &SerializedFileReader<File>, count: usize) -> Result<Vec<Row>, ReadError> {
    let rows = reader
        .get_row_iter(None)?
        .take(count)
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Runs `job` for each of `0..jobs` on several threads; results come back in job order.
///
/// Opening a row group or a column chunk decompresses pages, which is most of the time
/// spent. Each thread opens the file itself: clones of one `File` share a cursor.
fn in_parallel<T: Send>(
    path: &Path,
    jobs: usize,
    job: impl Fn(&SerializedFileReader<File>, usize) -> Result<T, ReadError> + Sync,
) -> Result<Vec<T>, ReadError> {
    let workers = thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(jobs);
    let job = &job;
    let results: Vec<Result<Vec<(usize, T)>, ReadError>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                scope.spawn(move || {
                    let reader = SerializedFileReader::new(File::open(path)?)?;
                    (worker..jobs)
                        .step_by(workers)
                        .map(|index| Ok((index, job(&reader, index)?)))
                        .collect()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    let mut slots: Vec<Option<T>> = (0..jobs).map(|_| None).collect();
    for result in results {
        for (index, value) in result? {
            slots[index] = Some(value);
        }
    }
    Ok(slots.into_iter().flatten().collect())
}

/// Up to `count` rows taken from row groups spread over the file, the same number from
/// each, so the sample is not only the start of the file. Rows come back in row group
/// order, as if read one by one.
fn read_sample(path: &Path, groups: usize, count: usize) -> Result<Vec<Row>, ReadError> {
    let used = groups.min(count);
    if used == 0 {
        return Ok(Vec::new());
    }
    let per_group = count.div_ceil(used);
    let groups_read = in_parallel(path, used, |reader, slot| {
        read_group(reader, slot * groups / used, per_group)
    })?;
    let mut rows: Vec<Row> = groups_read.into_iter().flatten().collect();
    rows.truncate(count);
    Ok(rows)
}

fn read_group(
    reader: &SerializedFileReader<File>,
    index: usize,
    count: usize,
) -> Result<Vec<Row>, ReadError> {
    let rows = reader
        .get_row_group(index)?
        .get_row_iter(None)?
        .take(count)
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Windows that the sample of flat columns reads. Many short windows find values that sit
/// together, for example nulls only in the last rows of a file.
const SAMPLE_WINDOWS: u64 = 100;

/// What the sample windows hold for one flat column.
#[derive(Default)]
struct ColumnSample {
    nulls: u64,
    rows: u64,
    /// Present values as text, as whole rows of the file would show them.
    values: Vec<String>,
}

/// Reads `count` rows of every flat column in windows from the first row of the file to
/// the last, and gives the columns their profiles and any null counts missing from the
/// metadata. Pages between the windows are skipped without being decompressed. Returns
/// the rows read per column, or `None` without flat columns.
fn sample_flat_columns(
    path: &Path,
    metadata: &ParquetMetaData,
    columns: &mut [ColumnSummary],
    count: usize,
) -> Result<Option<u64>, ReadError> {
    let schema = metadata.file_metadata().schema_descr();
    let flat: Vec<(usize, usize)> = top_level_leaves(schema)
        .into_iter()
        .enumerate()
        .filter_map(|(index, leaf)| Some((index, leaf?)))
        .filter(|&(_, leaf)| schema.column(leaf).max_rep_level() == 0)
        .collect();
    if flat.is_empty() {
        return Ok(None);
    }
    let total_rows = row_count(metadata)?;
    let ranges = ranges_per_row_group(&sample_windows(total_rows, count as u64), metadata);
    let jobs: Vec<(usize, usize)> = (0..flat.len())
        .flat_map(|column| (0..ranges.len()).map(move |row_group| (column, row_group)))
        .filter(|&(_, row_group)| !ranges[row_group].is_empty())
        .collect();
    let parts = in_parallel(path, jobs.len(), |reader, job| {
        let (column, row_group) = jobs[job];
        sample_column(reader, flat[column].1, row_group, &ranges[row_group])
    })?;
    let mut samples: Vec<ColumnSample> = flat.iter().map(|_| ColumnSample::default()).collect();
    for (&(column, _), part) in jobs.iter().zip(parts) {
        let sample = &mut samples[column];
        sample.nulls += part.nulls;
        sample.rows += part.rows;
        sample.values.extend(part.values);
    }

    let rows = samples[0].rows;
    for (&(index, _), sample) in flat.iter().zip(samples) {
        let basis = basis_of(sample.rows, total_rows);
        let column = &mut columns[index];
        column.nulls.get_or_insert(NullCount {
            count: sample.nulls,
            of_rows: sample.rows,
            basis,
        });
        let values: Vec<&str> = sample.values.iter().map(String::as_str).collect();
        column.profile = Some(profile_of(&values, column.column_type, basis));
    }
    Ok(Some(rows))
}

/// `count` rows of the file in evenly spread windows; the first window starts at the
/// first row and the last one ends at the last row.
fn sample_windows(total_rows: u64, count: u64) -> Vec<Range<u64>> {
    if count >= total_rows {
        return std::iter::once(0..total_rows).collect();
    }
    let windows = SAMPLE_WINDOWS.min(count);
    let gaps = total_rows - count;
    let mut rows_before = 0;
    (0..windows)
        .map(|window| {
            let length = count / windows + u64::from(window < count % windows);
            let gap = if windows > 1 {
                window * gaps / (windows - 1)
            } else {
                0
            };
            let start = rows_before + gap;
            rows_before += length;
            start..start + length
        })
        .collect()
}

/// The windows cut at row group borders, as row ranges inside each row group.
fn ranges_per_row_group(
    windows: &[Range<u64>],
    metadata: &ParquetMetaData,
) -> Vec<Vec<Range<u64>>> {
    let mut first_row = 0;
    metadata
        .row_groups()
        .iter()
        .map(|row_group| {
            let end = first_row + u64::try_from(row_group.num_rows()).unwrap_or(0);
            let inside = windows
                .iter()
                .filter(|window| window.start < end && window.end > first_row)
                .map(|window| {
                    window.start.max(first_row) - first_row..window.end.min(end) - first_row
                })
                .collect();
            first_row = end;
            inside
        })
        .collect()
}

/// One flat column in the given row ranges of one row group.
fn sample_column(
    reader: &SerializedFileReader<File>,
    leaf: usize,
    row_group: usize,
    ranges: &[Range<u64>],
) -> Result<ColumnSample, ReadError> {
    let descriptor = reader
        .metadata()
        .file_metadata()
        .schema_descr()
        .column(leaf);
    let pages = reader
        .get_row_group(row_group)?
        .get_column_page_reader(leaf)?;
    let descriptor = &descriptor;
    let sample = match get_column_reader(descriptor.clone(), pages) {
        ColumnReader::BoolColumnReader(column) => sample_in(column, ranges, descriptor, |d, v| {
            Ok(Field::convert_bool(d, v))
        }),
        ColumnReader::Int32ColumnReader(column) => sample_in(column, ranges, descriptor, |d, v| {
            Ok(Field::convert_int32(d, v))
        }),
        ColumnReader::Int64ColumnReader(column) => sample_in(column, ranges, descriptor, |d, v| {
            Ok(Field::convert_int64(d, v))
        }),
        ColumnReader::Int96ColumnReader(column) => sample_in(column, ranges, descriptor, |d, v| {
            Ok(Field::convert_int96(d, v))
        }),
        ColumnReader::FloatColumnReader(column) => sample_in(column, ranges, descriptor, |d, v| {
            Ok(Field::convert_float(d, v))
        }),
        ColumnReader::DoubleColumnReader(column) => {
            sample_in(column, ranges, descriptor, |d, v| {
                Ok(Field::convert_double(d, v))
            })
        }
        ColumnReader::ByteArrayColumnReader(column) => {
            sample_in(column, ranges, descriptor, Field::convert_byte_array)
        }
        ColumnReader::FixedLenByteArrayColumnReader(column) => {
            sample_in(column, ranges, descriptor, |d, v| {
                Field::convert_byte_array(d, v.into())
            })
        }
    }?;
    Ok(sample)
}

/// Reads the ranges of one column chunk. A value is null when its definition level is
/// below the column's maximum; present values are turned into the same `Field` that a
/// whole row would hold, so their text does not depend on how they were read.
fn sample_in<T: DataType>(
    mut column: ColumnReaderImpl<T>,
    ranges: &[Range<u64>],
    descriptor: &ColumnDescPtr,
    to_field: impl Fn(&ColumnDescPtr, T::T) -> Result<Field, ParquetError>,
) -> Result<ColumnSample, ParquetError> {
    let max_definition = descriptor.max_def_level();
    let mut sample = ColumnSample::default();
    let mut position = 0;
    let (mut levels, mut values) = (Vec::new(), Vec::new());
    for range in ranges {
        column.skip_records((range.start - position) as usize)?;
        levels.clear();
        values.clear();
        let (read, _, _) = column.read_records(
            (range.end - range.start) as usize,
            Some(&mut levels),
            None,
            &mut values,
        )?;
        sample.nulls += levels
            .iter()
            .filter(|&&level| level < max_definition)
            .count() as u64;
        sample.rows += read as u64;
        for value in values.drain(..) {
            sample
                .values
                .extend(field_text(&to_field(descriptor, value)?));
        }
        position = range.end;
    }
    Ok(sample)
}

/// Profiles the columns that the flat sample did not cover, from whole rows.
fn add_profiles(columns: &mut [ColumnSummary], rows: &[Row], total_rows: u64) {
    let basis = basis_of(rows.len() as u64, total_rows);
    let texts: Vec<Vec<Option<String>>> = rows.iter().map(row_text).collect();
    for (index, column) in columns.iter_mut().enumerate() {
        if column.profile.is_some() {
            continue;
        }
        let values: Vec<&str> = texts
            .iter()
            .filter_map(|row| row.get(index)?.as_deref())
            .collect();
        column.profile = Some(profile_of(&values, column.column_type, basis));
    }
}

fn profile_of(values: &[&str], column_type: ColumnType, basis: Basis) -> ColumnProfile {
    profile(values, column_type, basis, &|value| value.parse().ok())
}

fn basis_of(sampled: u64, total_rows: u64) -> Basis {
    if sampled == total_rows {
        Basis::WholeFile
    } else {
        Basis::Sample
    }
}

/// Counts nulls in the rows read for columns still without a null count: nested and
/// repeated ones.
fn fill_missing_nulls(columns: &mut [ColumnSummary], rows: &[Row], total_rows: u64) {
    let sampled = rows.len() as u64;
    let basis = basis_of(sampled, total_rows);
    for (index, column) in columns.iter_mut().enumerate() {
        if column.nulls.is_some() {
            continue;
        }
        let count = rows
            .iter()
            .filter(|row| matches!(row.get_column_iter().nth(index), Some((_, Field::Null))))
            .count() as u64;
        column.nulls = Some(NullCount {
            count,
            of_rows: sampled,
            basis,
        });
    }
}

fn row_text(row: &Row) -> Vec<Option<String>> {
    row.get_column_iter()
        .map(|(_, field)| field_text(field))
        .collect()
}

fn field_text(field: &Field) -> Option<String> {
    match field {
        Field::Null => None,
        Field::Str(text) => Some(text.clone()),
        // The library writes floats in scientific notation outside a fixed range.
        Field::Float(value) => Some(value.to_string()),
        Field::Double(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}
