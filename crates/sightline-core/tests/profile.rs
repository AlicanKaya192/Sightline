use sightline_core::{Basis, ColumnProfile, ColumnType, Shape, profile};

fn number(value: &str) -> Option<f64> {
    value.parse().ok()
}

fn decimal_comma(value: &str) -> Option<f64> {
    value.replacen(',', ".", 1).parse().ok()
}

fn of(values: &[&str], column_type: ColumnType) -> ColumnProfile {
    profile(values, column_type, Basis::WholeFile, &number)
}

fn histogram(profile: &ColumnProfile) -> &[u64] {
    match &profile.shape {
        Shape::Histogram(bins) => bins,
        other => panic!("expected a histogram, got {other:?}"),
    }
}

#[test]
fn empty_column_has_nothing_to_show() {
    let profile = of(&[], ColumnType::Float);

    assert_eq!(
        profile,
        ColumnProfile {
            values: 0,
            distinct: 0,
            basis: Basis::WholeFile,
            shape: Shape::None,
            skewness: None,
            single_value: None,
        }
    );
}

#[test]
fn counts_values_and_distinct_values() {
    let profile = of(&["a", "b", "a"], ColumnType::Text);

    assert_eq!((profile.values, profile.distinct), (3, 2));
}

#[test]
fn keeps_the_basis() {
    let profile = profile(&["a"], ColumnType::Text, Basis::Sample, &number);

    assert_eq!(profile.basis, Basis::Sample);
}

#[test]
fn text_shows_most_frequent_values_first_with_ties_in_text_order() {
    let profile = of(&["x", "y", "y", "z", "y", "x", "w"], ColumnType::Text);

    assert_eq!(
        profile.shape,
        Shape::TopValues(vec![("y".into(), 3), ("x".into(), 2), ("w".into(), 1)])
    );
}

#[test]
fn booleans_show_top_values() {
    let profile = of(&["true", "false", "true"], ColumnType::Boolean);

    assert_eq!(
        profile.shape,
        Shape::TopValues(vec![("true".into(), 2), ("false".into(), 1)])
    );
}

#[test]
fn numbers_fall_into_equal_width_bins() {
    let values: Vec<String> = (0..12).map(|n| n.to_string()).collect();
    let values: Vec<&str> = values.iter().map(String::as_str).collect();

    let profile = of(&values, ColumnType::Integer);

    assert_eq!(histogram(&profile), [1; 12]);
}

#[test]
fn a_single_number_fills_the_first_bin() {
    let profile = of(&["5", "5"], ColumnType::Integer);

    let mut expected = [0; 12];
    expected[0] = 2;
    assert_eq!(histogram(&profile), expected);
}

#[test]
fn histogram_skips_values_that_are_not_finite() {
    let profile = of(&["1", "NaN", "inf", "3"], ColumnType::Float);

    assert_eq!(histogram(&profile).iter().sum::<u64>(), 2);
    assert_eq!(profile.values, 4);
}

#[test]
fn numbers_are_read_with_the_given_parser() {
    let profile = profile(
        &["1,5", "2,5"],
        ColumnType::Float,
        Basis::WholeFile,
        &decimal_comma,
    );

    assert_eq!(histogram(&profile).iter().sum::<u64>(), 2);
}

#[test]
fn dates_fall_into_bins_in_time_order() {
    let profile = of(&["2026-09-12", "2026-09-01"], ColumnType::Date);

    let bins = histogram(&profile);
    assert_eq!((bins[0], bins[11]), (1, 1));
}

#[test]
fn date_times_fall_into_bins_in_time_order() {
    let profile = of(
        &[
            "2026-09-01 10:00",
            "2026-09-01T09:00:30",
            "2026-09-02 08:00",
        ],
        ColumnType::DateTime,
    );

    let bins = histogram(&profile);
    assert_eq!(bins.iter().sum::<u64>(), 3);
    assert_eq!((bins[0], bins[11]), (2, 1));
}

#[test]
fn symmetric_numbers_have_zero_skewness() {
    let profile = of(&["1", "2", "3"], ColumnType::Integer);

    assert_eq!(profile.skewness, Some(0.0));
}

#[test]
fn a_long_right_tail_gives_positive_skewness() {
    let profile = of(&["1", "1", "1", "1", "10"], ColumnType::Integer);

    let skewness = profile.skewness.expect("numeric column has a skewness");
    assert!((skewness - 1.5).abs() < 1e-9, "{skewness}");
}

#[test]
fn text_has_no_skewness() {
    let profile = of(&["a", "b", "c"], ColumnType::Text);

    assert_eq!(profile.skewness, None);
}

#[test]
fn a_constant_column_has_its_single_value_and_no_skewness() {
    let profile = of(&["7", "7", "7"], ColumnType::Integer);

    assert_eq!(profile.single_value.as_deref(), Some("7"));
    assert_eq!(profile.skewness, None);
}

#[test]
fn binary_columns_have_no_shape() {
    let profile = of(&["00ff"], ColumnType::Binary);

    assert_eq!(profile.shape, Shape::None);
}
