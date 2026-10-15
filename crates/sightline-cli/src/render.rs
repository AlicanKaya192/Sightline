//! Plain text output for a terminal. Developer tool: not translated.

use sightline_core::{
    Basis, Bounds, ColumnSummary, ColumnType, NullCount, RowCount, TableFormat, TableSummary,
    TextEncoding,
};

/// Longer values in the first rows are cut so the table fits a terminal.
const MAX_CELL_CHARS: usize = 32;
const LABEL_WIDTH: usize = 10;
const MISSING: &str = "-";

/// The summary as lines of text, starting with the file name.
pub fn render_table(file_name: &str, summary: &TableSummary) -> String {
    let mut text = format!("{file_name}\n");
    text += &labelled("Format:", &format_text(&summary.format));
    text += &labelled("Rows:", &rows_text(summary.rows));
    text += &labelled("Columns:", &summary.columns.len().to_string());
    text += "\n";
    text += &aligned(&column_rows(&summary.columns));
    text += "\nFirst rows:\n";
    text += &aligned(&head_rows(summary));
    text
}

fn labelled(label: &str, value: &str) -> String {
    format!("{label:<LABEL_WIDTH$}{value}\n")
}

fn format_text(format: &TableFormat) -> String {
    match format {
        TableFormat::Parquet { row_groups } => format!("Parquet, {row_groups} row groups"),
        TableFormat::Csv {
            delimiter,
            encoding,
        } => format!(
            "CSV, delimiter '{}', {}",
            char::from(*delimiter),
            encoding_name(*encoding)
        ),
    }
}

fn encoding_name(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::Utf8 => "UTF-8",
        TextEncoding::Windows1254 => "Windows-1254",
    }
}

fn rows_text(rows: RowCount) -> String {
    match rows {
        RowCount::Exact(count) => count.to_string(),
        RowCount::AtLeast(count) => format!("at least {count}"),
    }
}

fn column_rows(columns: &[ColumnSummary]) -> Vec<Vec<String>> {
    let title = ["column", "type", "nulls", "min", "max"]
        .map(String::from)
        .to_vec();
    let lines = columns.iter().map(|column| {
        let (min, max) = column
            .bounds
            .as_ref()
            .map_or((MISSING, MISSING), |Bounds { min, max }| (min, max));
        vec![
            column.name.clone(),
            type_name(column.column_type).to_string(),
            column.nulls.map_or(MISSING.to_string(), nulls_text),
            min.to_string(),
            max.to_string(),
        ]
    });
    std::iter::once(title).chain(lines).collect()
}

fn type_name(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Boolean => "boolean",
        ColumnType::Integer => "integer",
        ColumnType::Float => "float",
        ColumnType::Decimal => "decimal",
        ColumnType::Date => "date",
        ColumnType::Time => "time",
        ColumnType::DateTime => "datetime",
        ColumnType::Text => "text",
        ColumnType::Binary => "binary",
        ColumnType::Nested => "nested",
    }
}

fn nulls_text(nulls: NullCount) -> String {
    let percent = nulls.ratio() * 100.0;
    match nulls.basis {
        Basis::WholeFile => format!("{percent:.1}%"),
        Basis::Sample => format!("{percent:.1}% (sample of {})", nulls.of_rows),
    }
}

fn head_rows(summary: &TableSummary) -> Vec<Vec<String>> {
    let title = summary.columns.iter().map(|c| shortened(&c.name)).collect();
    let rows = summary.head.iter().map(|row| {
        row.iter()
            .map(|value| value.as_deref().map_or("null".to_string(), shortened))
            .collect()
    });
    std::iter::once(title).chain(rows).collect()
}

fn shortened(value: &str) -> String {
    if value.chars().count() <= MAX_CELL_CHARS {
        return value.to_string();
    }
    let kept: String = value.chars().take(MAX_CELL_CHARS - 1).collect();
    format!("{kept}…")
}

/// Indented rows with every column padded to its widest cell.
fn aligned(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, &width)| format!("{cell:<width$}"))
                .collect();
            format!("  {}\n", cells.join("  ").trim_end())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sightline_core::{
        Basis, Bounds, ColumnSummary, ColumnType, NullCount, RowCount, TableFormat, TextEncoding,
    };

    fn summary(rows: RowCount, basis: Basis) -> TableSummary {
        TableSummary {
            format: TableFormat::Csv {
                delimiter: b';',
                encoding: TextEncoding::Windows1254,
            },
            rows,
            columns: vec![
                ColumnSummary {
                    name: "id".into(),
                    column_type: ColumnType::Integer,
                    nulls: Some(NullCount {
                        count: 1,
                        of_rows: 4,
                        basis,
                    }),
                    bounds: Some(Bounds {
                        min: "1".into(),
                        max: "9".into(),
                    }),
                },
                ColumnSummary {
                    name: "şehir".into(),
                    column_type: ColumnType::Text,
                    nulls: None,
                    bounds: None,
                },
            ],
            head: vec![vec![Some("1".into()), None]],
        }
    }

    fn line_starting_with<'a>(text: &'a str, start: &str) -> &'a str {
        text.lines()
            .find(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("no line starts with {start:?} in:\n{text}"))
    }

    #[test]
    fn starts_with_file_name_and_format() {
        let text = render_table("data.csv", &summary(RowCount::Exact(4), Basis::WholeFile));

        assert!(text.starts_with("data.csv\n"), "{text}");
        assert_eq!(
            line_starting_with(&text, "Format"),
            "Format:   CSV, delimiter ';', Windows-1254"
        );
    }

    #[test]
    fn says_when_the_row_count_is_a_lower_bound() {
        let exact = render_table("a.csv", &summary(RowCount::Exact(4), Basis::WholeFile));
        let partial = render_table("a.csv", &summary(RowCount::AtLeast(4), Basis::Sample));

        assert_eq!(line_starting_with(&exact, "Rows"), "Rows:     4");
        assert_eq!(line_starting_with(&partial, "Rows"), "Rows:     at least 4");
    }

    #[test]
    fn column_line_shows_type_nulls_and_bounds() {
        let text = render_table("a.csv", &summary(RowCount::AtLeast(4), Basis::Sample));

        let id = line_starting_with(&text, "  id ");
        let words: Vec<&str> = id.split_whitespace().collect();
        assert_eq!(
            words,
            ["id", "integer", "25.0%", "(sample", "of", "4)", "1", "9"]
        );
    }

    #[test]
    fn unknown_values_are_shown_as_dashes() {
        let text = render_table("a.csv", &summary(RowCount::Exact(4), Basis::WholeFile));

        let city = line_starting_with(&text, "  şehir ");
        let words: Vec<&str> = city.split_whitespace().collect();
        assert_eq!(words, ["şehir", "text", "-", "-", "-"]);
    }

    #[test]
    fn first_rows_show_missing_values_as_null() {
        let text = render_table("a.csv", &summary(RowCount::Exact(4), Basis::WholeFile));

        let after_title = text.split("First rows:\n").nth(1).expect("head section");
        let rows: Vec<Vec<&str>> = after_title
            .lines()
            .map(|line| line.split_whitespace().collect())
            .collect();
        assert_eq!(rows, [vec!["id", "şehir"], vec!["1", "null"]]);
    }
}
