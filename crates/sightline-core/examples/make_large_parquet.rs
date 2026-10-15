//! Writes a Parquet file of at least the given size for speed measurements.
//!
//! cargo run --release -p sightline-core --example make_large_parquet -- <path> [megabytes]

use std::fs::File;
use std::sync::Arc;

use parquet::basic::Compression;
use parquet::data_type::{BoolType, ByteArray, ByteArrayType, DoubleType, Int64Type};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::{SerializedColumnWriter, SerializedFileWriter};
use parquet::schema::parser::parse_message_type;

const SCHEMA: &str = "message measurement {
    required int64 id;
    required double value;
    required int64 amount;
    required binary category (STRING);
    required boolean flag;
    optional binary note (STRING);
}";
const ROWS_PER_GROUP: usize = 1_000_000;
const DEFAULT_MEGABYTES: usize = 1024;
const CATEGORIES: [&str; 8] = [
    "ankara", "istanbul", "izmir", "bursa", "antalya", "konya", "adana", "trabzon",
];
/// One note in ten is missing.
const NOTE_NULL_EVERY: u64 = 10;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: make_large_parquet <path> [megabytes]")?;
    let megabytes = match args.next() {
        Some(value) => value.parse()?,
        None => DEFAULT_MEGABYTES,
    };
    let target_bytes = megabytes * 1024 * 1024;

    let schema = Arc::new(parse_message_type(SCHEMA)?);
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = SerializedFileWriter::new(File::create(&path)?, schema, Arc::new(properties))?;
    let mut random = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut first_id = 0;
    let mut row_groups = 0;
    while writer.bytes_written() < target_bytes {
        write_row_group(&mut writer, &mut random, first_id)?;
        first_id += ROWS_PER_GROUP as i64;
        row_groups += 1;
    }
    let metadata = writer.close()?;
    println!(
        "{path}: {} rows, {row_groups} row groups",
        metadata.file_metadata().num_rows()
    );
    Ok(())
}

fn write_row_group(
    writer: &mut SerializedFileWriter<File>,
    random: &mut XorShift,
    first_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let ids: Vec<i64> = (first_id..first_id + ROWS_PER_GROUP as i64).collect();
    let values: Vec<f64> = (0..ROWS_PER_GROUP).map(|_| random.unit()).collect();
    let amounts: Vec<i64> = (0..ROWS_PER_GROUP)
        .map(|_| (random.next() % 1_000_000) as i64)
        .collect();
    let categories: Vec<ByteArray> = (0..ROWS_PER_GROUP)
        .map(|_| ByteArray::from(CATEGORIES[(random.next() % CATEGORIES.len() as u64) as usize]))
        .collect();
    let flags: Vec<bool> = (0..ROWS_PER_GROUP)
        .map(|_| random.next().is_multiple_of(2))
        .collect();
    let note_levels: Vec<i16> = (0..ROWS_PER_GROUP)
        .map(|_| i16::from(!random.next().is_multiple_of(NOTE_NULL_EVERY)))
        .collect();
    let notes: Vec<ByteArray> = note_levels
        .iter()
        .filter(|&&level| level == 1)
        .map(|_| ByteArray::from(format!("{:016x}", random.next()).as_str()))
        .collect();

    let mut row_group = writer.next_row_group()?;
    let mut column = next_column(&mut row_group)?;
    column.typed::<Int64Type>().write_batch(&ids, None, None)?;
    column.close()?;
    let mut column = next_column(&mut row_group)?;
    column
        .typed::<DoubleType>()
        .write_batch(&values, None, None)?;
    column.close()?;
    let mut column = next_column(&mut row_group)?;
    column
        .typed::<Int64Type>()
        .write_batch(&amounts, None, None)?;
    column.close()?;
    let mut column = next_column(&mut row_group)?;
    column
        .typed::<ByteArrayType>()
        .write_batch(&categories, None, None)?;
    column.close()?;
    let mut column = next_column(&mut row_group)?;
    column.typed::<BoolType>().write_batch(&flags, None, None)?;
    column.close()?;
    let mut column = next_column(&mut row_group)?;
    column
        .typed::<ByteArrayType>()
        .write_batch(&notes, Some(&note_levels), None)?;
    column.close()?;
    row_group.close()?;
    Ok(())
}

fn next_column<'a>(
    row_group: &'a mut parquet::file::writer::SerializedRowGroupWriter<'_, File>,
) -> Result<SerializedColumnWriter<'a>, Box<dyn std::error::Error>> {
    Ok(row_group.next_column()?.ok_or("schema has fewer columns")?)
}

/// Small fast pseudo-random generator; the data only needs to resist compression.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}
