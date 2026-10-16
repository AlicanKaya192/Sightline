//! Entries of a zip archive, as Excel writes them: stored or deflated, without ZIP64 or
//! encryption. Only the central directory and the entries asked for are read.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::DeflateDecoder;

use super::ReadError;

const END_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const LOCAL_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const END_RECORD_SIZE: usize = 22;
/// The end record may be followed by a comment of up to this many bytes.
const MAX_COMMENT: usize = 0xFFFF;
const DIRECTORY_ENTRY_SIZE: usize = 46;
const LOCAL_HEADER_SIZE: usize = 30;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
const ENCRYPTED_FLAG: u16 = 1;
/// A size or offset of all ones means the real value is in a ZIP64 record.
const ZIP64_MARKER: u32 = u32::MAX;

struct Entry {
    name: String,
    method: u16,
    flags: u16,
    compressed_size: u64,
    local_offset: u64,
}

pub struct Archive {
    file: File,
    entries: Vec<Entry>,
}

/// An entry's content, cut at the limit it was read with.
pub struct Part {
    pub bytes: Vec<u8>,
    /// False when the entry is longer than the limit.
    pub complete: bool,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self, ReadError> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        let tail_length = length.min((END_RECORD_SIZE + MAX_COMMENT) as u64);
        file.seek(SeekFrom::Start(length - tail_length))?;
        let mut tail = vec![0; tail_length as usize];
        file.read_exact(&mut tail)?;
        let end = tail
            .windows(END_SIGNATURE.len())
            .rposition(|window| window == END_SIGNATURE)
            .filter(|&at| at + END_RECORD_SIZE <= tail.len())
            .ok_or_else(|| malformed("not a zip archive"))?;
        let record = &tail[end..end + END_RECORD_SIZE];
        let count = u16_at(record, 10);
        let directory_size = u32_at(record, 12);
        let directory_offset = u32_at(record, 16);
        if count == u16::MAX || directory_offset == ZIP64_MARKER {
            return Err(malformed("ZIP64 archives are not supported"));
        }
        if u64::from(directory_offset) + u64::from(directory_size) > length {
            return Err(malformed("the central directory lies outside the file"));
        }
        file.seek(SeekFrom::Start(u64::from(directory_offset)))?;
        let mut directory = vec![0; directory_size as usize];
        file.read_exact(&mut directory)?;
        let entries = directory_entries(&directory, count)?;
        Ok(Self { file, entries })
    }

    /// At most `limit` bytes of the entry `name`; `None` when there is no such entry.
    pub fn read(&mut self, name: &str, limit: u64) -> Result<Option<Part>, ReadError> {
        let Some(entry) = self.entries.iter().find(|entry| entry.name == name) else {
            return Ok(None);
        };
        if entry.flags & ENCRYPTED_FLAG != 0 {
            return Err(malformed("the workbook is encrypted"));
        }
        let mut header = [0; LOCAL_HEADER_SIZE];
        self.file.seek(SeekFrom::Start(entry.local_offset))?;
        self.file.read_exact(&mut header)?;
        if header[..4] != LOCAL_SIGNATURE {
            return Err(malformed("an entry header is damaged"));
        }
        let skip = u64::from(u16_at(&header, 26)) + u64::from(u16_at(&header, 28));
        self.file.seek(SeekFrom::Current(skip as i64))?;
        let compressed = (&mut self.file).take(entry.compressed_size);
        // One byte over the limit tells whether the entry is longer.
        let mut bytes = Vec::new();
        match entry.method {
            STORED => compressed.take(limit + 1).read_to_end(&mut bytes)?,
            DEFLATED => DeflateDecoder::new(compressed)
                .take(limit + 1)
                .read_to_end(&mut bytes)?,
            other => {
                return Err(malformed(&format!(
                    "compression method {other} is not supported"
                )));
            }
        };
        let complete = bytes.len() as u64 <= limit;
        bytes.truncate(limit as usize);
        Ok(Some(Part { bytes, complete }))
    }
}

fn directory_entries(directory: &[u8], count: u16) -> Result<Vec<Entry>, ReadError> {
    let mut entries = Vec::with_capacity(usize::from(count));
    let mut at = 0;
    for _ in 0..count {
        let fixed = directory
            .get(at..at + DIRECTORY_ENTRY_SIZE)
            .filter(|fixed| fixed[..4] == DIRECTORY_SIGNATURE)
            .ok_or_else(|| malformed("the central directory is damaged"))?;
        let name_length = usize::from(u16_at(fixed, 28));
        let variable =
            name_length + usize::from(u16_at(fixed, 30)) + usize::from(u16_at(fixed, 32));
        let name_start = at + DIRECTORY_ENTRY_SIZE;
        let name = directory
            .get(name_start..name_start + name_length)
            .ok_or_else(|| malformed("the central directory is damaged"))?;
        let compressed_size = u32_at(fixed, 20);
        let local_offset = u32_at(fixed, 42);
        if compressed_size == ZIP64_MARKER || local_offset == ZIP64_MARKER {
            return Err(malformed("ZIP64 archives are not supported"));
        }
        entries.push(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            method: u16_at(fixed, 10),
            flags: u16_at(fixed, 8),
            compressed_size: u64::from(compressed_size),
            local_offset: u64::from(local_offset),
        });
        at = name_start + variable;
    }
    Ok(entries)
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn malformed(reason: &str) -> ReadError {
    ReadError::Malformed {
        format: "Excel",
        reason: reason.to_string(),
    }
}
