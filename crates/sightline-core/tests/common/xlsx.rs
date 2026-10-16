//! Builds small `.xlsx` files: a zip archive of the XML parts Excel writes.

use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;

const STORED: u16 = 0;
const DEFLATED: u16 = 8;

/// The parts of a workbook with `sheets` (name, `<sheetData>` content) in order.
pub fn workbook(
    sheets: &[(&str, &str)],
    shared_strings: &[&str],
    styles: Option<&str>,
) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let mut sheet_list = String::new();
    let mut relations = String::new();
    for (index, (name, data)) in sheets.iter().enumerate() {
        let number = index + 1;
        sheet_list += &format!(r#"<sheet name="{name}" sheetId="{number}" r:id="rId{number}"/>"#);
        relations += &format!(
            r#"<Relationship Id="rId{number}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{number}.xml"/>"#
        );
        parts.push((
            format!("xl/worksheets/sheet{number}.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{data}</sheetData></worksheet>"#
            ),
        ));
    }
    parts.push((
        "xl/workbook.xml".to_string(),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{sheet_list}</sheets></workbook>"#
        ),
    ));
    parts.push((
        "xl/_rels/workbook.xml.rels".to_string(),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relations}</Relationships>"#
        ),
    ));
    if !shared_strings.is_empty() {
        let items: String = shared_strings
            .iter()
            .map(|text| format!("<si><t>{text}</t></si>"))
            .collect();
        parts.push((
            "xl/sharedStrings.xml".to_string(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{items}</sst>"#
            ),
        ));
    }
    if let Some(styles) = styles {
        parts.push((
            "xl/styles.xml".to_string(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{styles}</styleSheet>"#
            ),
        ));
    }
    parts
}

/// A zip archive of `parts`, compressed with deflate or stored as they are.
pub fn zip(parts: &[(String, String)], deflate: bool) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, content) in parts {
        let offset = archive.len() as u32;
        let (method, data) = if deflate {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(content.as_bytes()).expect("compress");
            (DEFLATED, encoder.finish().expect("compress"))
        } else {
            (STORED, content.as_bytes().to_vec())
        };
        let crc = crc32(content.as_bytes());
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&0u16.to_le_bytes()); // flags
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&0u32.to_le_bytes()); // time and date
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(content.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes()); // extra length
        local.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(&local);
        archive.extend_from_slice(&data);

        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
        directory.extend_from_slice(&20u16.to_le_bytes()); // version needed
        directory.extend_from_slice(&0u16.to_le_bytes()); // flags
        directory.extend_from_slice(&method.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes()); // time and date
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(content.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = archive.len() as u32;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&[0; 4]); // disk numbers
    archive.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes()); // comment length
    archive
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
