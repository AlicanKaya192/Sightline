//! NumPy `.npy`: a magic string, a header dictionary, then the raw values.
//! The header is parsed as plain data, never evaluated. Object arrays hold pickles and
//! are not read. Large arrays are sampled from windows spread over the whole file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::py_literal::{self, Literal};
use super::{Limits, ReadError, Reader, has_extension};
use crate::profile::histogram;
use crate::summary::{ArrayStats, ArraySummary, Basis, DType, FileWarning, SizeCheck, Summary};

const FORMAT: &str = "npy";
const MAGIC: &[u8] = b"\x93NUMPY";
/// Far above any real header; bounds the memory a crafted file can ask for.
const MAX_HEADER_BYTES: u64 = 100_000;
const SAMPLE_WINDOWS: u64 = 100;
/// Bytes per character of a NumPy `U` (UTF-32) string.
const UNICODE_CHAR_BYTES: u64 = 4;

pub struct NpyReader;

impl Reader for NpyReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "npy")
    }

    fn summarize(&self, path: &Path, limits: &Limits) -> Result<Summary, ReadError> {
        let mut file = File::open(path)?;
        let file_bytes = file.metadata()?.len();
        let (header, header_end) = read_header(&mut file, file_bytes)?;
        let fields = header_fields(&header)?;
        let element = Element::from_descr(&fields.descr);
        let elements = fields
            .shape
            .iter()
            .try_fold(1u64, |count, &size| count.checked_mul(size));
        let size_check = size_check(&element, elements, header_end, file_bytes);
        let data = DataRange {
            start: header_end,
            values: available_values(&element, elements, header_end, file_bytes),
        };
        let stats = match element.number {
            Some(number) => Some(statistics(&mut file, number, &data, limits)?),
            None => None,
        };
        Ok(Summary::Array(ArraySummary {
            descr: descr_text(&fields.descr),
            dtype: element.dtype.clone(),
            shape: fields.shape,
            fortran_order: fields.fortran_order,
            size_check,
            stats,
            warnings: element.warning().into_iter().collect(),
        }))
    }
}

fn malformed(reason: impl Into<String>) -> ReadError {
    ReadError::Malformed {
        format: FORMAT,
        reason: reason.into(),
    }
}

/// The header text and the offset where the values start.
fn read_header(file: &mut File, file_bytes: u64) -> Result<(String, u64), ReadError> {
    let mut start = [0; 8];
    if file_bytes < start.len() as u64 {
        return Err(malformed("the file is shorter than the npy magic string"));
    }
    file.read_exact(&mut start)?;
    if &start[..MAGIC.len()] != MAGIC {
        return Err(malformed(
            "the file does not start with the npy magic string",
        ));
    }
    let version = start[MAGIC.len()];
    let length = match version {
        1 => {
            let mut field = [0; 2];
            file.read_exact(&mut field)?;
            u64::from(u16::from_le_bytes(field))
        }
        2 | 3 => {
            let mut field = [0; 4];
            file.read_exact(&mut field)?;
            u64::from(u32::from_le_bytes(field))
        }
        other => return Err(malformed(format!("format version {other} is not known"))),
    };
    let prefix = file.stream_position()?;
    if length > MAX_HEADER_BYTES {
        return Err(malformed(format!(
            "the header claims {length} bytes, over the limit of {MAX_HEADER_BYTES}"
        )));
    }
    if prefix + length > file_bytes {
        return Err(malformed(format!(
            "the header claims {length} bytes, beyond the end of the file"
        )));
    }
    let mut bytes = vec![0; length as usize];
    file.read_exact(&mut bytes)?;
    let text = if version == 3 {
        String::from_utf8(bytes).map_err(|_| malformed("the header is not UTF-8"))?
    } else {
        // Versions 1 and 2 are Latin-1, where every byte is the character of that number.
        bytes.into_iter().map(char::from).collect()
    };
    Ok((text, prefix + length))
}

struct HeaderFields {
    descr: Literal,
    fortran_order: bool,
    shape: Vec<u64>,
}

fn header_fields(header: &str) -> Result<HeaderFields, ReadError> {
    let dict = py_literal::parse(header).map_err(malformed)?;
    let missing = |key: &str| malformed(format!("the header has no valid {key:?}"));
    let descr = dict.get("descr").cloned().ok_or_else(|| missing("descr"))?;
    let Some(&Literal::Bool(fortran_order)) = dict.get("fortran_order") else {
        return Err(missing("fortran_order"));
    };
    let Some(Literal::Tuple(sizes)) = dict.get("shape") else {
        return Err(missing("shape"));
    };
    let shape = sizes
        .iter()
        .map(|size| match size {
            Literal::Int(size) => Some(*size),
            _ => None,
        })
        .collect::<Option<Vec<u64>>>()
        .ok_or_else(|| missing("shape"))?;
    Ok(HeaderFields {
        descr,
        fortran_order,
        shape,
    })
}

fn descr_text(descr: &Literal) -> String {
    match descr {
        Literal::Text(text) => text.clone(),
        structured => structured.to_string(),
    }
}

/// How one value of the array is stored.
struct Element {
    dtype: DType,
    /// `None` when unknown, or for objects, whose pickled size says nothing.
    bytes: Option<u64>,
    /// How to read a value as a number; `None` for types without statistics.
    number: Option<NumberKind>,
    is_object: bool,
}

#[derive(Debug, Clone, Copy)]
struct NumberKind {
    encoding: Encoding,
    bytes: u64,
    big_endian: bool,
}

#[derive(Debug, Clone, Copy)]
enum Encoding {
    Unsigned,
    Signed,
    Float,
}

impl Element {
    fn from_descr(descr: &Literal) -> Self {
        match descr {
            Literal::Text(text) => Self::from_type_string(text),
            _ => Self::other(descr_text(descr), None),
        }
    }

    /// A string such as `<f4`, `|b1`, `>i8`, `<U10` or `|O`.
    fn from_type_string(descr: &str) -> Self {
        let (big_endian, code) = match descr.chars().next() {
            Some('>') => (true, &descr[1..]),
            Some('<' | '|' | '=') => (false, &descr[1..]),
            _ => (false, descr),
        };
        let kind = code.chars().next().unwrap_or(' ');
        let size: Option<u64> = code.get(1..).and_then(|digits| digits.parse().ok());
        let numeric =
            |encoding, dtype| Self::number(dtype, encoding, size.unwrap_or(0), big_endian);
        match (kind, size) {
            ('b', Some(1)) => numeric(Encoding::Unsigned, DType::Bool),
            ('u', Some(1)) => numeric(Encoding::Unsigned, DType::U8),
            ('u', Some(2)) => numeric(Encoding::Unsigned, DType::U16),
            ('u', Some(4)) => numeric(Encoding::Unsigned, DType::U32),
            ('u', Some(8)) => numeric(Encoding::Unsigned, DType::U64),
            ('i', Some(1)) => numeric(Encoding::Signed, DType::I8),
            ('i', Some(2)) => numeric(Encoding::Signed, DType::I16),
            ('i', Some(4)) => numeric(Encoding::Signed, DType::I32),
            ('i', Some(8)) => numeric(Encoding::Signed, DType::I64),
            ('f', Some(2)) => numeric(Encoding::Float, DType::F16),
            ('f', Some(4)) => numeric(Encoding::Float, DType::F32),
            ('f', Some(8)) => numeric(Encoding::Float, DType::F64),
            ('c', Some(8)) => Self::other_known(DType::Complex64, 8),
            ('c', Some(16)) => Self::other_known(DType::Complex128, 16),
            ('U', Some(chars)) => {
                Self::other_known(DType::Text, chars.saturating_mul(UNICODE_CHAR_BYTES))
            }
            ('S' | 'a', Some(bytes)) => Self::other_known(DType::Text, bytes),
            ('O', _) => Self {
                dtype: DType::Other(descr.to_string()),
                bytes: None,
                number: None,
                is_object: true,
            },
            ('M' | 'm', _) => Self::other(descr.to_string(), Some(8)),
            ('V', bytes) => Self::other(descr.to_string(), bytes),
            _ => Self::other(descr.to_string(), None),
        }
    }

    fn number(dtype: DType, encoding: Encoding, bytes: u64, big_endian: bool) -> Self {
        Self {
            dtype,
            bytes: Some(bytes),
            number: Some(NumberKind {
                encoding,
                bytes,
                big_endian,
            }),
            is_object: false,
        }
    }

    fn other_known(dtype: DType, bytes: u64) -> Self {
        Self {
            dtype,
            bytes: Some(bytes),
            number: None,
            is_object: false,
        }
    }

    fn other(descr: String, bytes: Option<u64>) -> Self {
        Self {
            dtype: DType::Other(descr),
            bytes,
            number: None,
            is_object: false,
        }
    }

    fn warning(&self) -> Option<FileWarning> {
        if self.is_object {
            Some(FileWarning::PickleInside)
        } else if self.number.is_none() {
            Some(FileWarning::NoStatistics)
        } else {
            None
        }
    }
}

fn size_check(
    element: &Element,
    elements: Option<u64>,
    header_end: u64,
    file_bytes: u64,
) -> SizeCheck {
    let Some(data_bytes) = elements
        .zip(element.bytes)
        .and_then(|(count, bytes)| count.checked_mul(bytes))
    else {
        return SizeCheck::Unknown;
    };
    match header_end.checked_add(data_bytes) {
        Some(expected) if expected == file_bytes => SizeCheck::Consistent,
        Some(expected) => SizeCheck::Mismatch {
            expected,
            actual: file_bytes,
        },
        None => SizeCheck::Unknown,
    }
}

/// Values in the file, which a cut file may have fewer of than its shape says.
fn available_values(
    element: &Element,
    elements: Option<u64>,
    header_end: u64,
    file_bytes: u64,
) -> u64 {
    match element.bytes {
        Some(bytes) if bytes > 0 => {
            let present = (file_bytes - header_end) / bytes;
            elements.map_or(present, |count| count.min(present))
        }
        _ => 0,
    }
}

struct DataRange {
    start: u64,
    values: u64,
}

fn statistics(
    file: &mut File,
    number: NumberKind,
    data: &DataRange,
    limits: &Limits,
) -> Result<ArrayStats, ReadError> {
    let budget = (limits.sample_bytes as u64 / number.bytes).max(1);
    let (basis, windows) = if data.values <= budget {
        (Basis::WholeFile, vec![(0, data.values)])
    } else {
        (Basis::Sample, spread_windows(data.values, budget))
    };
    let mut values = Vec::new();
    for (first, count) in windows {
        file.seek(SeekFrom::Start(data.start + first * number.bytes))?;
        let mut bytes = vec![0; (count * number.bytes) as usize];
        file.read_exact(&mut bytes)?;
        values.extend(
            bytes
                .chunks_exact(number.bytes as usize)
                .map(|v| decode(v, number)),
        );
    }
    Ok(summarize_values(&values, basis))
}

/// Equal windows from the first value to the last, `budget` values in all.
fn spread_windows(total: u64, budget: u64) -> Vec<(u64, u64)> {
    let length = (budget / SAMPLE_WINDOWS).max(1);
    let last_start = total - length;
    (0..SAMPLE_WINDOWS)
        .map(|window| (last_start * window / (SAMPLE_WINDOWS - 1), length))
        .collect()
}

fn decode(bytes: &[u8], number: NumberKind) -> f64 {
    let mut raw = [0u8; 8];
    raw[..bytes.len()].copy_from_slice(bytes);
    if number.big_endian {
        raw[..bytes.len()].reverse();
    }
    let bits = u64::from_le_bytes(raw);
    let width = bytes.len() as u32 * 8;
    match number.encoding {
        Encoding::Unsigned => bits as f64,
        Encoding::Signed => {
            let shift = 64 - width;
            ((bits << shift) as i64 >> shift) as f64
        }
        Encoding::Float => match width {
            16 => half_to_f64(bits as u16),
            32 => f64::from(f32::from_bits(bits as u32)),
            _ => f64::from_bits(bits),
        },
    }
}

/// IEEE 754 half precision: 1 sign bit, 5 exponent bits (bias 15), 10 fraction bits.
fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f64::from(bits & 0x03FF);
    match exponent {
        0 => sign * fraction * 2f64.powi(-24),
        0x1F if fraction == 0.0 => sign * f64::INFINITY,
        0x1F => f64::NAN,
        _ => sign * (1.0 + fraction / 1024.0) * 2f64.powi(exponent - 15),
    }
}

fn summarize_values(values: &[f64], basis: Basis) -> ArrayStats {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let nan = values.iter().filter(|v| v.is_nan()).count() as u64;
    let min = finite.iter().copied().reduce(f64::min);
    let max = finite.iter().copied().reduce(f64::max);
    ArrayStats {
        values: values.len() as u64,
        basis,
        min,
        max,
        nan,
        infinite: values.len() as u64 - finite.len() as u64 - nan,
        histogram: if finite.is_empty() {
            Vec::new()
        } else {
            histogram(&finite)
        },
    }
}
