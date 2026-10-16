use sightline_core::{
    Basis, ColumnStats, ColumnSummary, ColumnType, Insight, InsightKind, analyze,
};

fn column(name: &str, column_type: ColumnType) -> ColumnSummary {
    ColumnSummary {
        name: name.into(),
        column_type,
        nulls: None,
        bounds: None,
        profile: None,
    }
}

/// Rows of one column from its values; `None` is a missing value.
fn one_column(values: &[Option<&str>]) -> Vec<Vec<Option<String>>> {
    values
        .iter()
        .map(|value| vec![value.map(str::to_string)])
        .collect()
}

fn numbers(values: impl IntoIterator<Item = f64>) -> Vec<Vec<Option<String>>> {
    values
        .into_iter()
        .map(|value| vec![Some(value.to_string())])
        .collect()
}

fn texts(values: &[&str]) -> Vec<Vec<Option<String>>> {
    values
        .iter()
        .map(|value| vec![Some(value.to_string())])
        .collect()
}

fn kinds(insights: &[Insight]) -> Vec<&InsightKind> {
    insights.iter().map(|insight| &insight.kind).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn numeric_columns_get_describe_statistics() {
    let columns = [column("n", ColumnType::Integer)];
    let rows = numbers((1..=9).map(f64::from));

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    let stats: ColumnStats = analysis.stats[0].expect("stats");
    assert_eq!(stats.count, 9);
    assert!(close(stats.mean, 5.0));
    assert!(close(stats.std, 7.5_f64.sqrt()), "{}", stats.std);
    assert_eq!(
        (stats.min, stats.q1, stats.median, stats.q3, stats.max),
        (1.0, 3.0, 5.0, 7.0, 9.0)
    );
    assert_eq!(analysis.rows, 9);
}

#[test]
fn quartiles_are_interpolated_between_values() {
    let columns = [column("n", ColumnType::Float)];
    let rows = numbers([1.0, 2.0, 3.0, 4.0]);

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    let stats = analysis.stats[0].expect("stats");
    assert!(close(stats.q1, 1.75) && close(stats.median, 2.5) && close(stats.q3, 3.25));
}

#[test]
fn decimal_commas_are_read_when_the_file_uses_them() {
    let columns = [column("n", ColumnType::Float)];
    let rows = texts(&["1,5", "2,5"]);

    let (analysis, _) = analyze(&columns, &rows, true, Basis::WholeFile);

    assert!(close(analysis.stats[0].expect("stats").mean, 2.0));
}

#[test]
fn text_columns_get_their_most_frequent_values_and_no_statistics() {
    let columns = [column("city", ColumnType::Text)];
    let rows = texts(&["Ağrı", "İzmir", "Ağrı", "Van", "İzmir", "Ağrı"]);

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    assert_eq!(analysis.stats[0], None);
    assert_eq!(
        analysis.categories[0],
        [
            ("Ağrı".to_string(), 3),
            ("İzmir".to_string(), 2),
            ("Van".to_string(), 1)
        ]
    );
}

#[test]
fn correlation_covers_numeric_columns_only() {
    let columns = [
        column("x", ColumnType::Integer),
        column("name", ColumnType::Text),
        column("double", ColumnType::Integer),
        column("negative", ColumnType::Integer),
    ];
    let rows: Vec<Vec<Option<String>>> = (1..=40)
        .map(|x| {
            vec![
                Some(x.to_string()),
                Some(format!("n{x}")),
                Some((2 * x).to_string()),
                Some((-x).to_string()),
            ]
        })
        .collect();

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    let correlation = analysis.correlation.expect("correlation");
    assert_eq!(correlation.columns, [0, 2, 3]);
    let at = |i: usize, j: usize| correlation.values[i * 3 + j];
    assert!(close(at(0, 0), 1.0) && close(at(0, 1), 1.0) && close(at(0, 2), -1.0));
    assert!(close(at(2, 1), -1.0));
}

#[test]
fn missing_map_marks_empty_cells_of_the_first_rows() {
    let columns = [column("a", ColumnType::Text), column("b", ColumnType::Text)];
    let mut rows = vec![
        vec![Some("x".to_string()), None],
        vec![None, Some("y".to_string())],
    ];
    rows.extend((0..300).map(|_| vec![Some("x".to_string()), Some("y".to_string())]));

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    assert_eq!((analysis.missing.rows, analysis.missing.columns), (200, 2));
    assert_eq!(analysis.missing.missing[..4], [false, true, true, false]);
}

#[test]
fn zeros_far_below_the_other_values_look_like_missing_values() {
    // Like the Insulin column of the Pima diabetes data: many zeros, then 14 and up.
    let mut insulin: Vec<f64> = vec![0.0; 40];
    insulin.extend((0..60).map(|i| 14.0 + f64::from(i) * 4.0));
    let columns = [column("Insulin", ColumnType::Integer)];

    let (_, insights) = analyze(&columns, &numbers(insulin), false, Basis::WholeFile);

    assert!(
        kinds(&insights).iter().any(
            |kind| matches!(kind, InsightKind::ZerosAsMissing { ratio } if close(*ratio, 0.4))
        ),
        "{insights:?}"
    );
}

#[test]
fn zeros_next_to_small_counts_are_ordinary_values() {
    // Like Pregnancies: 0, 1, 2, ... are all real counts.
    let pregnancies = (0..100).map(|i| f64::from(i % 15));
    let columns = [column("Pregnancies", ColumnType::Integer)];

    let (_, insights) = analyze(&columns, &numbers(pregnancies), false, Basis::WholeFile);

    assert!(
        !kinds(&insights)
            .iter()
            .any(|kind| matches!(kind, InsightKind::ZerosAsMissing { .. })),
        "{insights:?}"
    );
}

#[test]
fn values_far_outside_the_quartiles_are_outliers() {
    let mut values: Vec<f64> = (1..=60).map(f64::from).collect();
    values.push(1000.0);
    let columns = [column("price", ColumnType::Integer)];

    let (_, insights) = analyze(&columns, &numbers(values), false, Basis::WholeFile);

    assert!(
        kinds(&insights)
            .iter()
            .any(|kind| matches!(kind, InsightKind::Outliers { count: 1, .. })),
        "{insights:?}"
    );
}

#[test]
fn evenly_spread_values_have_no_outliers() {
    let columns = [column("n", ColumnType::Integer)];

    let (_, insights) = analyze(
        &columns,
        &numbers((1..=100).map(f64::from)),
        false,
        Basis::WholeFile,
    );

    assert!(insights.is_empty(), "{insights:?}");
}

#[test]
fn a_few_values_with_one_nearly_everywhere_is_an_imbalanced_class() {
    let mut values = vec!["no"; 95];
    values.extend(["yes"; 5]);
    let columns = [column("churn", ColumnType::Text)];

    let (_, insights) = analyze(&columns, &texts(&values), false, Basis::WholeFile);

    assert_eq!(
        kinds(&insights),
        [&InsightKind::Imbalanced {
            value: "no".into(),
            ratio: 0.95
        }]
    );
}

#[test]
fn a_balanced_class_gives_no_warning() {
    let mut values = vec!["no"; 60];
    values.extend(["yes"; 40]);
    let columns = [column("churn", ColumnType::Text)];

    let (_, insights) = analyze(&columns, &texts(&values), false, Basis::WholeFile);

    assert!(insights.is_empty(), "{insights:?}");
}

#[test]
fn repeated_rows_are_reported_for_the_table() {
    let columns = [column("a", ColumnType::Text), column("b", ColumnType::Text)];
    let row = |a: &str, b: &str| vec![Some(a.to_string()), Some(b.to_string())];
    let rows = vec![row("x", "1"), row("y", "2"), row("x", "1"), row("x", "1")];

    let (analysis, insights) = analyze(&columns, &rows, false, Basis::WholeFile);

    assert_eq!(analysis.duplicates, 2);
    let duplicate = insights
        .iter()
        .find(|insight| matches!(insight.kind, InsightKind::DuplicateRows { .. }))
        .expect("duplicate rows insight");
    assert_eq!(duplicate.column, "");
    assert_eq!(
        duplicate.kind,
        InsightKind::DuplicateRows {
            count: 2,
            of_rows: 4
        }
    );
}

#[test]
fn a_text_column_of_numbers_names_the_values_that_are_not_numbers() {
    let mut values: Vec<String> = (20..39).map(|n| n.to_string()).collect();
    values.push("yok".into());
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    let columns = [column("age", ColumnType::Text)];

    let (_, insights) = analyze(&columns, &texts(&refs), false, Basis::WholeFile);

    assert_eq!(
        kinds(&insights),
        [&InsightKind::NumbersAsText {
            ratio: 0.95,
            examples: vec!["yok".into()]
        }]
    );
}

#[test]
fn one_category_written_two_ways_is_reported_with_turkish_case_rules() {
    let columns = [column("city", ColumnType::Text)];
    let rows = texts(&[
        "İstanbul",
        "istanbul",
        "İstanbul ",
        "IŞIK",
        "ışık",
        "Van",
        "Van",
        "Van",
    ]);

    let (_, insights) = analyze(&columns, &rows, false, Basis::WholeFile);

    let variants: Vec<&Vec<String>> = insights
        .iter()
        .filter_map(|insight| match &insight.kind {
            InsightKind::InconsistentSpelling { variants } => Some(variants),
            _ => None,
        })
        .collect();
    assert_eq!(
        variants,
        // In byte order, so the list does not change between runs.
        [&vec![
            "istanbul".to_string(),
            "İstanbul".to_string(),
            "İstanbul ".to_string()
        ]]
    );
}

#[test]
fn a_value_almost_everywhere_among_many_is_dominant() {
    let mut values = vec!["a".to_string(); 300];
    values.extend((0..11).map(|n| format!("other {n}")));
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    let columns = [column("code", ColumnType::Text)];

    let (_, insights) = analyze(&columns, &texts(&refs), false, Basis::WholeFile);

    assert!(
        kinds(&insights)
            .iter()
            .any(|kind| matches!(kind, InsightKind::DominantValue { value, .. } if value == "a")),
        "{insights:?}"
    );
}

#[test]
fn nearly_identical_numeric_columns_are_reported_once() {
    let columns = [
        column("cm", ColumnType::Float),
        column("inch", ColumnType::Float),
        column("noise", ColumnType::Integer),
    ];
    let rows: Vec<Vec<Option<String>>> = (1..=50)
        .map(|i| {
            let cm = f64::from(i) * 2.54;
            vec![
                Some(cm.to_string()),
                Some(f64::from(i).to_string()),
                Some(((i * 37) % 11).to_string()),
            ]
        })
        .collect();

    let (_, insights) = analyze(&columns, &rows, false, Basis::WholeFile);

    let found: Vec<(&str, &InsightKind)> = insights
        .iter()
        .filter(|insight| matches!(insight.kind, InsightKind::HighCorrelation { .. }))
        .map(|insight| (insight.column.as_str(), &insight.kind))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        matches!(found[0], ("cm", InsightKind::HighCorrelation { other, r }) if other == "inch" && close(*r, 1.0))
    );
}

#[test]
fn insights_carry_the_basis_of_the_sample() {
    let mut values = vec!["no"; 95];
    values.extend(["yes"; 5]);
    let columns = [column("churn", ColumnType::Text)];

    let (_, insights) = analyze(&columns, &texts(&values), false, Basis::Sample);

    assert_eq!(insights[0].basis, Basis::Sample);
}

#[test]
fn missing_values_are_left_out_of_every_measure() {
    let columns = [column("n", ColumnType::Integer)];
    let rows = one_column(&[Some("1"), None, Some("3")]);

    let (analysis, _) = analyze(&columns, &rows, false, Basis::WholeFile);

    let stats = analysis.stats[0].expect("stats");
    assert_eq!((stats.count, stats.mean), (2, 2.0));
}
