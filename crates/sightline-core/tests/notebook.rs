use std::path::Path;

use serde_json::Value;
use sightline_core::notebook::notebook;
use sightline_core::{
    ArraySummary, DType, FileWarning, ModelFormat, ModelSummary, OnnxInfo, PickleSummary, RowCount,
    SizeCheck, Summary, TableFormat, TableSummary, TextEncoding,
};

fn table(format: TableFormat) -> Summary {
    Summary::Table(TableSummary {
        format,
        rows: RowCount::Exact(0),
        columns: Vec::new(),
        head: Vec::new(),
        insights: Vec::new(),
        sampled_rows: None,
    })
}

fn model(format: ModelFormat) -> Summary {
    Summary::Model(ModelSummary {
        format,
        tensors: Vec::new(),
        size_check: SizeCheck::Consistent,
        warnings: Vec::new(),
    })
}

fn array(warnings: Vec<FileWarning>) -> Summary {
    Summary::Array(ArraySummary {
        descr: "<f8".to_string(),
        dtype: DType::F64,
        shape: vec![1],
        fortran_order: false,
        size_check: SizeCheck::Consistent,
        stats: None,
        warnings,
    })
}

fn pickle() -> Summary {
    Summary::Pickle(PickleSummary {
        protocol: Some(4),
        opcodes: 1,
        imports: Vec::new(),
        calls: 0,
        warnings: vec![FileWarning::RunsCodeWhenLoaded],
    })
}

fn parsed(path: &str, summary: &Summary) -> Value {
    serde_json::from_str(&notebook(Path::new(path), summary)).expect("valid JSON")
}

/// The code of every code cell, joined.
fn code(path: &str, summary: &Summary) -> String {
    parsed(path, summary)["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .filter(|cell| cell["cell_type"] == "code")
        .flat_map(|cell| cell["source"].as_array().expect("source lines").clone())
        .map(|line| line.as_str().expect("text line").to_string())
        .collect()
}

const CSV: TableFormat = TableFormat::Csv {
    delimiter: b';',
    encoding: TextEncoding::Windows1254,
};

#[test]
fn the_notebook_is_nbformat_4_with_a_title_and_a_code_cell() {
    let notebook = parsed(r"C:\data\sales.csv", &table(CSV));

    assert_eq!(notebook["nbformat"], 4);
    assert_eq!(notebook["metadata"]["kernelspec"]["language"], "python");
    let cells = notebook["cells"].as_array().expect("cells");
    assert_eq!(cells[0]["cell_type"], "markdown");
    assert_eq!(cells[0]["source"][0], "# sales.csv");
    assert_eq!(cells[1]["cell_type"], "code");
    assert_eq!(cells[1]["outputs"], Value::Array(Vec::new()));
    assert_eq!(cells[1]["execution_count"], Value::Null);
}

#[test]
fn csv_code_uses_the_detected_delimiter_and_encoding() {
    let code = code(r"C:\data\sales.csv", &table(CSV));

    assert!(code.contains("pd.read_csv("), "{code}");
    assert!(code.contains(r#"sep=";""#), "{code}");
    assert!(code.contains(r#"encoding="cp1254""#), "{code}");
}

#[test]
fn utf8_csv_code_needs_no_encoding() {
    let format = TableFormat::Csv {
        delimiter: b',',
        encoding: TextEncoding::Utf8,
    };

    let code = code(r"C:\data\a.csv", &table(format));

    assert!(!code.contains("encoding="), "{code}");
}

#[test]
fn parquet_code_reads_with_pandas() {
    let code = code(
        r"C:\data\trips.parquet",
        &table(TableFormat::Parquet { row_groups: 1 }),
    );

    assert!(code.contains("pd.read_parquet("), "{code}");
}

#[test]
fn the_path_is_a_correctly_escaped_python_string() {
    let path = r"C:\Veri\şehir'ler\a b.csv";

    let code = code(path, &table(CSV));

    assert!(code.contains(r#""C:\\Veri\\şehir'ler\\a b.csv""#), "{code}");
}

#[test]
fn pickle_code_only_lists_opcodes_and_never_loads() {
    let code = code(r"C:\data\model.pkl", &pickle());

    assert!(code.contains("pickletools.dis("), "{code}");
    assert!(!code.contains("pickle.load"), "{code}");
    assert!(!code.contains("loads("), "{code}");
    assert!(!code.contains("import pickle\n"), "{code}");
}

#[test]
fn npy_code_refuses_pickles() {
    let code = code(r"C:\data\a.npy", &array(Vec::new()));

    assert!(code.contains("np.load("), "{code}");
    assert!(code.contains("allow_pickle=False"), "{code}");
}

#[test]
fn an_object_array_gets_no_loading_code() {
    let code = code(
        r"C:\data\objects.npy",
        &array(vec![FileWarning::PickleInside]),
    );

    assert!(!code.contains("np.load("), "{code}");
    assert!(!code.contains("allow_pickle=True"), "{code}");
}

#[test]
fn safetensors_code_opens_without_loading_every_weight() {
    let code = code(
        r"C:\models\model.safetensors",
        &model(ModelFormat::Safetensors {
            metadata: Vec::new(),
        }),
    );

    assert!(code.contains("safe_open("), "{code}");
}

#[test]
fn onnx_code_does_not_load_external_weights() {
    let info = OnnxInfo {
        ir_version: 8,
        opsets: Vec::new(),
        producer: String::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        operators: Vec::new(),
    };

    let code = code(r"C:\models\model.onnx", &model(ModelFormat::Onnx(info)));

    assert!(code.contains("onnx.load("), "{code}");
    assert!(code.contains("load_external_data=False"), "{code}");
}
