//! Test fixtures. Files go to Cargo's per-target temporary folder and are overwritten on each run.

#![allow(dead_code)] // each test binary uses a different part of this module

pub mod protobuf;
pub mod xlsx;

use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

use parquet::data_type::{BoolType, ByteArray, ByteArrayType, DoubleType, Int32Type, Int64Type};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;
use sightline_core::{ArraySummary, ModelSummary, PickleSummary, Summary, TableSummary};

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

pub fn write_bytes(name: &str, content: &[u8]) -> PathBuf {
    let path = fixture_path(name);
    fs::write(&path, content).expect("write fixture");
    path
}

pub fn table(summary: Summary) -> TableSummary {
    match summary {
        Summary::Table(table) => table,
        other => panic!("expected a table summary, got {other:?}"),
    }
}

pub fn model(summary: Summary) -> ModelSummary {
    match summary {
        Summary::Model(model) => model,
        other => panic!("expected a model summary, got {other:?}"),
    }
}

pub fn array(summary: Summary) -> ArraySummary {
    match summary {
        Summary::Array(array) => array,
        other => panic!("expected an array summary, got {other:?}"),
    }
}

pub fn pickle(summary: Summary) -> PickleSummary {
    match summary {
        Summary::Pickle(pickle) => pickle,
        other => panic!("expected a pickle summary, got {other:?}"),
    }
}

/// An `.npy` file of format `version` (1, 2 or 3) with the `header` dictionary text, padded
/// with spaces and a newline the way NumPy does, then `data`.
pub fn write_npy(name: &str, version: u8, header: &str, data: &[u8]) -> PathBuf {
    let length_bytes = if version == 1 { 2 } else { 4 };
    let prefix = 6 + 2 + length_bytes;
    let unpadded = prefix + header.len() + 1;
    let padding = (64 - unpadded % 64) % 64;
    let text = format!("{header}{}\n", " ".repeat(padding));
    let mut bytes = b"\x93NUMPY".to_vec();
    bytes.extend_from_slice(&[version, 0]);
    if version == 1 {
        bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
    } else {
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(data);
    write_bytes(name, &bytes)
}

/// A safetensors file: header length, `header` JSON, then `data_len` zero bytes of weights.
pub fn write_safetensors(name: &str, header: &str, data_len: usize) -> PathBuf {
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(header.as_bytes());
    bytes.resize(bytes.len() + data_len, 0);
    write_bytes(name, &bytes)
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
