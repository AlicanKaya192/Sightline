mod common;

use std::path::Path;

use common::{table, write_bytes};
use sightline_core::{
    Basis, ColumnType, CsvReader, InsightKind, Limits, NullCount, Reader, RowCount, Shape,
    TableFormat, TableSummary, TextEncoding,
};

fn summarize_bytes(name: &str, content: &[u8], limits: &Limits) -> TableSummary {
    let path = write_bytes(name, content);
    table(CsvReader.summarize(&path, limits).expect("summary"))
}

fn summarize(name: &str, content: &str) -> TableSummary {
    summarize_bytes(name, content.as_bytes(), &Limits::default())
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

#[test]
fn accepts_csv_file_names_only() {
    assert!(CsvReader.can_read(Path::new("data.csv")));
    assert!(CsvReader.can_read(Path::new("DATA.CSV")));
    assert!(!CsvReader.can_read(Path::new("data.parquet")));
}

#[test]
fn detects_comma_delimiter() {
    let summary = summarize("comma.csv", "id,name,score\n1,ali,2.5\n2,veli,3\n");

    assert_eq!(
        summary.format,
        TableFormat::Csv {
            delimiter: b',',
            encoding: TextEncoding::Utf8
        }
    );
    assert_eq!(names(&summary), ["id", "name", "score"]);
    assert_eq!(summary.rows, RowCount::Exact(2));
}

#[test]
fn detects_semicolon_delimiter() {
    let summary = summarize("semicolon.csv", "id;name\n1;ali, veli\n2;can\n");

    assert!(matches!(
        summary.format,
        TableFormat::Csv {
            delimiter: b';',
            ..
        }
    ));
    assert_eq!(names(&summary), ["id", "name"]);
}

#[test]
fn reads_windows_1254_text() {
    let (bytes, _, _) = encoding_rs::WINDOWS_1254.encode("ad;şehir\nAyşe;İzmir\nIşık;Muğla\n");

    let summary = summarize_bytes("turkish_ansi.csv", &bytes, &Limits::default());

    assert_eq!(
        summary.format,
        TableFormat::Csv {
            delimiter: b';',
            encoding: TextEncoding::Windows1254
        }
    );
    assert_eq!(names(&summary), ["ad", "şehir"]);
    assert_eq!(summary.head[1], [text("Işık"), text("Muğla")]);
}

#[test]
fn skips_utf8_byte_order_mark() {
    let summary = summarize_bytes("bom.csv", b"\xEF\xBB\xBFid,ad\n1,x\n", &Limits::default());

    assert_eq!(names(&summary), ["id", "ad"]);
    assert!(matches!(
        summary.format,
        TableFormat::Csv {
            encoding: TextEncoding::Utf8,
            ..
        }
    ));
}

#[test]
fn infers_column_types() {
    let summary = summarize(
        "types.csv",
        "flag,count,ratio,day,moment,label\n\
         true,1,1.5,2024-01-31,2024-01-31 10:00:00,a\n\
         FALSE,-2,2,2024-02-01,2024-02-01T11:30:00,b\n",
    );

    assert_eq!(
        types(&summary),
        [
            ColumnType::Boolean,
            ColumnType::Integer,
            ColumnType::Float,
            ColumnType::Date,
            ColumnType::DateTime,
            ColumnType::Text,
        ]
    );
}

#[test]
fn accepts_decimal_comma_when_semicolon_separates_fields() {
    let summary = summarize("decimal_comma.csv", "id;fiyat\n1;2,5\n2;3,75\n");

    assert_eq!(types(&summary), [ColumnType::Integer, ColumnType::Float]);
}

#[test]
fn rejects_impossible_dates() {
    let summary = summarize("bad_date.csv", "day\n2024-13-01\n");

    assert_eq!(types(&summary), [ColumnType::Text]);
}

#[test]
fn counts_empty_fields_as_nulls_over_the_whole_file() {
    let summary = summarize("nulls.csv", "a,b\n1,\n,x\n3,y\n");

    let expected = Some(NullCount {
        count: 1,
        of_rows: 3,
        basis: Basis::WholeFile,
    });
    assert_eq!(summary.columns[0].nulls, expected);
    assert_eq!(summary.columns[1].nulls, expected);
    assert_eq!(summary.head[1], [None, text("x")]);
}

#[test]
fn missing_trailing_fields_are_nulls() {
    let summary = summarize("ragged.csv", "a,b,c\n1,2\n");

    assert_eq!(summary.head, [vec![text("1"), text("2"), None]]);
}

#[test]
fn keeps_quoted_delimiters_inside_a_field() {
    let summary = summarize("quoted.csv", "id,note\n1,\"a, b\"\n");

    assert_eq!(summary.head, [vec![text("1"), text("a, b")]]);
}

#[test]
fn stops_at_the_sample_row_limit() {
    let content = format!("n,v\n{}", "1,\n".repeat(10));
    let limits = Limits {
        head_rows: 2,
        sample_rows: 4,
        ..Limits::default()
    };

    let summary = summarize_bytes("long.csv", content.as_bytes(), &limits);

    assert_eq!(summary.rows, RowCount::AtLeast(4));
    assert_eq!(
        summary.columns[1].nulls,
        Some(NullCount {
            count: 4,
            of_rows: 4,
            basis: Basis::Sample,
        })
    );
}

#[test]
fn ignores_a_row_cut_by_the_byte_limit() {
    let content = format!("a,b\n{}", "1,2\n".repeat(100));
    let limits = Limits {
        sample_bytes: 22,
        ..Limits::default()
    };

    let summary = summarize_bytes("cut.csv", content.as_bytes(), &limits);

    assert_eq!(summary.rows, RowCount::AtLeast(4));
}

#[test]
fn returns_at_most_head_rows() {
    let limits = Limits {
        head_rows: 2,
        ..Limits::default()
    };

    let summary = summarize_bytes("head.csv", b"n\n1\n2\n3\n4\n", &limits);

    assert_eq!(summary.head, [vec![text("1")], vec![text("2")]]);
    assert_eq!(summary.rows, RowCount::Exact(4));
}

#[test]
fn empty_file_has_no_rows_and_no_columns() {
    let summary = summarize("empty.csv", "");

    assert_eq!(summary.rows, RowCount::Exact(0));
    assert!(summary.columns.is_empty());
}

fn bounds(summary: &TableSummary, column: usize) -> Option<(&str, &str)> {
    summary.columns[column]
        .bounds
        .as_ref()
        .map(|b| (b.min.as_str(), b.max.as_str()))
}

#[test]
fn integer_bounds_compare_by_value() {
    let summary = summarize("int_bounds.csv", "n\n10\n9\n-3\n");

    assert_eq!(bounds(&summary, 0), Some(("-3", "10")));
}

#[test]
fn float_bounds_keep_the_decimal_comma() {
    let summary = summarize("float_bounds.csv", "id;fiyat\n1;12,50\n2;7,25\n3;349,00\n");

    assert_eq!(bounds(&summary, 1), Some(("7,25", "349,00")));
}

#[test]
fn date_and_datetime_bounds_are_chronological() {
    let content = "d,t\n2026-09-05,2026-09-05 10:00\n2026-09-01,2026-09-01T09:00\n";

    let summary = summarize("date_bounds.csv", content);

    assert_eq!(bounds(&summary, 0), Some(("2026-09-01", "2026-09-05")));
    assert_eq!(
        bounds(&summary, 1),
        Some(("2026-09-01T09:00", "2026-09-05 10:00"))
    );
}

#[test]
fn boolean_bounds_put_false_first() {
    let summary = summarize("bool_bounds.csv", "b\ntrue\nFALSE\n");

    assert_eq!(bounds(&summary, 0), Some(("FALSE", "true")));
}

#[test]
fn bounds_skip_empty_values() {
    let summary = summarize("empty_bounds.csv", "n,x\n5,a\n,b\n2,c\n");

    assert_eq!(bounds(&summary, 0), Some(("2", "5")));
}

#[test]
fn text_columns_have_no_bounds() {
    let summary = summarize("text_bounds.csv", "ad\nZeynep\nÇağla\n");

    assert_eq!(bounds(&summary, 0), None);
}

#[test]
fn a_sample_gives_no_bounds() {
    let content = format!("n\n{}", "1\n".repeat(10));
    let limits = Limits {
        sample_rows: 3,
        ..Limits::default()
    };

    let summary = summarize_bytes("sample_bounds.csv", content.as_bytes(), &limits);

    assert_eq!(summary.columns[0].bounds, None);
}

#[test]
fn columns_get_a_profile_read_with_the_decimal_comma() {
    let summary = summarize("csv_profile.csv", "id;fiyat\n1;2,5\n2;3,75\n2;\n");

    let id = summary.columns[0].profile.as_ref().expect("id profile");
    assert_eq!((id.values, id.distinct), (3, 2));
    let price = summary.columns[1].profile.as_ref().expect("price profile");
    assert!(
        matches!(&price.shape, Shape::Histogram(bins) if bins.iter().sum::<u64>() == 2),
        "{price:?}"
    );
}

#[test]
fn a_profile_from_a_sample_says_so() {
    let content = format!("n\n{}", "1\n".repeat(10));
    let limits = Limits {
        sample_rows: 3,
        ..Limits::default()
    };

    let summary = summarize_bytes("sample_profile.csv", content.as_bytes(), &limits);

    let profile = summary.columns[0].profile.as_ref().expect("profile");
    assert_eq!((profile.values, profile.basis), (3, Basis::Sample));
}

#[test]
fn the_summary_carries_the_insights_of_its_columns() {
    let ids: String = (1..=20).map(|n| format!("{n}\n")).collect();

    let summary = summarize("csv_insights.csv", &format!("id\n{ids}"));

    assert_eq!(
        summary.insights.iter().map(|i| &i.kind).collect::<Vec<_>>(),
        [&InsightKind::LooksLikeId]
    );
}

#[test]
fn a_sample_reports_how_many_rows_it_used() {
    let content = format!("n\n{}", "1\n".repeat(10));
    let limits = Limits {
        sample_rows: 3,
        ..Limits::default()
    };

    let sampled = summarize_bytes("sampled_rows.csv", content.as_bytes(), &limits);
    let whole = summarize("whole_rows.csv", "n\n1\n2\n");

    assert_eq!(sampled.sampled_rows, Some(3));
    assert_eq!(whole.sampled_rows, None);
}
