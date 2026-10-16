mod common;

use std::path::Path;

use common::xlsx::{workbook, zip};
use common::{table, write_bytes};
use sightline_core::{
    Basis, ColumnType, Limits, ReadError, Reader, RowCount, TableFormat, TableSummary, XlsxReader,
    summarize,
};

fn summarize_parts(name: &str, parts: &[(String, String)], deflate: bool) -> TableSummary {
    let path = write_bytes(name, &zip(parts, deflate));
    table(
        XlsxReader
            .summarize(&path, &Limits::default())
            .expect("summary"),
    )
}

fn names(summary: &TableSummary) -> Vec<&str> {
    summary.columns.iter().map(|c| c.name.as_str()).collect()
}

fn types(summary: &TableSummary) -> Vec<ColumnType> {
    summary.columns.iter().map(|c| c.column_type).collect()
}

fn text(value: &str) -> Option<String> {
    Some(value.to_string())
}

/// Header row from shared strings 0..3, then two rows of numbers and text.
const PEOPLE: &str = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row><row r="2"><c r="A2"><v>34</v></c><c r="B2"><v>1.5</v></c><c r="C2" t="s"><v>3</v></c></row><row r="3"><c r="A3"><v>41</v></c><c r="B3"><v>2.25</v></c><c r="C3" t="s"><v>4</v></c></row>"#;
const PEOPLE_STRINGS: [&str; 5] = ["age", "score", "city", "İzmir", "Ağrı"];

#[test]
fn accepts_excel_workbook_file_names_only() {
    assert!(XlsxReader.can_read(Path::new("data.xlsx")));
    assert!(XlsxReader.can_read(Path::new("DATA.XLSM")));
    assert!(!XlsxReader.can_read(Path::new("data.xls")));
    assert!(!XlsxReader.can_read(Path::new("data.csv")));
}

#[test]
fn reads_header_types_rows_and_first_rows() {
    let parts = workbook(&[("People", PEOPLE)], &PEOPLE_STRINGS, None);

    let summary = summarize_parts("people.xlsx", &parts, false);

    assert_eq!(names(&summary), ["age", "score", "city"]);
    assert_eq!(
        types(&summary),
        [ColumnType::Integer, ColumnType::Float, ColumnType::Text]
    );
    assert_eq!(summary.rows, RowCount::Exact(2));
    assert_eq!(
        summary.head,
        [
            vec![text("34"), text("1.5"), text("İzmir")],
            vec![text("41"), text("2.25"), text("Ağrı")],
        ]
    );
    assert_eq!(
        summary.columns[0].nulls.expect("nulls").basis,
        Basis::WholeFile
    );
}

#[test]
fn reads_compressed_parts() {
    let parts = workbook(&[("People", PEOPLE)], &PEOPLE_STRINGS, None);

    let summary = summarize_parts("people-deflate.xlsx", &parts, true);

    assert_eq!(names(&summary), ["age", "score", "city"]);
    assert_eq!(summary.rows, RowCount::Exact(2));
}

#[test]
fn summarizes_the_first_sheet_and_names_them_all() {
    let other = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>x</t></is></c></row>"#;
    let parts = workbook(
        &[("People", PEOPLE), ("Notes", other)],
        &PEOPLE_STRINGS,
        None,
    );

    let summary = summarize_parts("two-sheets.xlsx", &parts, false);

    assert_eq!(
        summary.format,
        TableFormat::Excel {
            sheets: vec!["People".to_string(), "Notes".to_string()]
        }
    );
    assert_eq!(names(&summary), ["age", "score", "city"]);
}

#[test]
fn missing_cells_stay_empty_in_their_column() {
    let data = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>a</t></is></c><c r="B1" t="inlineStr"><is><t>b</t></is></c><c r="C1" t="inlineStr"><is><t>c</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="C2"><v>3</v></c></row><row r="3"><c r="B3"><v>2</v></c></row>"#;
    let parts = workbook(&[("S", data)], &[], None);

    let summary = summarize_parts("gaps.xlsx", &parts, false);

    assert_eq!(
        summary.head,
        [
            vec![text("1"), None, text("3")],
            vec![None, text("2"), None],
        ]
    );
}

#[test]
fn reads_inline_strings_booleans_errors_and_formula_results() {
    let data = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>flag</t></is></c><c r="B1" t="inlineStr"><is><t>total</t></is></c><c r="C1" t="inlineStr"><is><t>note</t></is></c></row><row r="2"><c r="A2" t="b"><v>1</v></c><c r="B2"><f>SUM(1,2)</f><v>3</v></c><c r="C2" t="e"><v>#DIV/0!</v></c></row><row r="3"><c r="A3" t="b"><v>0</v></c><c r="B3"><f>SUM(2,2)</f><v>4</v></c><c r="C3" t="str"><f>"a"&amp;"b"</f><v>ab</v></c></row>"#;
    let parts = workbook(&[("S", data)], &[], None);

    let summary = summarize_parts("kinds.xlsx", &parts, false);

    assert_eq!(
        summary.head,
        [
            vec![text("true"), text("3"), text("#DIV/0!")],
            vec![text("false"), text("4"), text("ab")],
        ]
    );
    assert_eq!(
        types(&summary)[..2],
        [ColumnType::Boolean, ColumnType::Integer]
    );
}

#[test]
fn date_formatted_numbers_become_dates() {
    // Style 1 uses the built-in date format 14, style 2 a custom date and time format.
    let styles = r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/></cellXfs>"#;
    let data = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>day</t></is></c><c r="B1" t="inlineStr"><is><t>at</t></is></c><c r="C1" t="inlineStr"><is><t>plain</t></is></c></row><row r="2"><c r="A2" s="1"><v>45580</v></c><c r="B2" s="2"><v>45580.5</v></c><c r="C2" s="0"><v>45580</v></c></row><row r="3"><c r="A3" s="1"><v>45581</v></c><c r="B3" s="2"><v>45581.75</v></c><c r="C3"><v>7</v></c></row>"#;
    let parts = workbook(&[("S", data)], &[], Some(styles));

    let summary = summarize_parts("dates.xlsx", &parts, false);

    assert_eq!(
        summary.head,
        [
            vec![
                text("2024-10-15"),
                text("2024-10-15 12:00:00"),
                text("45580")
            ],
            vec![text("2024-10-16"), text("2024-10-16 18:00:00"), text("7")],
        ]
    );
    assert_eq!(
        types(&summary),
        [ColumnType::Date, ColumnType::DateTime, ColumnType::Integer]
    );
}

#[test]
fn text_with_markup_characters_is_unescaped() {
    let parts = workbook(
        &[(
            "S",
            r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row><row r="2"><c r="A2" t="s"><v>1</v></c></row>"#,
        )],
        &["name", "Tom &amp; Jerry &lt;3 &#246;"],
        None,
    );

    let summary = summarize_parts("escaped.xlsx", &parts, false);

    assert_eq!(summary.head, [vec![text("Tom & Jerry <3 ö")]]);
}

#[test]
fn rows_beyond_the_sample_are_counted() {
    let mut data =
        String::from(r#"<row r="1"><c r="A1" t="inlineStr"><is><t>n</t></is></c></row>"#);
    for row in 2..=30 {
        data += &format!(r#"<row r="{row}"><c r="A{row}"><v>{row}</v></c></row>"#);
    }
    let parts = workbook(&[("S", &data)], &[], None);
    let path = write_bytes("many.xlsx", &zip(&parts, true));
    let limits = Limits {
        sample_rows: 10,
        head_rows: 5,
        ..Limits::default()
    };

    let summary = table(XlsxReader.summarize(&path, &limits).expect("summary"));

    assert_eq!(summary.rows, RowCount::Exact(29));
    assert_eq!(summary.head.len(), 5);
    assert_eq!(summary.sampled_rows, Some(10));
}

#[test]
fn a_file_that_is_not_a_zip_archive_is_malformed() {
    // Password-protected workbooks are OLE files, not zip archives.
    let path = write_bytes(
        "encrypted.xlsx",
        b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 not a zip archive",
    );

    let error = XlsxReader
        .summarize(&path, &Limits::default())
        .expect_err("must fail");

    assert!(matches!(error, ReadError::Malformed { .. }), "{error:?}");
}

#[test]
fn summarize_picks_the_excel_reader() {
    let parts = workbook(&[("People", PEOPLE)], &PEOPLE_STRINGS, None);
    let path = write_bytes("picked.xlsx", &zip(&parts, true));

    let summary = table(summarize(&path, &Limits::default()).expect("summary"));

    assert_eq!(summary.rows, RowCount::Exact(2));
}
