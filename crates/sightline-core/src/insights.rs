//! Rules that turn column summaries into warnings and notes.

use crate::summary::{Basis, ColumnSummary, ColumnType, Insight, InsightKind};

/// Share of missing values from which a column is reported.
pub const HIGH_NULL_RATIO: f64 = 0.2;
/// Values needed before "every value is different" means an ID column.
pub const ID_MIN_VALUES: u64 = 20;
/// Values needed before "every value is the same" is worth saying.
pub const SINGLE_VALUE_MIN_VALUES: u64 = 2;
/// Values needed before skewness is trusted.
pub const SKEW_MIN_VALUES: u64 = 30;
/// Distinct values needed before skewness describes a shape; fewer are codes or small
/// counts, where it says little.
pub const SKEW_MIN_DISTINCT: u64 = 20;
/// Skewness beyond this, in either direction, is reported.
pub const SKEW_LIMIT: f64 = 2.0;

/// Insights for the columns, in column order.
pub fn insights(columns: &[ColumnSummary]) -> Vec<Insight> {
    columns.iter().flat_map(column_insights).collect()
}

fn column_insights(column: &ColumnSummary) -> Vec<Insight> {
    [
        high_nulls(column),
        looks_like_id(column),
        single_value(column),
        skewed(column),
        date_range(column),
    ]
    .into_iter()
    .flatten()
    .map(|(kind, basis)| Insight {
        column: column.name.clone(),
        kind,
        basis,
    })
    .collect()
}

fn high_nulls(column: &ColumnSummary) -> Option<(InsightKind, Basis)> {
    let nulls = column.nulls?;
    let ratio = nulls.ratio();
    (nulls.of_rows > 0 && ratio >= HIGH_NULL_RATIO)
        .then_some((InsightKind::HighNulls { ratio }, nulls.basis))
}

fn looks_like_id(column: &ColumnSummary) -> Option<(InsightKind, Basis)> {
    let profile = column.profile.as_ref()?;
    let id_type = matches!(column.column_type, ColumnType::Integer | ColumnType::Text);
    (id_type && profile.values >= ID_MIN_VALUES && profile.distinct == profile.values)
        .then_some((InsightKind::LooksLikeId, profile.basis))
}

fn single_value(column: &ColumnSummary) -> Option<(InsightKind, Basis)> {
    let profile = column.profile.as_ref()?;
    let value = profile.single_value.clone()?;
    (profile.values >= SINGLE_VALUE_MIN_VALUES)
        .then_some((InsightKind::SingleValue { value }, profile.basis))
}

fn skewed(column: &ColumnSummary) -> Option<(InsightKind, Basis)> {
    let profile = column.profile.as_ref()?;
    let skewness = profile.skewness?;
    (profile.values >= SKEW_MIN_VALUES
        && profile.distinct >= SKEW_MIN_DISTINCT
        && skewness.abs() > SKEW_LIMIT)
        .then_some((InsightKind::Skewed { skewness }, profile.basis))
}

fn date_range(column: &ColumnSummary) -> Option<(InsightKind, Basis)> {
    if !matches!(column.column_type, ColumnType::Date | ColumnType::DateTime) {
        return None;
    }
    let bounds = column.bounds.as_ref()?;
    Some((
        InsightKind::DateRange {
            from: bounds.min.clone(),
            to: bounds.max.clone(),
        },
        Basis::WholeFile,
    ))
}
