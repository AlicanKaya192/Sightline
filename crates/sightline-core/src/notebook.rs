//! "Open in notebook": a Jupyter notebook (nbformat 4) whose code reads the file.
//! A pickle gets code that only lists its opcodes; nothing generated ever unpickles.

use std::path::Path;

use serde_json::{Value, json};

use crate::summary::{FileWarning, ModelFormat, Summary, TableFormat, TextEncoding};

/// The `.ipynb` text for `path`, with code chosen from what its summary found.
pub fn notebook(path: &Path, summary: &Summary) -> String {
    let file_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let notebook = json!({
        "cells": [
            {
                "cell_type": "markdown",
                "metadata": {},
                "source": [format!("# {file_name}")],
            },
            {
                "cell_type": "code",
                "execution_count": null,
                "metadata": {},
                "outputs": [],
                "source": source_lines(&code(&python_string(&path.to_string_lossy()), summary)),
            },
        ],
        "metadata": {
            "kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"},
            "language_info": {"name": "python"},
        },
        "nbformat": 4,
        "nbformat_minor": 4,
    });
    let mut text = serde_json::to_string_pretty(&notebook).expect("a JSON value always serializes");
    text.push('\n');
    text
}

/// Python code reading the file whose path is the Python string literal `path`.
fn code(path: &str, summary: &Summary) -> String {
    match summary {
        Summary::Table(table) => match &table.format {
            TableFormat::Parquet { .. } => {
                format!("import pandas as pd\n\ndf = pd.read_parquet({path})\ndf.head(20)")
            }
            TableFormat::Csv {
                delimiter,
                encoding,
            } => {
                let separator = python_string(&char::from(*delimiter).to_string());
                let encoding = match encoding {
                    TextEncoding::Utf8 => String::new(),
                    TextEncoding::Windows1254 => ", encoding=\"cp1254\"".to_string(),
                };
                format!(
                    "import pandas as pd\n\ndf = pd.read_csv({path}, sep={separator}{encoding})\ndf.head(20)"
                )
            }
            TableFormat::Excel { sheets } => {
                // pandas reads the first sheet when given 0, as Sightline does.
                let sheet = sheets
                    .first()
                    .map_or_else(|| "0".to_string(), |name| python_string(name));
                format!(
                    "import pandas as pd\n\ndf = pd.read_excel({path}, sheet_name={sheet})\ndf.head(20)"
                )
            }
        },
        Summary::Model(model) => match model.format {
            ModelFormat::Safetensors { .. } => format!(
                "from safetensors import safe_open\n\n\
                 with safe_open({path}, framework=\"numpy\") as file:\n\
                 \x20   for name in file.keys():\n\
                 \x20       print(name, file.get_slice(name).get_shape())"
            ),
            ModelFormat::Onnx(_) => format!(
                "import onnx\n\n\
                 model = onnx.load({path}, load_external_data=False)\n\
                 print(onnx.helper.printable_graph(model.graph))"
            ),
        },
        Summary::Array(array) if array.warnings.contains(&FileWarning::PickleInside) => {
            "# This array holds pickled Python objects. Loading them means unpickling,\n\
             # which can run code, so no loading code is written here."
                .to_string()
        }
        Summary::Array(_) => format!(
            "import numpy as np\n\n\
             array = np.load({path}, allow_pickle=False)\n\
             array.shape, array.dtype"
        ),
        Summary::Pickle(_) => format!(
            "import pickletools\n\n\
             # Loading a pickle can run code, so it is only listed here, never loaded.\n\
             with open({path}, \"rb\") as file:\n\
             \x20   pickletools.dis(file)"
        ),
    }
}

/// A double-quoted Python string literal for `text`.
fn python_string(text: &str) -> String {
    let mut literal = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            c if c.is_control() => literal.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

/// Notebook source: lines that keep their newline, except the last.
fn source_lines(code: &str) -> Value {
    let lines: Vec<&str> = code.split_inclusive('\n').collect();
    json!(lines)
}
