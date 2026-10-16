//! Turns a model summary (safetensors, ONNX) into the values the model view shows.

use sightline_core::layers::{totals, tree};
use sightline_core::{Dimension, ModelFormat as CoreFormat, ModelSummary, TensorInfo};

use crate::file_view::{dtype_name, file_warning_row, shape_text, size_check_warning, size_state};
use crate::view::{NumberFormat, size_text, to_int};
use crate::{DtypeRow, IoRow, LayerRow, ModelFormat, ModelInfo, NamedValue, TensorRow, WarningRow};

const UNKNOWN: &str = "—";
/// ONNX writes the standard operator set with an empty domain.
const DEFAULT_ONNX_DOMAIN: &str = "ai.onnx";

pub fn model_info(
    name: &str,
    size_bytes: u64,
    summary: &ModelSummary,
    format: &NumberFormat,
) -> ModelInfo {
    let total = totals(&summary.tensors).total;
    let mut info = ModelInfo {
        name: name.into(),
        size: size_text(size_bytes, format).into(),
        parameters: total
            .map_or_else(String::new, |count| format.count(count))
            .into(),
        parameters_known: total.is_some(),
        tensors: to_int(summary.tensors.len()),
        size_state: size_state(&summary.size_check),
        ..ModelInfo::default()
    };
    match &summary.format {
        CoreFormat::Safetensors { .. } => info.format = ModelFormat::Safetensors,
        CoreFormat::Onnx(onnx) => {
            info.format = ModelFormat::Onnx;
            info.ir_version = onnx.ir_version.to_string().into();
            info.opsets = onnx
                .opsets
                .iter()
                .map(|(domain, version)| {
                    let domain = if domain.is_empty() {
                        DEFAULT_ONNX_DOMAIN
                    } else {
                        domain
                    };
                    format!("{domain} {version}")
                })
                .collect::<Vec<_>>()
                .join(", ")
                .into();
            info.producer = onnx.producer.as_str().into();
        }
    }
    info
}

/// Parameters per element type with their share of the total, most first.
pub fn dtype_rows(tensors: &[TensorInfo], format: &NumberFormat) -> Vec<DtypeRow> {
    let totals = totals(tensors);
    let all: u64 = totals.by_dtype.iter().map(|(_, count)| count).sum();
    totals
        .by_dtype
        .iter()
        .map(|(dtype, count)| DtypeRow {
            name: dtype_name(dtype).into(),
            count: format.count(*count).into(),
            percent: format
                .one_decimal(if all == 0 {
                    0.0
                } else {
                    *count as f64 * 100.0 / all as f64
                })
                .into(),
        })
        .collect()
}

pub fn metadata_rows(summary: &ModelSummary) -> Vec<NamedValue> {
    match &summary.format {
        CoreFormat::Safetensors { metadata } => metadata
            .iter()
            .map(|(name, value)| NamedValue {
                name: name.as_str().into(),
                value: value.as_str().into(),
            })
            .collect(),
        CoreFormat::Onnx(_) => Vec::new(),
    }
}

/// The module tree, each line named by its last part and indented by its depth.
pub fn layer_rows(tensors: &[TensorInfo], format: &NumberFormat) -> Vec<LayerRow> {
    tree(tensors)
        .into_iter()
        .map(|node| LayerRow {
            name: node.path.rsplit('.').next().unwrap_or_default().into(),
            depth: to_int(node.depth),
            parameters: node
                .parameters
                .map_or_else(|| UNKNOWN.to_string(), |count| format.count(count))
                .into(),
            tensors: i32::try_from(node.tensors).unwrap_or(i32::MAX),
        })
        .collect()
}

pub fn tensor_rows(tensors: &[TensorInfo], format: &NumberFormat) -> Vec<TensorRow> {
    tensors
        .iter()
        .map(|tensor| TensorRow {
            name: tensor.name.as_str().into(),
            dtype: dtype_name(&tensor.dtype).into(),
            shape: shape_text(&tensor.shape, format).into(),
            parameters: tensor
                .elements()
                .map_or_else(|| UNKNOWN.to_string(), |count| format.count(count))
                .into(),
        })
        .collect()
}

/// ONNX graph inputs, then outputs.
pub fn io_rows(summary: &ModelSummary, format: &NumberFormat) -> Vec<IoRow> {
    let CoreFormat::Onnx(onnx) = &summary.format else {
        return Vec::new();
    };
    let row = |input: bool| {
        move |value: &sightline_core::ValueInfo| IoRow {
            name: value.name.as_str().into(),
            input,
            dtype: value
                .dtype
                .as_ref()
                .map_or_else(|| "?".to_string(), dtype_name)
                .into(),
            shape: dimensions_text(&value.shape, format).into(),
        }
    };
    onnx.inputs
        .iter()
        .map(row(true))
        .chain(onnx.outputs.iter().map(row(false)))
        .collect()
}

pub fn operator_rows(summary: &ModelSummary, format: &NumberFormat) -> Vec<NamedValue> {
    let CoreFormat::Onnx(onnx) = &summary.format else {
        return Vec::new();
    };
    onnx.operators
        .iter()
        .map(|(name, count)| NamedValue {
            name: name.as_str().into(),
            value: format.count(*count).into(),
        })
        .collect()
}

pub fn model_warnings(summary: &ModelSummary, format: &NumberFormat) -> Vec<WarningRow> {
    size_check_warning(&summary.size_check, format)
        .into_iter()
        .chain(
            summary
                .warnings
                .iter()
                .map(|warning| file_warning_row(warning, format)),
        )
        .collect()
}

/// `[batch × 3 × 224 × 224]`, with `?` for a size the file does not give.
fn dimensions_text(dimensions: &[Dimension], format: &NumberFormat) -> String {
    let parts: Vec<String> = dimensions
        .iter()
        .map(|dimension| match dimension {
            Dimension::Fixed(size) => format.count(*size),
            Dimension::Named(name) => name.clone(),
            Dimension::Unknown => "?".to_string(),
        })
        .collect();
    format!("[{}]", parts.join(" × "))
}

#[cfg(test)]
mod tests {
    use sightline_core::{DType, FileWarning, OnnxInfo, SizeCheck, ValueInfo};

    use super::*;
    use crate::{SizeState, WarningKind};

    const ENGLISH: NumberFormat = NumberFormat::ENGLISH;

    fn tensor(name: &str, dtype: DType, shape: &[u64]) -> TensorInfo {
        TensorInfo {
            name: name.to_string(),
            dtype,
            shape: shape.to_vec(),
        }
    }

    fn safetensors(tensors: Vec<TensorInfo>) -> ModelSummary {
        ModelSummary {
            format: CoreFormat::Safetensors {
                metadata: vec![("format".to_string(), "pt".to_string())],
            },
            tensors,
            size_check: SizeCheck::Consistent,
            warnings: Vec::new(),
        }
    }

    fn onnx() -> ModelSummary {
        ModelSummary {
            format: CoreFormat::Onnx(OnnxInfo {
                ir_version: 8,
                opsets: vec![(String::new(), 17), ("ai.onnx.ml".to_string(), 3)],
                producer: "pytorch 2.1".to_string(),
                inputs: vec![ValueInfo {
                    name: "pixels".to_string(),
                    dtype: Some(DType::F32),
                    shape: vec![
                        Dimension::Named("batch".to_string()),
                        Dimension::Fixed(3),
                        Dimension::Unknown,
                    ],
                }],
                outputs: vec![ValueInfo {
                    name: "logits".to_string(),
                    dtype: None,
                    shape: vec![Dimension::Fixed(1000)],
                }],
                operators: vec![("Conv".to_string(), 53), ("Relu".to_string(), 49)],
            }),
            tensors: Vec::new(),
            size_check: SizeCheck::Unknown,
            warnings: vec![FileWarning::ExternalData { tensors: 2 }],
        }
    }

    #[test]
    fn info_counts_parameters_and_tensors() {
        let summary = safetensors(vec![
            tensor("a", DType::F32, &[1000, 1000]),
            tensor("b", DType::F32, &[24]),
        ]);

        let info = model_info("model.safetensors", 4_000_096, &summary, &ENGLISH);

        assert_eq!(info.format, ModelFormat::Safetensors);
        assert_eq!(info.parameters, "1,000,024");
        assert!(info.parameters_known);
        assert_eq!(info.tensors, 2);
        assert_eq!(info.size_state, SizeState::Consistent);
    }

    #[test]
    fn onnx_info_names_the_default_operator_set() {
        let info = model_info("model.onnx", 10, &onnx(), &ENGLISH);

        assert_eq!(info.format, ModelFormat::Onnx);
        assert_eq!(info.ir_version, "8");
        assert_eq!(info.opsets, "ai.onnx 17, ai.onnx.ml 3");
        assert_eq!(info.producer, "pytorch 2.1");
    }

    #[test]
    fn dtype_rows_give_each_type_its_share() {
        let rows = dtype_rows(
            &[tensor("a", DType::F16, &[3]), tensor("b", DType::F32, &[1])],
            &ENGLISH,
        );

        assert_eq!(
            rows,
            vec![
                DtypeRow {
                    name: "F16".into(),
                    count: "3".into(),
                    percent: "75.0".into(),
                },
                DtypeRow {
                    name: "F32".into(),
                    count: "1".into(),
                    percent: "25.0".into(),
                },
            ]
        );
    }

    #[test]
    fn layer_rows_are_named_by_their_last_part() {
        let rows = layer_rows(
            &[tensor("model.layers.0.weight", DType::F32, &[2, 2])],
            &ENGLISH,
        );

        let names: Vec<(&str, i32)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.depth))
            .collect();
        assert_eq!(names, [("model", 0), ("layers", 1), ("0", 2)]);
        assert_eq!(rows[2].parameters, "4");
    }

    #[test]
    fn tensor_rows_show_type_shape_and_count() {
        let rows = tensor_rows(&[tensor("w", DType::BF16, &[4096, 11008])], &ENGLISH);

        assert_eq!(
            rows,
            vec![TensorRow {
                name: "w".into(),
                dtype: "BF16".into(),
                shape: "4,096 × 11,008".into(),
                parameters: "45,088,768".into(),
            }]
        );
    }

    #[test]
    fn metadata_is_listed_for_safetensors() {
        let rows = metadata_rows(&safetensors(Vec::new()));

        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].name.as_str(), rows[0].value.as_str()),
            ("format", "pt")
        );
    }

    #[test]
    fn io_rows_list_inputs_then_outputs_with_named_sizes() {
        let rows = io_rows(&onnx(), &ENGLISH);

        assert_eq!(rows.len(), 2);
        assert!(rows[0].input);
        assert_eq!(rows[0].shape, "[batch × 3 × ?]");
        assert_eq!(rows[0].dtype, "F32");
        assert!(!rows[1].input);
        assert_eq!(rows[1].dtype, "?");
    }

    #[test]
    fn operator_rows_keep_the_order_and_counts() {
        let rows = operator_rows(&onnx(), &ENGLISH);

        assert_eq!(
            (rows[0].name.as_str(), rows[0].value.as_str()),
            ("Conv", "53")
        );
    }

    #[test]
    fn warnings_start_with_the_size_check() {
        let mut summary = onnx();
        summary.size_check = SizeCheck::Mismatch {
            expected: 1,
            actual: 2,
        };

        let kinds: Vec<WarningKind> = model_warnings(&summary, &ENGLISH)
            .iter()
            .map(|row| row.kind)
            .collect();

        assert_eq!(
            kinds,
            [WarningKind::SizeMismatch, WarningKind::ExternalData]
        );
    }
}
