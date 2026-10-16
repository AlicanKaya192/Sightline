//! The shown summary as plain text for the clipboard, in the window's language. It is built
//! from the rows the window already shows, so the copy says exactly what the window says.

use slint::{ComponentHandle, Model};

use crate::{
    AppWindow, ArrayInfo, DtypeRow, FileInfo, ImportRow, ModelInfo, PickleInfo, SchemaRow,
    ViewState, Words,
};

/// Words of the copied text, already translated. The two patterns hold `{}` for a value.
#[derive(Debug, Clone, Default)]
pub struct Labels {
    pub rows: String,
    pub columns: String,
    pub size: String,
    pub schema: String,
    pub warnings: String,
    pub missing: String,
    pub distinct: String,
    pub parameters: String,
    pub tensors: String,
    pub parameters_by_type: String,
    pub shape: String,
    pub dtype: String,
    pub min: String,
    pub max: String,
    pub nan: String,
    pub infinite: String,
    pub protocol: String,
    pub imports: String,
    pub calls: String,
    pub dangerous: String,
    pub at_least: String,
    pub percent: String,
}

/// One schema row with its type name in the window's language.
pub struct Column<'a> {
    pub row: &'a SchemaRow,
    pub kind: String,
}

/// The summary the window shows, or `None` when it shows no file.
pub fn shown(window: &AppWindow) -> Option<String> {
    let words = window.global::<Words>();
    let labels = labels(&words);
    let warnings: Vec<String> = window
        .get_warnings()
        .iter()
        .map(|row| words.invoke_warning(row).into())
        .collect();
    match window.get_state() {
        ViewState::Loaded => {
            let rows: Vec<SchemaRow> = window.get_schema().iter().collect();
            let columns: Vec<Column> = rows
                .iter()
                .map(|row| Column {
                    row,
                    kind: words.invoke_kind(row.kind).into(),
                })
                .collect();
            Some(table(&window.get_info(), &columns, &warnings, &labels))
        }
        ViewState::Model => {
            let dtypes: Vec<DtypeRow> = window.get_dtype_rows().iter().collect();
            Some(model(&window.get_model_info(), &dtypes, &warnings, &labels))
        }
        ViewState::Array => Some(array(&window.get_array_info(), &warnings, &labels)),
        ViewState::Pickle => {
            let imports: Vec<ImportRow> = window.get_import_rows().iter().collect();
            Some(pickle(
                &window.get_pickle_info(),
                &imports,
                &warnings,
                &labels,
            ))
        }
        _ => None,
    }
}

fn labels(words: &Words) -> Labels {
    Labels {
        rows: words.get_rows().into(),
        columns: words.get_columns().into(),
        size: words.get_size().into(),
        schema: words.get_schema().into(),
        warnings: words.get_warnings().into(),
        missing: words.get_missing().into(),
        distinct: words.get_distinct().into(),
        parameters: words.get_parameters().into(),
        tensors: words.get_tensors().into(),
        parameters_by_type: words.get_parameters_by_type().into(),
        shape: words.get_shape().into(),
        dtype: words.get_dtype().into(),
        min: words.get_min().into(),
        max: words.get_max().into(),
        nan: words.get_nan().into(),
        infinite: words.get_infinite().into(),
        protocol: words.get_protocol().into(),
        imports: words.get_imports().into(),
        calls: words.get_calls().into(),
        dangerous: words.get_dangerous().into(),
        at_least: words.invoke_at_least("{}".into()).into(),
        percent: words.invoke_percent("{}".into()).into(),
    }
}

pub fn table(info: &FileInfo, columns: &[Column], warnings: &[String], labels: &Labels) -> String {
    let rows = if info.rows_exact {
        info.rows.to_string()
    } else {
        fill(&labels.at_least, &info.rows)
    };
    let mut text = format!(
        "{}
",
        info.name
    );
    text += &fact(&labels.rows, &rows);
    text += &fact(&labels.columns, &info.columns.to_string());
    text += &fact(&labels.size, &info.size);
    let lines: Vec<String> = columns
        .iter()
        .map(|column| column_line(column, labels))
        .collect();
    text += &section(&labels.schema, &lines);
    text += &section(&labels.warnings, warnings);
    text
}

pub fn model(
    info: &ModelInfo,
    dtypes: &[DtypeRow],
    warnings: &[String],
    labels: &Labels,
) -> String {
    let mut text = format!(
        "{}
",
        info.name
    );
    if info.parameters_known {
        text += &fact(&labels.parameters, &info.parameters);
    }
    text += &fact(&labels.tensors, &info.tensors.to_string());
    text += &fact(&labels.size, &info.size);
    let lines: Vec<String> = dtypes
        .iter()
        .map(|row| {
            let percent = fill(&labels.percent, &row.percent);
            format!("{}: {} ({percent})", row.name, row.count)
        })
        .collect();
    text += &section(&labels.parameters_by_type, &lines);
    text += &section(&labels.warnings, warnings);
    text
}

pub fn array(info: &ArrayInfo, warnings: &[String], labels: &Labels) -> String {
    let mut text = format!(
        "{}
",
        info.name
    );
    text += &fact(&labels.shape, &info.shape);
    text += &fact(&labels.dtype, &info.descr);
    text += &fact(&labels.size, &info.size);
    if info.has_stats {
        text += &fact(&labels.min, &info.min);
        text += &fact(&labels.max, &info.max);
        text += &fact(&labels.nan, &info.nan);
        text += &fact(&labels.infinite, &info.infinite);
    }
    text += &section(&labels.warnings, warnings);
    text
}

pub fn pickle(
    info: &PickleInfo,
    imports: &[ImportRow],
    warnings: &[String],
    labels: &Labels,
) -> String {
    let mut text = format!(
        "{}
",
        info.name
    );
    if info.protocol_known {
        text += &fact(&labels.protocol, &info.protocol);
    }
    text += &fact(&labels.calls, &info.calls);
    text += &fact(&labels.size, &info.size);
    let lines: Vec<String> = imports
        .iter()
        .map(|import| {
            let name = format!("{}.{}", import.module, import.name);
            if import.dangerous {
                format!("{name} ({})", labels.dangerous)
            } else {
                name
            }
        })
        .collect();
    text += &section(&labels.imports, &lines);
    text += &section(&labels.warnings, warnings);
    text
}

fn column_line(column: &Column, labels: &Labels) -> String {
    let row = column.row;
    let mut parts = vec![format!("{}: {}", row.name, column.kind)];
    if row.nulls_known {
        let percent = fill(&labels.percent, &row.nulls_percent);
        parts.push(format!("{percent} {}", labels.missing));
    }
    if row.distinct_known {
        parts.push(format!("{} {}", row.distinct, labels.distinct));
    }
    if row.bounds_known {
        parts.push(format!("{} {}", labels.min, row.min));
        parts.push(format!("{} {}", labels.max, row.max));
    }
    parts.join(" · ")
}

fn fact(label: &str, value: &str) -> String {
    format!(
        "{label}: {value}
"
    )
}

/// A titled list after an empty line; nothing when the list is empty.
fn section(title: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "
{title}
"
    );
    for line in lines {
        text += &format!(
            "- {line}
"
        );
    }
    text
}

/// Puts `value` where the translated pattern has `{}`.
fn fill(pattern: &str, value: &str) -> String {
    pattern.replacen("{}", value, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Labels {
        Labels {
            rows: "Rows".into(),
            columns: "Columns".into(),
            size: "Size".into(),
            schema: "Schema".into(),
            warnings: "Warnings".into(),
            missing: "missing".into(),
            distinct: "distinct values".into(),
            parameters: "Parameters".into(),
            tensors: "Tensors".into(),
            parameters_by_type: "Parameters by type".into(),
            shape: "Shape".into(),
            dtype: "Type".into(),
            min: "Min".into(),
            max: "Max".into(),
            nan: "NaN values".into(),
            infinite: "Infinite values".into(),
            protocol: "Protocol".into(),
            imports: "Imports".into(),
            calls: "Calls".into(),
            dangerous: "dangerous".into(),
            at_least: "at least {}".into(),
            percent: "{}%".into(),
        }
    }

    fn info() -> FileInfo {
        FileInfo {
            name: "people.csv".into(),
            size: "1.2 KB".into(),
            rows: "1,234".into(),
            rows_exact: true,
            columns: 2,
            ..Default::default()
        }
    }

    fn age() -> SchemaRow {
        SchemaRow {
            name: "age".into(),
            nulls_known: true,
            nulls_percent: "3.2".into(),
            distinct_known: true,
            distinct: "54".into(),
            bounds_known: true,
            min: "1".into(),
            max: "90".into(),
            ..Default::default()
        }
    }

    fn city() -> SchemaRow {
        SchemaRow {
            name: "city".into(),
            ..Default::default()
        }
    }

    #[test]
    fn table_lists_the_facts_every_column_and_the_warnings() {
        let (age, city) = (age(), city());
        let columns = [
            Column {
                row: &age,
                kind: "integer".into(),
            },
            Column {
                row: &city,
                kind: "text".into(),
            },
        ];
        let warnings = ["age: the distribution is strongly skewed.".to_string()];

        let text = table(&info(), &columns, &warnings, &labels());

        assert_eq!(
            text,
            "people.csv\n\
             Rows: 1,234\n\
             Columns: 2\n\
             Size: 1.2 KB\n\
             \n\
             Schema\n\
             - age: integer · 3.2% missing · 54 distinct values · Min 1 · Max 90\n\
             - city: text\n\
             \n\
             Warnings\n\
             - age: the distribution is strongly skewed.\n"
        );
    }

    #[test]
    fn row_count_from_a_sample_says_at_least() {
        let info = FileInfo {
            rows_exact: false,
            ..info()
        };

        let text = table(&info, &[], &[], &labels());

        assert!(text.contains("Rows: at least 1,234\n"), "{text}");
    }

    #[test]
    fn no_warnings_leave_out_the_warnings_section() {
        let text = table(&info(), &[], &[], &labels());

        assert!(!text.contains("Warnings"), "{text}");
    }

    #[test]
    fn patterns_follow_the_language() {
        let turkish = Labels {
            rows: "Satır".into(),
            missing: "boş".into(),
            at_least: "en az {}".into(),
            percent: "%{}".into(),
            ..labels()
        };
        let info = FileInfo {
            rows_exact: false,
            rows: "1.234".into(),
            ..info()
        };
        let age = age();
        let columns = [Column {
            row: &age,
            kind: "tam sayı".into(),
        }];

        let text = table(&info, &columns, &[], &turkish);

        assert!(text.contains("Satır: en az 1.234\n"), "{text}");
        assert!(text.contains("%3.2 boş"), "{text}");
    }

    #[test]
    fn model_lists_parameters_by_type() {
        let info = ModelInfo {
            name: "model.safetensors".into(),
            size: "86.7 MB".into(),
            parameters: "22,713,728".into(),
            parameters_known: true,
            tensors: 101,
            ..Default::default()
        };
        let dtypes = [DtypeRow {
            name: "F32".into(),
            count: "22,713,216".into(),
            percent: "99.9".into(),
        }];

        let text = model(&info, &dtypes, &[], &labels());

        assert_eq!(
            text,
            "model.safetensors\n\
             Parameters: 22,713,728\n\
             Tensors: 101\n\
             Size: 86.7 MB\n\
             \n\
             Parameters by type\n\
             - F32: 22,713,216 (99.9%)\n"
        );
    }

    #[test]
    fn array_gives_statistics_only_when_there_are_some() {
        let info = ArrayInfo {
            name: "x.npy".into(),
            size: "80 KB".into(),
            descr: "<f4".into(),
            shape: "(100, 3)".into(),
            has_stats: true,
            min: "-1.5".into(),
            max: "2".into(),
            nan: "0".into(),
            infinite: "0".into(),
            ..Default::default()
        };

        let with_stats = array(&info, &[], &labels());
        let without = array(
            &ArrayInfo {
                has_stats: false,
                ..info.clone()
            },
            &[],
            &labels(),
        );

        assert_eq!(
            with_stats,
            "x.npy\n\
             Shape: (100, 3)\n\
             Type: <f4\n\
             Size: 80 KB\n\
             Min: -1.5\n\
             Max: 2\n\
             NaN values: 0\n\
             Infinite values: 0\n"
        );
        assert!(!without.contains("Min"), "{without}");
    }

    #[test]
    fn pickle_marks_dangerous_imports() {
        let info = PickleInfo {
            name: "unsafe.pkl".into(),
            size: "1 KB".into(),
            protocol: "4".into(),
            protocol_known: true,
            calls: "1".into(),
            ..Default::default()
        };
        let imports = [
            ImportRow {
                module: "os".into(),
                name: "system".into(),
                dangerous: true,
            },
            ImportRow {
                module: "collections".into(),
                name: "OrderedDict".into(),
                dangerous: false,
            },
        ];

        let text = pickle(&info, &imports, &[], &labels());

        assert_eq!(
            text,
            "unsafe.pkl\n\
             Protocol: 4\n\
             Calls: 1\n\
             Size: 1 KB\n\
             \n\
             Imports\n\
             - os.system (dangerous)\n\
             - collections.OrderedDict\n"
        );
    }
}
