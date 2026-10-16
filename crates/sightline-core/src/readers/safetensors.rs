//! safetensors: an 8-byte header length, a JSON header, then the weights. Only the header is read.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::{Map, Value};

use super::{Limits, ReadError, Reader, has_extension};
use crate::summary::{DType, ModelFormat, ModelSummary, SizeCheck, Summary, TensorInfo};

const FORMAT: &str = "safetensors";
const LENGTH_FIELD_BYTES: u64 = 8;
/// Bounds the memory a crafted header length can ask for.
const MAX_HEADER_BYTES: u64 = 100_000_000;
const METADATA_KEY: &str = "__metadata__";

pub struct SafetensorsReader;

impl Reader for SafetensorsReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "safetensors")
    }

    fn summarize(&self, path: &Path, _limits: &Limits) -> Result<Summary, ReadError> {
        let mut file = File::open(path)?;
        let file_bytes = file.metadata()?.len();
        let header = read_header(&mut file, file_bytes)?;
        let header_end = LENGTH_FIELD_BYTES + header.len() as u64;
        let entries = parse_header(&header)?;
        let mut stored = tensors(&entries)?;
        stored.sort_by_key(|tensor| tensor.offsets);
        let size_check = size_check(&stored, header_end, file_bytes);
        Ok(Summary::Model(ModelSummary {
            format: ModelFormat::Safetensors {
                metadata: metadata(&entries)?,
            },
            tensors: stored.into_iter().map(|tensor| tensor.info).collect(),
            size_check,
            warnings: Vec::new(),
        }))
    }
}

/// A tensor with the byte range it claims inside the data section.
struct StoredTensor {
    info: TensorInfo,
    offsets: (u64, u64),
}

fn malformed(reason: impl Into<String>) -> ReadError {
    ReadError::Malformed {
        format: FORMAT,
        reason: reason.into(),
    }
}

fn read_header(file: &mut File, file_bytes: u64) -> Result<Vec<u8>, ReadError> {
    if file_bytes < LENGTH_FIELD_BYTES {
        return Err(malformed(
            "the file is shorter than the header length field",
        ));
    }
    let mut length_field = [0; LENGTH_FIELD_BYTES as usize];
    file.read_exact(&mut length_field)?;
    let length = u64::from_le_bytes(length_field);
    if length > MAX_HEADER_BYTES {
        return Err(malformed(format!(
            "the header claims {length} bytes, over the limit of {MAX_HEADER_BYTES}"
        )));
    }
    if LENGTH_FIELD_BYTES + length > file_bytes {
        return Err(malformed(format!(
            "the header claims {length} bytes, beyond the end of the file"
        )));
    }
    let mut header = vec![0; length as usize];
    file.read_exact(&mut header)?;
    Ok(header)
}

fn parse_header(header: &[u8]) -> Result<Map<String, Value>, ReadError> {
    match serde_json::from_slice(header) {
        Ok(Value::Object(entries)) => Ok(entries),
        Ok(_) => Err(malformed("the header is not a JSON object")),
        Err(error) => Err(malformed(format!("the header is not valid JSON: {error}"))),
    }
}

fn metadata(entries: &Map<String, Value>) -> Result<Vec<(String, String)>, ReadError> {
    let Some(metadata) = entries.get(METADATA_KEY) else {
        return Ok(Vec::new());
    };
    let Value::Object(metadata) = metadata else {
        return Err(malformed("__metadata__ is not a JSON object"));
    };
    let mut pairs: Vec<(String, String)> = metadata
        .iter()
        .map(|(key, value)| match value {
            Value::String(text) => (key.clone(), text.clone()),
            other => (key.clone(), other.to_string()),
        })
        .collect();
    pairs.sort();
    Ok(pairs)
}

fn tensors(entries: &Map<String, Value>) -> Result<Vec<StoredTensor>, ReadError> {
    entries
        .iter()
        .filter(|(name, _)| name.as_str() != METADATA_KEY)
        .map(|(name, entry)| tensor(name, entry))
        .collect()
}

fn tensor(name: &str, entry: &Value) -> Result<StoredTensor, ReadError> {
    let invalid = |field: &str| malformed(format!("tensor {name:?} has no valid {field}"));
    let dtype = entry
        .get("dtype")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("dtype"))?;
    let shape = numbers(entry.get("shape")).ok_or_else(|| invalid("shape"))?;
    let offsets = match numbers(entry.get("data_offsets")).as_deref() {
        Some(&[begin, end]) => (begin, end),
        _ => return Err(invalid("data_offsets")),
    };
    Ok(StoredTensor {
        info: TensorInfo {
            name: name.to_string(),
            dtype: dtype_from_name(dtype),
            shape,
        },
        offsets,
    })
}

/// A JSON array of non-negative integers.
fn numbers(value: Option<&Value>) -> Option<Vec<u64>> {
    value?.as_array()?.iter().map(Value::as_u64).collect()
}

fn dtype_from_name(name: &str) -> DType {
    match name {
        "BOOL" => DType::Bool,
        "U8" => DType::U8,
        "I8" => DType::I8,
        "U16" => DType::U16,
        "I16" => DType::I16,
        "U32" => DType::U32,
        "I32" => DType::I32,
        "U64" => DType::U64,
        "I64" => DType::I64,
        "F16" => DType::F16,
        "BF16" => DType::BF16,
        "F32" => DType::F32,
        "F64" => DType::F64,
        "F8_E4M3" => DType::F8E4M3,
        "F8_E5M2" => DType::F8E5M2,
        other => DType::Other(other.to_string()),
    }
}

/// Tensors sorted by offset must fill the data section one after another, each with the
/// bytes its shape and type need.
fn size_check(stored: &[StoredTensor], header_end: u64, file_bytes: u64) -> SizeCheck {
    let Some(sizes) = stored
        .iter()
        .map(|tensor| tensor.info.bytes())
        .collect::<Option<Vec<u64>>>()
    else {
        return SizeCheck::Unknown;
    };
    let mut position = 0;
    for (tensor, size) in stored.iter().zip(&sizes) {
        let (begin, end) = tensor.offsets;
        if begin != position || end.checked_sub(begin) != Some(*size) {
            return SizeCheck::BadOffsets {
                tensor: tensor.info.name.clone(),
            };
        }
        position = end;
    }
    let expected = header_end.saturating_add(position);
    if expected == file_bytes {
        SizeCheck::Consistent
    } else {
        SizeCheck::Mismatch {
            expected,
            actual: file_bytes,
        }
    }
}
