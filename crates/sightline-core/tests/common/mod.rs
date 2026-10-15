//! Test fixtures. Files go to Cargo's per-target temporary folder and are overwritten on each run.

#![allow(dead_code)] // each test binary uses a different part of this module

use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

use parquet::data_type::{BoolType, ByteArray, ByteArrayType, DoubleType, Int32Type, Int64Type};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;
use sightline_core::{Summary, TableSummary};

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

pub fn write_bytes(name: &str, content: &[u8]) -> PathBuf {
    let path = fixture_path(name);
    fs::write(&path, content).expect("write fixture");
    path
}

pub fn table(summary: Summary) -> TableSummary {
    let Summary::Table(table) = summary;
    table
}

/// Values of one optional Parquet column inside one row group.
pub enum Values {
    Int64(Vec<Option<i64>>),
    Int32(Vec<Option<i32>>),
    Double(Vec<Option<f64>>),
    Bool(Vec<Option<bool>>),
    Text(Vec<Option<&'static str>>),
}

/// Writes a Parquet file whose columns are all `optional`; one inner Vec per row group.
pub fn write_parquet(
    name: &str,
    schema: &str,
    row_groups: &[Vec<Values>],
    properties: WriterProperties,
) -> PathBuf {
    let path = fixture_path(name);
    let schema = Arc::new(parse_message_type(schema).expect("valid schema"));
    let file = File::create(&path).expect("create fixture");
    let mut writer =
        SerializedFileWriter::new(file, schema, Arc::new(properties)).expect("parquet writer");
    for columns in row_groups {
        let mut row_group = writer.next_row_group().expect("row group");
        for values in columns {
            let mut column = row_group
                .next_column()
                .expect("column")
                .expect("schema has this column");
            write_column(&mut column, values);
            column.close().expect("close column");
        }
        row_group.close().expect("close row group");
    }
    writer.close().expect("close file");
    path
}

fn write_column(column: &mut parquet::file::writer::SerializedColumnWriter<'_>, values: &Values) {
    match values {
        Values::Int64(v) => {
            let (present, levels) = split(v);
            column
                .typed::<Int64Type>()
                .write_batch(&present, Some(&levels), None)
        }
        Values::Int32(v) => {
            let (present, levels) = split(v);
            column
                .typed::<Int32Type>()
                .write_batch(&present, Some(&levels), None)
        }
        Values::Double(v) => {
            let (present, levels) = split(v);
            column
                .typed::<DoubleType>()
                .write_batch(&present, Some(&levels), None)
        }
        Values::Bool(v) => {
            let (present, levels) = split(v);
            column
                .typed::<BoolType>()
                .write_batch(&present, Some(&levels), None)
        }
        Values::Text(v) => {
            let (present, levels) = split(v);
            let present: Vec<ByteArray> = present.into_iter().map(ByteArray::from).collect();
            column
                .typed::<ByteArrayType>()
                .write_batch(&present, Some(&levels), None)
        }
    }
    .expect("write values");
}

/// Present values and the definition level of every row (1 = present, 0 = null).
fn split<T: Clone>(values: &[Option<T>]) -> (Vec<T>, Vec<i16>) {
    let present = values.iter().flatten().cloned().collect();
    let levels = values.iter().map(|v| i16::from(v.is_some())).collect();
    (present, levels)
}
