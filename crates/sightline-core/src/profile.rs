//! Profiles of sampled column values: distinct count, histogram or top values, skewness.

use std::collections::HashMap;

use crate::summary::{Basis, ColumnProfile, ColumnType, Shape};

pub const HISTOGRAM_BINS: usize = 12;
pub const TOP_VALUES: usize = 3;
/// A spread this small next to the values' size is rounding, not variation.
const RELATIVE_SPREAD_FLOOR: f64 = 1e-12;
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Profiles the non-empty `values` of one column; `number` reads a numeric value.
pub fn profile(
    values: &[&str],
    column_type: ColumnType,
    basis: Basis,
    number: &dyn Fn(&str) -> Option<f64>,
) -> ColumnProfile {
    let counts = value_counts(values);
    let positions = positions(values, column_type, number);
    let skewness = match column_type {
        ColumnType::Integer | ColumnType::Float | ColumnType::Decimal => {
            positions.as_deref().and_then(skewness)
        }
        _ => None,
    };
    ColumnProfile {
        values: values.len() as u64,
        distinct: counts.len() as u64,
        basis,
        shape: shape(column_type, &counts, positions.as_deref()),
        skewness,
        single_value: single_value(&counts),
    }
}

fn value_counts<'a>(values: &[&'a str]) -> HashMap<&'a str, u64> {
    let mut counts = HashMap::new();
    for &value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    counts
}

/// Finite values on a number line: the number itself, or days since 1970 for dates.
/// `None` for columns that have no order on a line.
fn positions(
    values: &[&str],
    column_type: ColumnType,
    number: &dyn Fn(&str) -> Option<f64>,
) -> Option<Vec<f64>> {
    let read: &dyn Fn(&str) -> Option<f64> = match column_type {
        ColumnType::Integer | ColumnType::Float | ColumnType::Decimal => number,
        ColumnType::Date | ColumnType::DateTime => &day_number,
        _ => return None,
    };
    Some(
        values
            .iter()
            .filter_map(|value| read(value))
            .filter(|position| position.is_finite())
            .collect(),
    )
}

fn shape(column_type: ColumnType, counts: &HashMap<&str, u64>, positions: Option<&[f64]>) -> Shape {
    match positions {
        Some([]) => Shape::None,
        Some(positions) => Shape::Histogram(histogram(positions)),
        None if counts.is_empty() => Shape::None,
        None => match column_type {
            ColumnType::Boolean | ColumnType::Text => Shape::TopValues(top_values(counts)),
            _ => Shape::None,
        },
    }
}

pub(crate) fn histogram(positions: &[f64]) -> Vec<u64> {
    let min = positions.iter().copied().fold(f64::INFINITY, f64::min);
    let max = positions.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let width = max - min;
    let mut bins = vec![0; HISTOGRAM_BINS];
    for &position in positions {
        let index = if width > 0.0 {
            ((position - min) / width * HISTOGRAM_BINS as f64) as usize
        } else {
            0
        };
        bins[index.min(HISTOGRAM_BINS - 1)] += 1;
    }
    bins
}

/// Most frequent first; equal counts in text order, so the result does not change between runs.
fn top_values(counts: &HashMap<&str, u64>) -> Vec<(String, u64)> {
    let mut sorted: Vec<(&str, u64)> = counts.iter().map(|(&value, &n)| (value, n)).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    sorted
        .into_iter()
        .take(TOP_VALUES)
        .map(|(value, n)| (value.to_string(), n))
        .collect()
}

/// Fisher-Pearson coefficient; `None` when the values have no spread.
fn skewness(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let moment = |power: i32| {
        values
            .iter()
            .map(|value| (value - mean).powi(power))
            .sum::<f64>()
            / n
    };
    let variance = moment(2);
    if variance.sqrt() <= RELATIVE_SPREAD_FLOOR * mean.abs().max(1.0) {
        return None;
    }
    Some(moment(3) / variance.powf(1.5))
}

fn single_value(counts: &HashMap<&str, u64>) -> Option<String> {
    match counts.keys().collect::<Vec<_>>().as_slice() {
        [only] => Some(only.to_string()),
        _ => None,
    }
}

/// Days since 1970-01-01 for `YYYY-MM-DD`, with the time of day as a fraction when one
/// follows after a space or `T`; anything after the time (such as a zone) is ignored.
fn day_number(text: &str) -> Option<f64> {
    let date = text.get(..10)?;
    let mut parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let (Some(Some(year)), Some(Some(month)), Some(Some(day)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let days = days_from_civil(year, month, day) as f64;
    match text[10..].strip_prefix([' ', 'T']) {
        Some(time) => Some(days + seconds_of_day(time)? / SECONDS_PER_DAY),
        None => Some(days),
    }
}

/// `HH:MM[:SS[.fraction]]` at the start of `text`.
fn seconds_of_day(text: &str) -> Option<f64> {
    let time = text
        .split(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .next()?;
    let mut parts = time.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next().map_or(Some(0.0), |s| s.parse().ok())?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// Days from 1970-01-01 in the proleptic Gregorian calendar (H. Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
