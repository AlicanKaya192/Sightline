//! Parquet: schema, row count and statistics come from the footer metadata.
//! Rows are decoded only for the head and for columns without null counts.

use std::fs::File;
use std::path::Path;

use ::parquet::basic::{ConvertedType, LogicalType, Type as PhysicalType};
use ::parquet::errors::ParquetError;
use ::parquet::file::metadata::ParquetMetaData;
use ::parquet::file::reader::{FileReader, SerializedFileReader};
use ::parquet::file::statistics::{Statistics, ValueStatistics};
use ::parquet::record::{Field, Row};
use ::parquet::schema::types::ColumnDescriptor;

use super::{Limits, ReadError, Reader, has_extension};
use crate::summary::{
    Basis, Bounds, ColumnSummary, ColumnType, NullCount, RowCount, Summary, TableFormat,
    TableSummary,
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

        let needs_sample = columns.iter().any(|column| column.nulls.is_none());
        let rows_to_read = if needs_sample {
            limits.sample_rows.max(limits.head_rows)
        } else {
            limits.head_rows
        };
        let rows = read_rows(&reader, rows_to_read)?;
        let sample = &rows[..rows.len().min(limits.sample_rows)];
        fill_missing_nulls(&mut columns, sample, total_rows);

        Ok(Summary::Table(TableSummary {
            format: TableFormat::Parquet {
                row_groups: metadata.num_row_groups(),
            },
            rows: RowCount::Exact(total_rows),
            columns,
            head: rows.iter().take(limits.head_rows).map(row_text).collect(),
        }))
    }
}

fn row_count(metadata: &ParquetMetaData) -> Result<u64, ReadError> {
    let rows = metadata.file_metadata().num_rows();
    u64::try_from(rows)
        .map_err(|_| ParquetError::General(format!("negative row count: {rows}")).into())
}

/// One summary per top-level field. Nested fields get no statistics.
fn columns_from_metadata(metadata: &ParquetMetaData, total_rows: u64) -> Vec<ColumnSummary> {
    let schema = metadata.file_metadata().schema_descr();
    let mut leaf_of_field = vec![None; schema.root_schema().get_fields().len()];
    for leaf in 0..schema.num_columns() {
        let field = schema.get_column_root_idx(leaf);
        if schema.get_column_root(leaf).is_primitive() {
            leaf_of_field[field] = Some(leaf);
        }
    }

    schema
        .root_schema()
        .get_fields()
        .iter()
        .zip(leaf_of_field)
        .map(|(field, leaf)| match leaf {
            Some(leaf) => primitive_column(metadata, leaf, total_rows),
            None => ColumnSummary {
                name: field.name().to_string(),
                column_type: ColumnType::Nested,
                nulls: None,
                bounds: None,
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

/// Counts nulls in the rows read for columns whose metadata had no null count.
fn fill_missing_nulls(columns: &mut [ColumnSummary], rows: &[Row], total_rows: u64) {
    let sampled = rows.len() as u64;
    let basis = if sampled == total_rows {
        Basis::WholeFile
    } else {
        Basis::Sample
    };
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
