//! What the row sample of a table shows beyond each column on its own: statistics,
//! categories, correlations, missing cells, repeated rows, and the warnings they lead to.
//! Every number here comes from the sample rows a reader passes in.

use std::collections::{HashMap, HashSet};

use crate::summary::{
    Analysis, Basis, ColumnStats, ColumnSummary, ColumnType, Correlation, Insight, InsightKind,
    MissingMap,
};

/// Most frequent values kept per text column.
pub const CATEGORIES: usize = 10;
/// Numeric columns in the correlation matrix; the first ones are taken.
pub const MAX_CORRELATED_COLUMNS: usize = 30;
/// Rows drawn in the missing value map.
pub const MISSING_MAP_ROWS: usize = 200;

/// Values a column needs before a value-based warning is trusted.
pub const MIN_VALUES: usize = 30;
/// Zeros must be at least this share of the values to look like missing values.
pub const ZERO_MIN_RATIO: f64 = 0.01;
/// ... and the gap from zero to the smallest other value this many times wider than the
/// usual gap between neighbouring values.
pub const ZERO_GAP_FACTOR: f64 = 3.0;
/// Distinct non-zero values needed to know the usual gap.
pub const ZERO_MIN_DISTINCT: usize = 10;
/// Interquartile ranges beyond the quartiles from which a value is an outlier.
pub const OUTLIER_IQR: f64 = 1.5;
/// Share of outliers from which they are reported.
pub const OUTLIER_MIN_RATIO: f64 = 0.01;
/// Distinct values needed before outliers mean something; fewer are codes.
pub const OUTLIER_MIN_DISTINCT: usize = 10;
/// A column with at most this many values is treated as a class.
pub const CLASS_MAX_DISTINCT: usize = 10;
/// Share of the most frequent class from which the column is imbalanced.
pub const IMBALANCE_RATIO: f64 = 0.9;
/// Share of one value from which a column with many values is dominated by it.
pub const DOMINANT_RATIO: f64 = 0.95;
/// Share of numbers from which a text column is reported as numbers stored as text.
pub const NUMBERS_AS_TEXT_RATIO: f64 = 0.9;
/// Values needed before "numbers stored as text" is trusted.
pub const NUMBERS_AS_TEXT_MIN_VALUES: usize = 10;
/// Examples given of values that are not numbers.
pub const NUMBERS_AS_TEXT_EXAMPLES: usize = 3;
/// Variants listed for a category written in more than one way.
pub const SPELLING_VARIANTS: usize = 3;
/// Absolute correlation from which two columns are reported.
pub const HIGH_CORRELATION: f64 = 0.95;

pub fn analyze(
    columns: &[ColumnSummary],
    rows: &[Vec<Option<String>>],
    decimal_comma: bool,
    basis: Basis,
) -> (Analysis, Vec<Insight>) {
    let parse = |value: &str| parse_number(value, decimal_comma);
    // As written, with spaces at the ends: they matter to the spelling check only.
    let written: Vec<Vec<&str>> = (0..columns.len())
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.get(index)?.as_deref())
                .filter(|value| !value.trim().is_empty())
                .collect()
        })
        .collect();
    let values: Vec<Vec<&str>> = written
        .iter()
        .map(|column| column.iter().map(|value| value.trim()).collect())
        .collect();
    let numbers: Vec<Option<Vec<f64>>> = columns
        .iter()
        .zip(&values)
        .map(|(column, values)| {
            is_numeric(column.column_type)
                .then(|| values.iter().filter_map(|value| parse(value)).collect())
        })
        .collect();

    let mut insights = Vec::new();
    let duplicates = duplicate_rows(rows);
    // With one column, repeated rows are just repeated values.
    if duplicates > 0 && columns.len() > 1 {
        insights.push(Insight {
            column: String::new(),
            kind: InsightKind::DuplicateRows {
                count: duplicates,
                of_rows: rows.len() as u64,
            },
            basis,
        });
    }
    let correlation = correlation(columns, rows, &parse);
    for (index, column) in columns.iter().enumerate() {
        let found = column_insights(
            column,
            &values[index],
            &written[index],
            numbers[index].as_deref(),
            &parse,
        )
        .into_iter()
        .chain(correlated_with(index, columns, correlation.as_ref()));
        insights.extend(found.map(|kind| Insight {
            column: column.name.clone(),
            kind,
            basis,
        }));
    }

    let analysis = Analysis {
        rows: rows.len() as u64,
        stats: numbers
            .iter()
            .map(|numbers| numbers.as_deref().and_then(stats))
            .collect(),
        categories: columns
            .iter()
            .zip(&values)
            .map(|(column, values)| match column.column_type {
                ColumnType::Text | ColumnType::Boolean => most_frequent(values, CATEGORIES),
                _ => Vec::new(),
            })
            .collect(),
        correlation,
        missing: missing_map(columns.len(), rows),
        duplicates,
    };
    (analysis, insights)
}

fn is_numeric(column_type: ColumnType) -> bool {
    matches!(
        column_type,
        ColumnType::Integer | ColumnType::Float | ColumnType::Decimal
    )
}

fn parse_number(value: &str, decimal_comma: bool) -> Option<f64> {
    let number: f64 = if decimal_comma {
        value.trim().replacen(',', ".", 1).parse().ok()?
    } else {
        value.trim().parse().ok()?
    };
    number.is_finite().then_some(number)
}

fn column_insights(
    column: &ColumnSummary,
    values: &[&str],
    written: &[&str],
    numbers: Option<&[f64]>,
    parse: &dyn Fn(&str) -> Option<f64>,
) -> Vec<InsightKind> {
    let counts = value_counts(values);
    [
        numbers.and_then(zeros_as_missing),
        numbers.and_then(outliers),
        class_share(&counts, values.len()),
        numbers_as_text(column.column_type, values, parse),
        inconsistent_spelling(column.column_type, &value_counts(written)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn value_counts<'a>(values: &[&'a str]) -> HashMap<&'a str, u64> {
    let mut counts = HashMap::new();
    for &value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    counts
}

/// Most frequent first; equal counts in text order, so the result does not change.
fn most_frequent(values: &[&str], keep: usize) -> Vec<(String, u64)> {
    let mut sorted: Vec<(&str, u64)> = value_counts(values).into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    sorted
        .into_iter()
        .take(keep)
        .map(|(value, count)| (value.to_string(), count))
        .collect()
}

fn stats(numbers: &[f64]) -> Option<ColumnStats> {
    if numbers.is_empty() {
        return None;
    }
    let mut sorted = numbers.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len() as f64;
    let mean = sorted.iter().sum::<f64>() / n;
    let std = if sorted.len() > 1 {
        (sorted.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        f64::NAN
    };
    Some(ColumnStats {
        count: sorted.len() as u64,
        mean,
        std,
        min: sorted[0],
        q1: quantile(&sorted, 0.25),
        median: quantile(&sorted, 0.5),
        q3: quantile(&sorted, 0.75),
        max: sorted[sorted.len() - 1],
    })
}

/// Linear interpolation between the closest ranks, as pandas and NumPy do by default.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let position = q * (sorted.len() - 1) as f64;
    let below = position.floor() as usize;
    let above = position.ceil() as usize;
    sorted[below] + (sorted[above] - sorted[below]) * (position - below as f64)
}

fn zeros_as_missing(numbers: &[f64]) -> Option<InsightKind> {
    if numbers.len() < MIN_VALUES {
        return None;
    }
    let zeros = numbers.iter().filter(|&&x| x == 0.0).count();
    let ratio = zeros as f64 / numbers.len() as f64;
    let mut others: Vec<f64> = numbers.iter().copied().filter(|&x| x != 0.0).collect();
    if ratio < ZERO_MIN_RATIO || others.iter().any(|&x| x < 0.0) {
        return None;
    }
    others.sort_by(f64::total_cmp);
    others.dedup();
    if others.len() < ZERO_MIN_DISTINCT {
        return None;
    }
    let smallest = others[0];
    let usual_gap = (others[others.len() - 1] - smallest) / (others.len() - 1) as f64;
    (smallest > ZERO_GAP_FACTOR * usual_gap).then_some(InsightKind::ZerosAsMissing { ratio })
}

fn outliers(numbers: &[f64]) -> Option<InsightKind> {
    if numbers.len() < MIN_VALUES {
        return None;
    }
    let mut sorted = numbers.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut distinct = sorted.clone();
    distinct.dedup();
    if distinct.len() < OUTLIER_MIN_DISTINCT {
        return None;
    }
    let (q1, q3) = (quantile(&sorted, 0.25), quantile(&sorted, 0.75));
    let iqr = q3 - q1;
    if iqr <= 0.0 {
        return None;
    }
    let (low, high) = (q1 - OUTLIER_IQR * iqr, q3 + OUTLIER_IQR * iqr);
    let count = sorted.iter().filter(|&&x| x < low || x > high).count();
    let ratio = count as f64 / sorted.len() as f64;
    (count > 0 && ratio >= OUTLIER_MIN_RATIO).then_some(InsightKind::Outliers {
        count: count as u64,
        ratio,
        low,
        high,
    })
}

/// An imbalanced class among a few values, or one value dominating many.
fn class_share(counts: &HashMap<&str, u64>, values: usize) -> Option<InsightKind> {
    if values < MIN_VALUES || counts.len() < 2 {
        return None;
    }
    let (&value, &count) = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
    let ratio = count as f64 / values as f64;
    let value = value.to_string();
    if counts.len() <= CLASS_MAX_DISTINCT {
        (ratio >= IMBALANCE_RATIO).then_some(InsightKind::Imbalanced { value, ratio })
    } else {
        (ratio >= DOMINANT_RATIO).then_some(InsightKind::DominantValue { value, ratio })
    }
}

fn numbers_as_text(
    column_type: ColumnType,
    values: &[&str],
    parse: &dyn Fn(&str) -> Option<f64>,
) -> Option<InsightKind> {
    if column_type != ColumnType::Text || values.len() < NUMBERS_AS_TEXT_MIN_VALUES {
        return None;
    }
    let others: Vec<&str> = values
        .iter()
        .copied()
        .filter(|value| parse(value).is_none())
        .collect();
    let ratio = (values.len() - others.len()) as f64 / values.len() as f64;
    (ratio >= NUMBERS_AS_TEXT_RATIO && !others.is_empty()).then(|| InsightKind::NumbersAsText {
        ratio,
        examples: most_frequent(&others, NUMBERS_AS_TEXT_EXAMPLES)
            .into_iter()
            .map(|(value, _)| value)
            .collect(),
    })
}

/// The largest group of values that differ only in letter case or spaces at the ends.
fn inconsistent_spelling(
    column_type: ColumnType,
    counts: &HashMap<&str, u64>,
) -> Option<InsightKind> {
    if column_type != ColumnType::Text {
        return None;
    }
    let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
    for &value in counts.keys() {
        groups
            .entry(turkish_lowercase(value.trim()))
            .or_default()
            .push(value);
    }
    let mut variants = groups
        .into_values()
        .filter(|group| group.len() > 1)
        .max_by_key(|group| {
            (
                group.iter().map(|value| counts[value]).sum::<u64>(),
                group.len(),
            )
        })?;
    variants.sort_unstable();
    Some(InsightKind::InconsistentSpelling {
        variants: variants
            .into_iter()
            .take(SPELLING_VARIANTS)
            .map(str::to_string)
            .collect(),
    })
}

/// Lowercase with Turkish dotted and dotless i: `I` is `ı`, `İ` is `i`.
fn turkish_lowercase(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            'I' => vec!['ı'],
            'İ' => vec!['i'],
            other => other.to_lowercase().collect(),
        })
        .collect()
}

/// Rows equal to an earlier row.
fn duplicate_rows(rows: &[Vec<Option<String>>]) -> u64 {
    let mut seen = HashSet::with_capacity(rows.len());
    rows.iter().filter(|row| !seen.insert(*row)).count() as u64
}

fn correlation(
    columns: &[ColumnSummary],
    rows: &[Vec<Option<String>>],
    parse: &dyn Fn(&str) -> Option<f64>,
) -> Option<Correlation> {
    let numeric: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| is_numeric(column.column_type))
        .map(|(index, _)| index)
        .take(MAX_CORRELATED_COLUMNS)
        .collect();
    if numeric.len() < 2 {
        return None;
    }
    let cells: Vec<Vec<Option<f64>>> = rows
        .iter()
        .map(|row| {
            numeric
                .iter()
                .map(|&index| row.get(index)?.as_deref().and_then(parse))
                .collect()
        })
        .collect();
    let size = numeric.len();
    let mut values = vec![f64::NAN; size * size];
    for i in 0..size {
        for j in i..size {
            let r = pearson(cells.iter().filter_map(|row| Some((row[i]?, row[j]?))));
            values[i * size + j] = r;
            values[j * size + i] = r;
        }
    }
    Some(Correlation {
        columns: numeric,
        values,
    })
}

/// `NaN` when either side has no spread or there are fewer than two pairs.
fn pearson(pairs: impl Iterator<Item = (f64, f64)>) -> f64 {
    let pairs: Vec<(f64, f64)> = pairs.collect();
    if pairs.len() < 2 {
        return f64::NAN;
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut xy, mut xx, mut yy) = (0.0, 0.0, 0.0);
    for (x, y) in &pairs {
        let (dx, dy) = (x - mean_x, y - mean_y);
        xy += dx * dy;
        xx += dx * dx;
        yy += dy * dy;
    }
    if xx <= 0.0 || yy <= 0.0 {
        return f64::NAN;
    }
    (xy / (xx * yy).sqrt()).clamp(-1.0, 1.0)
}

/// High correlations of column `index` with later columns, so each pair is said once.
fn correlated_with(
    index: usize,
    columns: &[ColumnSummary],
    correlation: Option<&Correlation>,
) -> Vec<InsightKind> {
    let Some(correlation) = correlation else {
        return Vec::new();
    };
    let Some(i) = correlation.columns.iter().position(|&c| c == index) else {
        return Vec::new();
    };
    let size = correlation.columns.len();
    (i + 1..size)
        .filter_map(|j| {
            let r = correlation.values[i * size + j];
            (r.abs() >= HIGH_CORRELATION).then(|| InsightKind::HighCorrelation {
                other: columns[correlation.columns[j]].name.clone(),
                r,
            })
        })
        .collect()
}

fn missing_map(columns: usize, rows: &[Vec<Option<String>>]) -> MissingMap {
    let shown = &rows[..rows.len().min(MISSING_MAP_ROWS)];
    MissingMap {
        rows: shown.len(),
        columns,
        missing: shown
            .iter()
            .flat_map(|row| {
                (0..columns).map(|index| {
                    row.get(index)
                        .and_then(Option::as_deref)
                        .is_none_or(|value| value.trim().is_empty())
                })
            })
            .collect(),
    }
}
