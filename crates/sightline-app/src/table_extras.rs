//! Rows of the Statistics, Correlation, Categories and Missing values tabs, in the
//! window's number style, from the analysis of a table's sample.

use sightline_core::{Analysis, ColumnSummary};
use slint::{ModelRc, SharedString, VecModel};

use crate::view::NumberFormat;
use crate::{CategoryColumn, CategoryValue, CorrelationCell, MissingColumn, StatRow};

/// One row per numeric column.
pub fn stat_rows(
    columns: &[ColumnSummary],
    analysis: &Analysis,
    format: &NumberFormat,
) -> Vec<StatRow> {
    columns
        .iter()
        .zip(&analysis.stats)
        .filter_map(|(column, stats)| {
            let stats = stats.as_ref()?;
            Some(StatRow {
                name: column.name.as_str().into(),
                count: format.count(stats.count).into(),
                mean: format.number(stats.mean).into(),
                std: format.number(stats.std).into(),
                min: format.number(stats.min).into(),
                q1: format.number(stats.q1).into(),
                median: format.number(stats.median).into(),
                q3: format.number(stats.q3).into(),
                max: format.number(stats.max).into(),
            })
        })
        .collect()
}

/// Names of the correlated columns and the matrix cells, row by row.
pub fn correlation(
    columns: &[ColumnSummary],
    analysis: &Analysis,
    format: &NumberFormat,
) -> (Vec<SharedString>, Vec<CorrelationCell>) {
    let Some(correlation) = &analysis.correlation else {
        return (Vec::new(), Vec::new());
    };
    let names = correlation
        .columns
        .iter()
        .map(|&index| columns[index].name.as_str().into())
        .collect();
    let cells = correlation
        .values
        .iter()
        .map(|&r| CorrelationCell {
            text: if r.is_finite() {
                format.decimals(r, 2).into()
            } else {
                "—".into()
            },
            value: if r.is_finite() { r as f32 } else { 0.0 },
            defined: r.is_finite(),
        })
        .collect();
    (names, cells)
}

/// The most frequent values of each column that has categories; columns where no value
/// repeats (names, IDs) have none worth drawing.
pub fn category_columns(
    columns: &[ColumnSummary],
    analysis: &Analysis,
    format: &NumberFormat,
) -> Vec<CategoryColumn> {
    let rows = analysis.rows.max(1) as f64;
    columns
        .iter()
        .zip(&analysis.categories)
        .filter(|(_, values)| values.first().is_some_and(|(_, count)| *count > 1))
        .map(|(column, values)| {
            let largest = values.iter().map(|(_, count)| *count).max().unwrap_or(1) as f32;
            let values: Vec<CategoryValue> = values
                .iter()
                .map(|(text, count)| CategoryValue {
                    text: text.as_str().into(),
                    count: format.count(*count).into(),
                    share: format.one_decimal(*count as f64 / rows * 100.0).into(),
                    ratio: *count as f32 / largest,
                })
                .collect();
            CategoryColumn {
                name: column.name.as_str().into(),
                values: ModelRc::new(VecModel::from(values)),
            }
        })
        .collect()
}

/// Columns of the missing value map with their share of empty cells, the rows drawn
/// and the cells, row by row.
pub fn missing(
    columns: &[ColumnSummary],
    analysis: &Analysis,
    format: &NumberFormat,
) -> (Vec<MissingColumn>, i32, Vec<bool>) {
    let map = &analysis.missing;
    let names = columns
        .iter()
        .take(map.columns)
        .enumerate()
        .map(|(index, column)| {
            let empty = (0..map.rows)
                .filter(|row| map.missing[row * map.columns + index])
                .count();
            MissingColumn {
                name: column.name.as_str().into(),
                percent: format
                    .one_decimal(empty as f64 / map.rows.max(1) as f64 * 100.0)
                    .into(),
            }
        })
        .collect();
    (
        names,
        i32::try_from(map.rows).unwrap_or(i32::MAX),
        map.missing.clone(),
    )
}

#[cfg(test)]
mod tests {
    use sightline_core::{
        Analysis, ColumnStats, ColumnSummary, ColumnType, Correlation, MissingMap,
    };
    use slint::Model;

    use super::*;

    const TURKISH: NumberFormat = NumberFormat {
        decimal: ',',
        thousands: '.',
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

    fn analysis() -> Analysis {
        Analysis {
            rows: 4,
            stats: vec![
                Some(ColumnStats {
                    count: 1200,
                    mean: 2.5,
                    std: f64::NAN,
                    min: -1.0,
                    q1: 1.25,
                    median: 2.0,
                    q3: 3.75,
                    max: 1500.5,
                }),
                None,
            ],
            categories: vec![Vec::new(), vec![("Ağrı".into(), 3), ("Van".into(), 1)]],
            correlation: Some(Correlation {
                columns: vec![0],
                values: vec![1.0],
            }),
            missing: MissingMap {
                rows: 2,
                columns: 2,
                missing: vec![false, true, true, true],
            },
            duplicates: 0,
        }
    }

    fn columns() -> [ColumnSummary; 2] {
        [
            column("n", ColumnType::Float),
            column("city", ColumnType::Text),
        ]
    }

    #[test]
    fn statistics_are_written_in_the_language_style() {
        let rows = stat_rows(&columns(), &analysis(), &TURKISH);

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            [
                row.name.as_str(),
                row.count.as_str(),
                row.mean.as_str(),
                row.std.as_str(),
                row.q1.as_str(),
                row.max.as_str(),
            ],
            ["n", "1.200", "2,5", "—", "1,25", "1.500,5"]
        );
    }

    #[test]
    fn categories_carry_counts_shares_and_bar_lengths() {
        let found = category_columns(&columns(), &analysis(), &TURKISH);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "city");
        let values: Vec<(String, String, String, f32)> = found[0]
            .values
            .iter()
            .map(|v| (v.text.into(), v.count.into(), v.share.into(), v.ratio))
            .collect();
        assert_eq!(
            values,
            [
                ("Ağrı".into(), "3".into(), "75,0".into(), 1.0),
                ("Van".into(), "1".into(), "25,0".into(), 1.0 / 3.0),
            ]
        );
    }

    #[test]
    fn columns_where_no_value_repeats_have_no_categories() {
        let mut analysis = analysis();
        analysis.categories[1] = vec![("Abbott".into(), 1), ("Adams".into(), 1)];

        assert!(category_columns(&columns(), &analysis, &TURKISH).is_empty());
    }

    #[test]
    fn correlation_cells_without_a_value_are_marked() {
        let mut analysis = analysis();
        analysis.correlation = Some(Correlation {
            columns: vec![0, 1],
            values: vec![1.0, f64::NAN, f64::NAN, -0.5],
        });

        let (names, cells) = correlation(&columns(), &analysis, &TURKISH);

        assert_eq!(names, ["n", "city"]);
        assert_eq!(cells[0].text, "1,00");
        assert!(!cells[1].defined);
        assert_eq!(cells[1].text, "—");
        assert_eq!((cells[3].text.as_str(), cells[3].value), ("-0,50", -0.5));
    }

    #[test]
    fn missing_map_gives_each_column_its_share_of_empty_cells() {
        let (names, rows, cells) = missing(&columns(), &analysis(), &TURKISH);

        let shares: Vec<(&str, &str)> = names
            .iter()
            .map(|c| (c.name.as_str(), c.percent.as_str()))
            .collect();
        assert_eq!(shares, [("n", "50,0"), ("city", "100,0")]);
        assert_eq!((rows, cells.len()), (2, 4));
    }
}
