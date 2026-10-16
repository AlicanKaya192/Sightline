use sightline_core::{
    Basis, Bounds, ColumnProfile, ColumnSummary, ColumnType, Insight, InsightKind, NullCount,
    Shape, insights,
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

fn with_nulls(mut column: ColumnSummary, count: u64, of_rows: u64) -> ColumnSummary {
    column.nulls = Some(NullCount {
        count,
        of_rows,
        basis: Basis::WholeFile,
    });
    column
}

fn with_profile(mut column: ColumnSummary, values: u64, distinct: u64) -> ColumnSummary {
    column.profile = Some(ColumnProfile {
        values,
        distinct,
        basis: Basis::WholeFile,
        shape: Shape::None,
        skewness: None,
        single_value: None,
    });
    column
}

fn with_skewness(column: ColumnSummary, values: u64, skewness: f64) -> ColumnSummary {
    let mut column = with_profile(column, values, values);
    column.profile.as_mut().expect("profile").skewness = Some(skewness);
    column
}

fn with_single_value(column: ColumnSummary, values: u64) -> ColumnSummary {
    let mut column = with_profile(column, values, 1);
    column.profile.as_mut().expect("profile").single_value = Some("TR".into());
    column
}

fn kinds(columns: &[ColumnSummary]) -> Vec<InsightKind> {
    insights(columns).into_iter().map(|i| i.kind).collect()
}

#[test]
fn a_plain_column_has_no_insights() {
    let plain = with_profile(with_nulls(column("a", ColumnType::Float), 1, 10), 9, 5);

    assert_eq!(insights(&[plain]), []);
}

#[test]
fn twenty_percent_missing_is_high() {
    let at_limit = with_nulls(column("a", ColumnType::Text), 20, 100);
    let below = with_nulls(column("b", ColumnType::Text), 19, 100);

    assert_eq!(kinds(&[at_limit]), [InsightKind::HighNulls { ratio: 0.2 }]);
    assert_eq!(kinds(&[below]), []);
}

#[test]
fn no_rows_means_no_null_warning() {
    let empty = with_nulls(column("a", ColumnType::Text), 0, 0);

    assert_eq!(kinds(&[empty]), []);
}

#[test]
fn twenty_different_integers_look_like_an_id() {
    let id = with_profile(column("id", ColumnType::Integer), 20, 20);

    assert_eq!(kinds(&[id]), [InsightKind::LooksLikeId]);
}

#[test]
fn different_text_values_look_like_an_id() {
    let code = with_profile(column("code", ColumnType::Text), 50, 50);

    assert_eq!(kinds(&[code]), [InsightKind::LooksLikeId]);
}

#[test]
fn too_few_or_repeated_values_are_not_an_id() {
    let few = with_profile(column("a", ColumnType::Integer), 19, 19);
    let repeated = with_profile(column("b", ColumnType::Integer), 20, 19);

    assert_eq!(kinds(&[few, repeated]), []);
}

#[test]
fn different_floats_are_not_an_id() {
    let measured = with_profile(column("a", ColumnType::Float), 100, 100);

    assert_eq!(kinds(&[measured]), []);
}

#[test]
fn the_same_value_everywhere_is_reported() {
    let country = with_single_value(column("country", ColumnType::Text), 2);

    assert_eq!(
        kinds(&[country]),
        [InsightKind::SingleValue { value: "TR".into() }]
    );
}

#[test]
fn one_value_alone_is_not_a_single_value_column() {
    let lonely = with_single_value(column("a", ColumnType::Text), 1);

    assert_eq!(kinds(&[lonely]), []);
}

#[test]
fn strong_skew_in_either_direction_is_reported() {
    let right = with_skewness(column("a", ColumnType::Float), 30, 2.5);
    let left = with_skewness(column("b", ColumnType::Float), 30, -2.5);

    assert_eq!(
        kinds(&[right, left]),
        [
            InsightKind::Skewed { skewness: 2.5 },
            InsightKind::Skewed { skewness: -2.5 }
        ]
    );
}

#[test]
fn mild_skew_or_few_values_are_not_reported() {
    let mild = with_skewness(column("a", ColumnType::Float), 30, 2.0);
    let few = with_skewness(column("b", ColumnType::Float), 29, 5.0);

    assert_eq!(kinds(&[mild, few]), []);
}

#[test]
fn skew_of_a_few_codes_is_not_reported() {
    let mut codes = with_skewness(column("a", ColumnType::Integer), 1_000, 10.9);
    codes.profile.as_mut().expect("profile").distinct = 19;
    let mut amounts = with_skewness(column("b", ColumnType::Float), 1_000, 3.3);
    amounts.profile.as_mut().expect("profile").distinct = 20;

    assert_eq!(
        kinds(&[codes, amounts]),
        [InsightKind::Skewed { skewness: 3.3 }]
    );
}

#[test]
fn date_columns_report_their_range() {
    let mut day = column("day", ColumnType::Date);
    day.bounds = Some(Bounds {
        min: "2026-09-01".into(),
        max: "2026-09-12".into(),
    });

    assert_eq!(
        kinds(&[day]),
        [InsightKind::DateRange {
            from: "2026-09-01".into(),
            to: "2026-09-12".into()
        }]
    );
}

#[test]
fn numbers_with_bounds_report_no_range() {
    let mut amount = column("amount", ColumnType::Integer);
    amount.bounds = Some(Bounds {
        min: "1".into(),
        max: "9".into(),
    });

    assert_eq!(kinds(&[amount]), []);
}

#[test]
fn insights_follow_column_order_and_name_the_column() {
    let first = with_nulls(column("first", ColumnType::Text), 5, 10);
    let second = with_profile(
        with_nulls(column("second", ColumnType::Integer), 3, 10),
        20,
        20,
    );

    let found = insights(&[first, second]);

    let names: Vec<&str> = found.iter().map(|i| i.column.as_str()).collect();
    assert_eq!(names, ["first", "second", "second"]);
    assert_eq!(found[2].kind, InsightKind::LooksLikeId);
}

#[test]
fn an_insight_from_a_sample_says_so() {
    let mut id = with_profile(column("id", ColumnType::Integer), 20, 20);
    id.profile.as_mut().expect("profile").basis = Basis::Sample;

    assert_eq!(
        insights(&[id]),
        [Insight {
            column: "id".into(),
            kind: InsightKind::LooksLikeId,
            basis: Basis::Sample,
        }]
    );
}
