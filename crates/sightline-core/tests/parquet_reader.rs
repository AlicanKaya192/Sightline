mod common;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{Values, table, write_bytes, write_parquet};
use parquet::data_type::{Int32Type, Int64Type};
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;
use sightline_core::{
    Basis, Bounds, ColumnType, Limits, NullCount, ParquetReader, ReadError, Reader, RowCount,
    Shape, TableFormat, TableSummary,
};

const SCHEMA: &str = "message people {
    optional int64 id;
    optional binary name (STRING);
    optional double score;
    optional boolean active;
    optional int32 joined (DATE);
}";

/// Five rows in two row groups; 2022-01-08 is day 19000.
fn people_row_groups() -> Vec<Vec<Values>> {
    vec![
        vec![
            Values::Int64(vec![Some(1), Some(2), Some(3)]),
            Values::Text(vec![Some("ayşe"), None, Some("can")]),
            Values::Double(vec![Some(1.5), None, None]),
            Values::Bool(vec![Some(true), Some(false), None]),
            Values::Int32(vec![Some(19000), Some(19001), None]),
        ],
        vec![
            Values::Int64(vec![Some(4), Some(5)]),
            Values::Text(vec![Some("deniz"), Some("ali")]),
            Values::Double(vec![Some(-2.0), Some(3.25)]),
            Values::Bool(vec![None, Some(true)]),
            Values::Int32(vec![None, None]),
        ],
    ]
}

fn write_people(name: &str, statistics: EnabledStatistics) -> PathBuf {
    let properties = WriterProperties::builder()
        .set_statistics_enabled(statistics)
        .build();
    write_parquet(name, SCHEMA, &people_row_groups(), properties)
}

fn summarize(path: &Path, limits: &Limits) -> TableSummary {
    table(ParquetReader.summarize(path, limits).expect("summary"))
}

fn whole_file_nulls(count: u64) -> Option<NullCount> {
    Some(NullCount {
        count,
        of_rows: 5,
        basis: Basis::WholeFile,
    })
}

fn bounds(min: &str, max: &str) -> Option<Bounds> {
    Some(Bounds {
        min: min.into(),
        max: max.into(),
    })
}

#[test]
fn accepts_parquet_file_names_only() {
    assert!(ParquetReader.can_read(Path::new("data.parquet")));
    assert!(ParquetReader.can_read(Path::new("DATA.PARQUET")));
    assert!(!ParquetReader.can_read(Path::new("data.csv")));
    assert!(!ParquetReader.can_read(Path::new("parquet")));
}

#[test]
fn reads_row_count_and_row_groups_from_metadata() {
    let path = write_people("people_rows.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    assert_eq!(summary.rows, RowCount::Exact(5));
    assert_eq!(summary.format, TableFormat::Parquet { row_groups: 2 });
}

#[test]
fn maps_schema_to_column_names_and_types() {
    let path = write_people("people_schema.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    let columns: Vec<(&str, ColumnType)> = summary
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.column_type))
        .collect();
    assert_eq!(
        columns,
        [
            ("id", ColumnType::Integer),
            ("name", ColumnType::Text),
            ("score", ColumnType::Float),
            ("active", ColumnType::Boolean),
            ("joined", ColumnType::Date),
        ]
    );
}

#[test]
fn sums_null_counts_of_all_row_groups() {
    let path = write_people("people_nulls.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    let nulls: Vec<Option<NullCount>> = summary.columns.iter().map(|c| c.nulls).collect();
    assert_eq!(
        nulls,
        [
            whole_file_nulls(0),
            whole_file_nulls(1),
            whole_file_nulls(2),
            whole_file_nulls(2),
            whole_file_nulls(3),
        ]
    );
}

#[test]
fn merges_min_and_max_of_all_row_groups() {
    let path = write_people("people_bounds.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    let columns = &summary.columns;
    assert_eq!(columns[0].bounds, bounds("1", "5"));
    assert_eq!(columns[1].bounds, bounds("ali", "deniz"));
    assert_eq!(columns[2].bounds, bounds("-2", "3.25"));
    assert_eq!(columns[3].bounds, bounds("false", "true"));
}

#[test]
fn counts_nulls_in_a_sample_when_statistics_are_missing() {
    let path = write_people("people_no_stats.parquet", EnabledStatistics::None);
    let limits = Limits {
        sample_rows: 4,
        ..Limits::default()
    };

    let summary = summarize(&path, &limits);

    let name = &summary.columns[1];
    assert_eq!(
        name.nulls,
        Some(NullCount {
            count: 1,
            of_rows: 4,
            basis: Basis::Sample,
        })
    );
    assert_eq!(name.bounds, None);
}

#[test]
fn sample_covering_every_row_counts_for_the_whole_file() {
    let path = write_people("people_no_stats_full.parquet", EnabledStatistics::None);

    let summary = summarize(&path, &Limits::default());

    assert_eq!(summary.columns[2].nulls, whole_file_nulls(2));
}

#[test]
fn returns_first_rows_as_text_with_nulls_as_none() {
    let path = write_people("people_head.parquet", EnabledStatistics::Chunk);
    let limits = Limits {
        head_rows: 2,
        ..Limits::default()
    };

    let summary = summarize(&path, &limits);

    let text = |s: &str| Some(s.to_string());
    assert_eq!(
        summary.head,
        [
            vec![
                text("1"),
                text("ayşe"),
                text("1.5"),
                text("true"),
                text("2022-01-08")
            ],
            vec![text("2"), None, None, text("false"), text("2022-01-09")],
        ]
    );
}

#[test]
fn rejects_a_file_that_is_not_parquet() {
    let path = write_bytes("not_really.parquet", b"id,name\n1,ali\n");

    let result = ParquetReader.summarize(&path, &Limits::default());

    assert!(matches!(result, Err(ReadError::Parquet(_))), "{result:?}");
}

fn same_value_per_row_group(name: &str) -> PathBuf {
    let row_groups: Vec<Vec<Values>> = (1..=3)
        .map(|n| vec![Values::Int64(vec![Some(n); 5])])
        .collect();
    let schema = "message t { optional int64 n; }";
    write_parquet(
        name,
        schema,
        &row_groups,
        WriterProperties::builder().build(),
    )
}

#[test]
fn sample_is_spread_over_all_row_groups() {
    let path = same_value_per_row_group("spread.parquet");
    let limits = Limits {
        sample_rows: 3,
        ..Limits::default()
    };

    let summary = summarize(&path, &limits);

    let profile = summary.columns[0].profile.as_ref().expect("profile");
    assert_eq!((profile.values, profile.distinct), (3, 3));
    assert_eq!(profile.basis, Basis::Sample);
}

#[test]
fn first_rows_stay_the_first_rows_of_the_file() {
    let path = same_value_per_row_group("spread_head.parquet");
    let limits = Limits {
        head_rows: 4,
        sample_rows: 3,
        ..Limits::default()
    };

    let summary = summarize(&path, &limits);

    assert_eq!(summary.head, vec![vec![Some("1".to_string())]; 4]);
}

#[test]
fn every_column_gets_a_profile_of_its_values() {
    let path = write_people("people_profile.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    let id = summary.columns[0].profile.as_ref().expect("id profile");
    assert_eq!((id.values, id.distinct, id.basis), (5, 5, Basis::WholeFile));
    let name = summary.columns[1].profile.as_ref().expect("name profile");
    assert_eq!(name.values, 4);
}

#[test]
fn a_sample_reports_how_many_rows_it_used() {
    let path = same_value_per_row_group("sampled_rows.parquet");
    let limits = Limits {
        sample_rows: 3,
        ..Limits::default()
    };

    let sampled = summarize(&path, &limits);
    let whole = summarize(&path, &Limits::default());

    assert_eq!(sampled.sampled_rows, Some(3));
    assert_eq!(whole.sampled_rows, None);
}

/// 20,000 rows in one row group and many small pages, without statistics; only the
/// last 1,000 rows are null, as in files where missing values sit at the end.
fn nulls_at_the_end(name: &str) -> PathBuf {
    let values: Vec<Option<i64>> = (0..20_000).map(|n| (n < 19_000).then_some(n)).collect();
    let properties = WriterProperties::builder()
        .set_statistics_enabled(EnabledStatistics::None)
        .set_write_batch_size(100)
        .set_data_page_row_count_limit(100)
        .build();
    write_parquet(
        name,
        "message t { optional int64 n; }",
        &[vec![Values::Int64(values)]],
        properties,
    )
}

#[test]
fn null_sample_reaches_the_end_of_a_row_group() {
    let path = nulls_at_the_end("nulls_at_end.parquet");
    let limits = Limits {
        sample_rows: 1_000,
        ..Limits::default()
    };

    let nulls = summarize(&path, &limits).columns[0]
        .nulls
        .expect("null count");

    assert_eq!((nulls.of_rows, nulls.basis), (1_000, Basis::Sample));
    let ratio = nulls.ratio();
    assert!((0.03..=0.07).contains(&ratio), "null ratio {ratio}");
}

#[test]
fn null_sample_covering_every_row_is_exact() {
    let path = nulls_at_the_end("nulls_at_end_full.parquet");
    let limits = Limits {
        sample_rows: 20_000,
        ..Limits::default()
    };

    let nulls = summarize(&path, &limits).columns[0].nulls;

    assert_eq!(
        nulls,
        Some(NullCount {
            count: 1_000,
            of_rows: 20_000,
            basis: Basis::WholeFile,
        })
    );
}

/// Two row groups of 10,000 rows with many small pages. Only the last 1,000 rows of the
/// file hold 2 and "b"; the column `e` is always empty.
fn values_at_the_end(name: &str) -> PathBuf {
    let row_group = |late: bool| {
        let late_row = |row: usize| late && row >= 9_000;
        vec![
            Values::Int64(
                (0..10_000)
                    .map(|row| Some(if late_row(row) { 2 } else { 1 }))
                    .collect(),
            ),
            Values::Text(
                (0..10_000)
                    .map(|row| Some(if late_row(row) { "b" } else { "a" }))
                    .collect(),
            ),
            Values::Double(vec![None; 10_000]),
        ]
    };
    let properties = WriterProperties::builder()
        .set_write_batch_size(100)
        .set_data_page_row_count_limit(100)
        .build();
    write_parquet(
        name,
        "message t { optional int64 n; optional binary s (STRING); optional double e; }",
        &[row_group(false), row_group(true)],
        properties,
    )
}

#[test]
fn profile_sample_reaches_the_end_of_the_file() {
    let path = values_at_the_end("values_at_end.parquet");
    let limits = Limits {
        sample_rows: 1_000,
        ..Limits::default()
    };

    let summary = summarize(&path, &limits);

    let numbers = summary.columns[0].profile.as_ref().expect("number profile");
    assert_eq!((numbers.values, numbers.distinct), (1_000, 2));
    assert_eq!(numbers.basis, Basis::Sample);
    let texts = summary.columns[1].profile.as_ref().expect("text profile");
    let Shape::TopValues(top) = &texts.shape else {
        panic!("text shape {:?}", texts.shape);
    };
    assert!(top.iter().any(|(value, _)| value == "b"), "{top:?}");
    assert_eq!(summary.sampled_rows, Some(1_000));
}

#[test]
fn an_always_empty_column_gets_an_empty_profile() {
    let path = values_at_the_end("always_empty.parquet");
    let limits = Limits {
        sample_rows: 1_000,
        ..Limits::default()
    };

    let empty = &summarize(&path, &limits).columns[2];

    assert_eq!(empty.profile.as_ref().expect("profile").values, 0);
    assert_eq!(empty.nulls.map(|nulls| nulls.ratio()), Some(1.0));
}

/// Five rows of a flat column and a nested one; the group is null in rows 0 and 4 and
/// present with a null inside in row 2.
fn with_a_nested_column(name: &str) -> PathBuf {
    let path = common::fixture_path(name);
    let schema = "message t { optional int64 n; optional group g { optional int32 x; } }";
    let schema = Arc::new(parse_message_type(schema).expect("valid schema"));
    let properties = WriterProperties::builder()
        .set_statistics_enabled(EnabledStatistics::None)
        .build();
    let file = File::create(&path).expect("create fixture");
    let mut writer =
        SerializedFileWriter::new(file, schema, Arc::new(properties)).expect("parquet writer");
    let mut row_group = writer.next_row_group().expect("row group");
    let mut column = row_group.next_column().expect("column").expect("n");
    column
        .typed::<Int64Type>()
        .write_batch(&[1, 2, 3, 4, 5], Some(&[1; 5]), None)
        .expect("write n");
    column.close().expect("close n");
    let mut column = row_group.next_column().expect("column").expect("g.x");
    column
        .typed::<Int32Type>()
        .write_batch(&[7, 8], Some(&[0, 2, 1, 2, 0]), None)
        .expect("write g.x");
    column.close().expect("close g.x");
    row_group.close().expect("close row group");
    writer.close().expect("close file");
    path
}

#[test]
fn nested_columns_still_count_nulls_from_whole_rows() {
    let path = with_a_nested_column("nested.parquet");

    let summary = summarize(&path, &Limits::default());

    assert_eq!(summary.columns[1].column_type, ColumnType::Nested);
    assert_eq!(summary.columns[1].nulls, whole_file_nulls(2));
    assert_eq!(summary.columns[0].nulls, whole_file_nulls(0));
}

#[test]
fn analysis_rows_keep_every_null_in_its_own_row_and_column() {
    let path = write_people("people_analysis.parquet", EnabledStatistics::Chunk);

    let summary = summarize(&path, &Limits::default());

    let missing = &summary.analysis.missing;
    assert_eq!((missing.rows, missing.columns), (5, 5));
    // Row 2 of the file: id 2, no name, no score, false, 2022-01-09.
    assert_eq!(missing.missing[5..10], [false, true, true, false, false]);
    let score = summary.analysis.stats[2].expect("score statistics");
    assert_eq!((score.count, score.min, score.max), (3, -2.0, 3.25));
}
