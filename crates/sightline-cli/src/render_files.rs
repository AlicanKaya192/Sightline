//! Plain text for model, array and pickle files. Developer tool: not translated.

use sightline_core::layers::totals;
use sightline_core::{
    ArraySummary, DType, FileWarning, ModelFormat, ModelSummary, PickleSummary, SizeCheck,
};

use crate::render::{aligned, labelled};

const MISSING: &str = "-";

pub fn render_model(file_name: &str, summary: &ModelSummary) -> String {
    let totals = totals(&summary.tensors);
    let mut text = format!("{file_name}\n");
    let format = match &summary.format {
        ModelFormat::Safetensors { .. } => "safetensors".to_string(),
        ModelFormat::Onnx(onnx) => format!("ONNX, IR version {}", onnx.ir_version),
    };
    text += &labelled("Format:", &format);
    text += &labelled(
        "Params:",
        &totals
            .total
            .map_or_else(|| MISSING.to_string(), |n| n.to_string()),
    );
    text += &labelled("Tensors:", &summary.tensors.len().to_string());
    text += &labelled("Size:", &size_check_text(&summary.size_check));
    for (dtype, count) in &totals.by_dtype {
        text += &labelled("", &format!("{count} {}", dtype_text(dtype)));
    }
    text += &warnings_text(&summary.warnings);
    text += "\n";
    let mut rows = vec![["tensor", "type", "shape"].map(String::from).to_vec()];
    rows.extend(summary.tensors.iter().map(|tensor| {
        vec![
            tensor.name.clone(),
            dtype_text(&tensor.dtype),
            shape_text(&tensor.shape),
        ]
    }));
    text += &aligned(&rows);
    text
}

pub fn render_array(file_name: &str, summary: &ArraySummary) -> String {
    let mut text = format!("{file_name}\n");
    text += &labelled("Type:", &summary.descr);
    text += &labelled("Shape:", &shape_text(&summary.shape));
    text += &labelled("Size:", &size_check_text(&summary.size_check));
    if let Some(stats) = &summary.stats {
        let number =
            |value: Option<f64>| value.map_or_else(|| MISSING.to_string(), |v| v.to_string());
        text += &labelled("Min:", &number(stats.min));
        text += &labelled("Max:", &number(stats.max));
        text += &labelled("NaN:", &stats.nan.to_string());
        text += &labelled("Infinite:", &stats.infinite.to_string());
        text += &labelled("Read:", &format!("{} values", stats.values));
    }
    text += &warnings_text(&summary.warnings);
    text
}

pub fn render_pickle(file_name: &str, summary: &PickleSummary) -> String {
    let mut text = format!("{file_name}\n");
    text += &labelled(
        "Protocol:",
        &summary
            .protocol
            .map_or_else(|| "0 or 1".to_string(), |p| p.to_string()),
    );
    text += &labelled("Opcodes:", &summary.opcodes.to_string());
    text += &labelled("Calls:", &summary.calls.to_string());
    text += &warnings_text(&summary.warnings);
    text += "\nImports:\n";
    for import in &summary.imports {
        let mark = if import.dangerous { "  DANGEROUS" } else { "" };
        text += &format!("  {}.{}{mark}\n", import.module, import.name);
    }
    text
}

fn size_check_text(check: &SizeCheck) -> String {
    match check {
        SizeCheck::Consistent => "matches the header".to_string(),
        SizeCheck::Mismatch { expected, actual } => {
            format!("header says {expected} bytes, file has {actual}")
        }
        SizeCheck::BadOffsets { tensor } => format!("bad offsets at {tensor}"),
        SizeCheck::Unknown => "not checked".to_string(),
    }
}

fn warnings_text(warnings: &[FileWarning]) -> String {
    warnings
        .iter()
        .map(|warning| {
            let line = match warning {
                FileWarning::ExternalData { tensors } => {
                    format!("{tensors} tensors keep their weights in other files")
                }
                FileWarning::PickleInside => "holds pickled objects; not read".to_string(),
                FileWarning::NoStatistics => "no statistics for this type".to_string(),
                FileWarning::RunsCodeWhenLoaded => "loading it can run code".to_string(),
                FileWarning::DangerousImport { module, name } => {
                    format!("calls {module}.{name}")
                }
                FileWarning::StoppedEarly => "stopped before the end of the file".to_string(),
            };
            labelled("Warning:", &line)
        })
        .collect()
}

fn shape_text(shape: &[u64]) -> String {
    let sizes: Vec<String> = shape.iter().map(u64::to_string).collect();
    format!("[{}]", sizes.join(", "))
}

fn dtype_text(dtype: &DType) -> String {
    match dtype {
        DType::Other(name) => name.clone(),
        known => format!("{known:?}"),
    }
}

#[cfg(test)]
mod tests {
    use sightline_core::{PickleImport, TensorInfo};

    use super::*;

    fn model() -> ModelSummary {
        ModelSummary {
            format: ModelFormat::Safetensors {
                metadata: Vec::new(),
            },
            tensors: vec![
                TensorInfo {
                    name: "w".to_string(),
                    dtype: DType::F32,
                    shape: vec![3, 4],
                },
                TensorInfo {
                    name: "b".to_string(),
                    dtype: DType::F32,
                    shape: vec![4],
                },
            ],
            size_check: SizeCheck::Consistent,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn model_text_has_the_parameter_total_and_tensors() {
        let text = render_model("m.safetensors", &model());

        assert!(text.contains("Params:   16\n"), "{text}");
        assert!(text.contains("w       F32   [3, 4]"), "{text}");
    }

    #[test]
    fn pickle_text_marks_dangerous_imports() {
        let summary = PickleSummary {
            protocol: None,
            opcodes: 5,
            imports: vec![PickleImport {
                module: "os".to_string(),
                name: "system".to_string(),
                dangerous: true,
            }],
            calls: 1,
            warnings: vec![FileWarning::RunsCodeWhenLoaded],
        };

        let text = render_pickle("a.pkl", &summary);

        assert!(text.contains("  os.system  DANGEROUS"), "{text}");
        assert!(text.contains("Protocol: 0 or 1"), "{text}");
    }
}
