//! Excel workbooks (`.xlsx`, `.xlsm`): the first sheet is read as a table whose first
//! row is the header. Formulas are never calculated; the result Excel saved is shown.
//! Macros in `.xlsm` files are never read.

use std::collections::HashMap;
use std::path::Path;

use super::csv::{row_count, summarize_columns};
use super::xml::{Token, attribute, local_name, tokens, unescape};
use super::zip_archive::Archive;
use super::{Limits, ReadError, Reader, has_extension};
use crate::insights::insights;
use crate::summary::{Basis, Summary, TableFormat, TableSummary};

/// Most bytes one part may unpack to; larger sheets are counted only this far. Also keeps
/// a small file that unpacks to gigabytes from filling the memory.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
const WORKBOOK: &str = "xl/workbook.xml";
const WORKBOOK_RELATIONS: &str = "xl/_rels/workbook.xml.rels";
const SHARED_STRINGS: &str = "xl/sharedStrings.xml";
const STYLES: &str = "xl/styles.xml";
/// Day 0 of Excel's default date system, 1899-12-30, as days since 1970-01-01.
const EPOCH_1900: i64 = -25_569;
/// Day 0 of the 1904 date system of old Mac workbooks, 1904-01-01.
const EPOCH_1904: i64 = -24_107;
/// Excel counts the non-existent 1900-02-29 as day 60; earlier days are one off.
const FICTIONAL_LEAP_DAY: i64 = 60;
const SECONDS_PER_DAY: f64 = 86_400.0;

pub struct XlsxReader;

impl Reader for XlsxReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "xlsx") || has_extension(path, "xlsm")
    }

    fn summarize(&self, path: &Path, limits: &Limits) -> Result<Summary, ReadError> {
        let mut archive = Archive::open(path)?;
        let workbook = Workbook::parse(&required_text(&mut archive, WORKBOOK)?);
        let first = workbook
            .sheets
            .first()
            .ok_or_else(|| malformed("the workbook has no sheets"))?;
        let relations = relations(&required_text(&mut archive, WORKBOOK_RELATIONS)?);
        let target = relations
            .get(&first.relation)
            .ok_or_else(|| malformed("the first sheet is missing"))?;
        let shared = optional_text(&mut archive, SHARED_STRINGS)?
            .map(|text| shared_strings(&text))
            .unwrap_or_default();
        let styles = optional_text(&mut archive, STYLES)?
            .map(|text| date_styles(&text))
            .unwrap_or_default();
        let sheet = archive
            .read(&part_path(target), MAX_PART_BYTES)?
            .ok_or_else(|| malformed("the first sheet is missing"))?;
        let cells = Cells {
            shared: &shared,
            styles: &styles,
            epoch: workbook.epoch,
        };
        let kept = limits.sample_rows.max(limits.head_rows);
        let table = read_sheet(&String::from_utf8_lossy(&sheet.bytes), &cells, kept);

        let is_complete = sheet.complete;
        let sample = &table.rows[..table.rows.len().min(limits.sample_rows)];
        let basis = if is_complete && sample.len() == table.data_rows {
            Basis::WholeFile
        } else {
            Basis::Sample
        };
        let columns = summarize_columns(&table.header, sample, false, basis);
        Ok(Summary::Table(TableSummary {
            format: TableFormat::Excel {
                sheets: workbook
                    .sheets
                    .into_iter()
                    .map(|sheet| sheet.name)
                    .collect(),
            },
            rows: row_count(table.data_rows, is_complete),
            insights: insights(&columns),
            columns,
            head: table.rows.iter().take(limits.head_rows).cloned().collect(),
            sampled_rows: (basis == Basis::Sample).then_some(sample.len() as u64),
        }))
    }
}

fn required_text(archive: &mut Archive, name: &str) -> Result<String, ReadError> {
    optional_text(archive, name)?.ok_or_else(|| malformed(&format!("{name} is missing")))
}

fn optional_text(archive: &mut Archive, name: &str) -> Result<Option<String>, ReadError> {
    let Some(part) = archive.read(name, MAX_PART_BYTES)? else {
        return Ok(None);
    };
    if !part.complete {
        return Err(malformed(&format!("{name} is too large")));
    }
    Ok(Some(String::from_utf8_lossy(&part.bytes).into_owned()))
}

struct Sheet {
    name: String,
    relation: String,
}

struct Workbook {
    sheets: Vec<Sheet>,
    epoch: i64,
}

impl Workbook {
    fn parse(text: &str) -> Self {
        let mut workbook = Workbook {
            sheets: Vec::new(),
            epoch: EPOCH_1900,
        };
        for token in tokens(text) {
            let Token::Start {
                name, attributes, ..
            } = token
            else {
                continue;
            };
            match local_name(name) {
                "sheet" => {
                    if let (Some(name), Some(relation)) =
                        (attribute(attributes, "name"), attribute(attributes, "id"))
                    {
                        workbook.sheets.push(Sheet { name, relation });
                    }
                }
                "workbookPr" => {
                    if matches!(
                        attribute(attributes, "date1904").as_deref(),
                        Some("1" | "true")
                    ) {
                        workbook.epoch = EPOCH_1904;
                    }
                }
                _ => {}
            }
        }
        workbook
    }
}

/// Relationship id to target, as written.
fn relations(text: &str) -> HashMap<String, String> {
    tokens(text)
        .filter_map(|token| match token {
            Token::Start {
                name, attributes, ..
            } if local_name(name) == "Relationship" => Some((
                attribute(attributes, "Id")?,
                attribute(attributes, "Target")?,
            )),
            _ => None,
        })
        .collect()
}

/// Targets are relative to `xl/`, or absolute from the archive root.
fn part_path(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    }
}

/// Every `<si>` item's text; phonetic hints (`<rPh>`) are left out.
fn shared_strings(text: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    let mut in_phonetic = false;
    for token in tokens(text) {
        match token {
            Token::Start { name, empty, .. } => match local_name(name) {
                "t" if !empty => in_text = true,
                "rPh" if !empty => in_phonetic = true,
                "si" if empty => strings.push(String::new()),
                _ => {}
            },
            Token::End { name } => match local_name(name) {
                "t" => in_text = false,
                "rPh" => in_phonetic = false,
                "si" => strings.push(std::mem::take(&mut current)),
                _ => {}
            },
            Token::Text(text) if in_text && !in_phonetic => current += &unescape(text),
            Token::Text(_) => {}
        }
    }
    strings
}

/// How a cell style shows numbers that are dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateStyle {
    NotDate,
    Date,
    DateTime,
}

/// The date style of each cell format (`<cellXfs>`), by its index.
fn date_styles(text: &str) -> Vec<DateStyle> {
    let mut custom = HashMap::new();
    let mut styles = Vec::new();
    let mut in_cell_formats = false;
    for token in tokens(text) {
        match token {
            Token::Start {
                name,
                attributes,
                empty,
            } => match local_name(name) {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        attribute(attributes, "numFmtId"),
                        attribute(attributes, "formatCode"),
                    ) {
                        custom.insert(id, code);
                    }
                }
                "cellXfs" if !empty => in_cell_formats = true,
                "xf" if in_cell_formats => {
                    let id = attribute(attributes, "numFmtId").unwrap_or_default();
                    styles.push(match custom.get(&id) {
                        Some(code) => custom_date_style(code),
                        None => built_in_date_style(&id),
                    });
                }
                _ => {}
            },
            Token::End { name } if local_name(name) == "cellXfs" => in_cell_formats = false,
            _ => {}
        }
    }
    styles
}

/// Excel's built-in formats 14 to 17 are dates and 22 a date with a time; 18 to 21 and
/// 45 to 47 are times of day, which stay numbers.
fn built_in_date_style(id: &str) -> DateStyle {
    match id.parse::<u32>() {
        Ok(14..=17) => DateStyle::Date,
        Ok(22) => DateStyle::DateTime,
        _ => DateStyle::NotDate,
    }
}

/// A custom format is a date when it shows a year or a day; quoted text, `[...]` parts
/// such as colors and escaped characters are not part of the pattern.
fn custom_date_style(code: &str) -> DateStyle {
    let mut pattern = String::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                chars.by_ref().find(|&c| c == '"');
            }
            '[' => {
                chars.by_ref().find(|&c| c == ']');
            }
            '\\' => {
                chars.next();
            }
            _ => pattern.push(c.to_ascii_lowercase()),
        }
    }
    let has_date = pattern.contains('y') || pattern.contains('d');
    let has_time = pattern.contains('h') || pattern.contains('s');
    match (has_date, has_time) {
        (true, true) => DateStyle::DateTime,
        (true, false) => DateStyle::Date,
        _ => DateStyle::NotDate,
    }
}

/// What turns a cell's stored value into text.
struct Cells<'a> {
    shared: &'a [String],
    styles: &'a [DateStyle],
    epoch: i64,
}

impl Cells<'_> {
    /// The cell's text; `None` for an empty cell.
    fn value(&self, kind: Option<&str>, style: Option<usize>, raw: &str) -> Option<String> {
        let value = match kind {
            Some("s") => self.shared.get(raw.trim().parse::<usize>().ok()?)?.clone(),
            Some("b") => match raw.trim() {
                "1" => "true".to_string(),
                _ => "false".to_string(),
            },
            Some("inlineStr" | "str" | "e" | "d") => raw.to_string(),
            _ => {
                let date_style = style
                    .and_then(|index| self.styles.get(index).copied())
                    .unwrap_or(DateStyle::NotDate);
                date_text(raw.trim(), date_style, self.epoch)
                    .unwrap_or_else(|| raw.trim().to_string())
            }
        };
        (!value.trim().is_empty()).then_some(value)
    }
}

/// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` for a serial date number in a date style.
fn date_text(raw: &str, style: DateStyle, epoch: i64) -> Option<String> {
    if style == DateStyle::NotDate {
        return None;
    }
    let serial: f64 = raw.parse().ok()?;
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * SECONDS_PER_DAY).round() as i64;
    if seconds >= SECONDS_PER_DAY as i64 {
        days += 1;
        seconds = 0;
    }
    let unix_days = if epoch == EPOCH_1900 {
        match days {
            FICTIONAL_LEAP_DAY => return None,
            ..FICTIONAL_LEAP_DAY => days + epoch + 1,
            _ => days + epoch,
        }
    } else {
        days + epoch
    };
    let (year, month, day) = civil_from_days(unix_days);
    let date = format!("{year:04}-{month:02}-{day:02}");
    Some(match style {
        DateStyle::DateTime => format!(
            "{date} {:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        ),
        _ => date,
    })
}

/// Year, month and day of a day count since 1970-01-01, in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

struct SheetTable {
    header: Vec<String>,
    /// The first data rows, one value per header column.
    rows: Vec<Vec<Option<String>>>,
    /// Every row with a value after the header, kept or not.
    data_rows: usize,
}

/// The first row with a value is the header; `kept` data rows after it are kept.
fn read_sheet(text: &str, cells: &Cells, kept: usize) -> SheetTable {
    let mut header: Option<Vec<(usize, String)>> = None;
    let mut rows: Vec<Vec<(usize, String)>> = Vec::new();
    let mut data_rows = 0;
    let mut row: Vec<(usize, String)> = Vec::new();
    let mut cell = CellState::default();
    for token in tokens(text) {
        match token {
            Token::Start {
                name,
                attributes,
                empty,
            } => match local_name(name) {
                "c" => {
                    cell = CellState {
                        column: attribute(attributes, "r")
                            .and_then(|reference| column_index(&reference))
                            .unwrap_or_else(|| row.last().map_or(0, |(column, _)| column + 1)),
                        kind: attribute(attributes, "t"),
                        style: attribute(attributes, "s").and_then(|s| s.parse().ok()),
                        ..CellState::default()
                    };
                    if empty {
                        cell = CellState::default();
                    }
                }
                "v" | "t" if !empty => cell.in_value = true,
                "rPh" if !empty => cell.in_phonetic = true,
                _ => {}
            },
            Token::End { name } => match local_name(name) {
                "v" | "t" => cell.in_value = false,
                "rPh" => cell.in_phonetic = false,
                "c" => {
                    let state = std::mem::take(&mut cell);
                    if let Some(value) = cells.value(state.kind.as_deref(), state.style, &state.raw)
                    {
                        row.push((state.column, value));
                    }
                }
                "row" => {
                    let values = std::mem::take(&mut row);
                    if values.is_empty() {
                        continue;
                    }
                    if header.is_none() {
                        header = Some(values);
                    } else {
                        data_rows += 1;
                        if rows.len() < kept {
                            rows.push(values);
                        }
                    }
                }
                _ => {}
            },
            Token::Text(text) if cell.in_value && !cell.in_phonetic => cell.raw += &unescape(text),
            Token::Text(_) => {}
        }
    }
    let header = header.unwrap_or_default();
    let width = header
        .iter()
        .chain(rows.iter().flatten())
        .map(|(column, _)| column + 1)
        .max()
        .unwrap_or(0);
    let mut names: Vec<String> = (0..width).map(column_letters).collect();
    for (column, name) in header {
        names[column] = name;
    }
    let rows = rows
        .into_iter()
        .map(|values| {
            let mut full = vec![None; width];
            for (column, value) in values {
                full[column] = Some(value);
            }
            full
        })
        .collect();
    SheetTable {
        header: names,
        rows,
        data_rows,
    }
}

#[derive(Default)]
struct CellState {
    column: usize,
    kind: Option<String>,
    style: Option<usize>,
    raw: String,
    in_value: bool,
    in_phonetic: bool,
}

/// Zero-based column of a cell reference such as `AB12`.
fn column_index(reference: &str) -> Option<usize> {
    let letters = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .map(|b| usize::from(b.to_ascii_uppercase() - b'A') + 1);
    let number = letters.fold(0usize, |number, letter| number * 26 + letter);
    number.checked_sub(1)
}

/// `A`, `B`, ..., `Z`, `AA`: the name of a column without a header.
fn column_letters(index: usize) -> String {
    let mut number = index + 1;
    let mut letters = Vec::new();
    while number > 0 {
        let rest = (number - 1) % 26;
        letters.push(b'A' + rest as u8);
        number = (number - 1) / 26;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap_or_default()
}

fn malformed(reason: &str) -> ReadError {
    ReadError::Malformed {
        format: "Excel",
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_references_and_letters_match() {
        assert_eq!(column_index("A1"), Some(0));
        assert_eq!(column_index("Z9"), Some(25));
        assert_eq!(column_index("AA10"), Some(26));
        assert_eq!(column_index("1"), None);
        for index in [0, 25, 26, 701, 702] {
            assert_eq!(
                column_index(&format!("{}1", column_letters(index))),
                Some(index)
            );
        }
    }

    #[test]
    fn serial_dates_follow_excels_calendar() {
        let date = |raw| date_text(raw, DateStyle::Date, EPOCH_1900);
        assert_eq!(date("1").as_deref(), Some("1900-01-01"));
        assert_eq!(date("59").as_deref(), Some("1900-02-28"));
        assert_eq!(date("60"), None);
        assert_eq!(date("61").as_deref(), Some("1900-03-01"));
        assert_eq!(date("45580").as_deref(), Some("2024-10-15"));
        assert_eq!(
            date_text("0", DateStyle::Date, EPOCH_1904).as_deref(),
            Some("1904-01-01")
        );
    }

    #[test]
    fn a_time_just_before_midnight_rounds_into_the_next_day() {
        assert_eq!(
            date_text("45580.999999999", DateStyle::DateTime, EPOCH_1900).as_deref(),
            Some("2024-10-16 00:00:00")
        );
    }

    #[test]
    fn custom_formats_are_dates_only_with_a_year_or_day() {
        assert_eq!(custom_date_style("dd.mm.yyyy"), DateStyle::Date);
        assert_eq!(custom_date_style("yyyy-mm-dd hh:mm"), DateStyle::DateTime);
        assert_eq!(custom_date_style("0.00"), DateStyle::NotDate);
        assert_eq!(custom_date_style("[Red]0.00"), DateStyle::NotDate);
        assert_eq!(custom_date_style("\"day\" 0"), DateStyle::NotDate);
        assert_eq!(custom_date_style("hh:mm"), DateStyle::NotDate);
    }
}
