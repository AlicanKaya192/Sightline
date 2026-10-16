mod common;

use common::protobuf::{
    Dim, Graph, bytes_field, external_tensor, graph, model, node, tensor, text_field, value_info,
    varint_field,
};
use common::{model as model_summary, write_bytes};
use sightline_core::layers::totals;
use sightline_core::{
    DType, Dimension, FileWarning, Limits, ModelFormat, OnnxInfo, OnnxReader, ReadError, Reader,
    SizeCheck, Summary, TensorInfo, ValueInfo,
};

const FLOAT: u64 = 1;
const INT64: u64 = 7;
const FLOAT16: u64 = 10;

fn summarize(name: &str, bytes: &[u8]) -> Result<Summary, ReadError> {
    OnnxReader.summarize(&write_bytes(name, bytes), &Limits::default())
}

fn small_graph() -> Graph {
    Graph {
        nodes: vec![node("Conv"), node("Relu"), node("Conv"), node("Gemm")],
        initializers: vec![
            tensor("conv.weight", FLOAT, &[8, 3, 3, 3], 8 * 3 * 3 * 3 * 4),
            tensor("fc.weight", FLOAT16, &[10, 8], 10 * 8 * 2),
        ],
        inputs: vec![
            value_info(
                "pixels",
                FLOAT,
                &[Dim::Param("batch"), Dim::Value(3), Dim::Unknown],
            ),
            value_info("conv.weight", FLOAT, &[Dim::Value(8)]),
        ],
        outputs: vec![value_info("logits", FLOAT, &[Dim::Value(10)])],
    }
}

fn onnx_info(summary: &sightline_core::ModelSummary) -> &OnnxInfo {
    match &summary.format {
        ModelFormat::Onnx(info) => info,
        other => panic!("expected ONNX, got {other:?}"),
    }
}

#[test]
fn accepts_the_onnx_extension() {
    assert!(OnnxReader.can_read("model.ONNX".as_ref()));
    assert!(!OnnxReader.can_read("model.onnx.data".as_ref()));
}

#[test]
fn reads_the_versions_and_the_producer() {
    let summary =
        model_summary(summarize("versions.onnx", &model(&graph(&small_graph()))).unwrap());

    let info = onnx_info(&summary);
    assert_eq!(info.ir_version, 8);
    assert_eq!(
        info.opsets,
        vec![(String::new(), 17), ("ai.onnx.ml".to_string(), 3)]
    );
    assert_eq!(info.producer, "pytorch 2.1");
}

#[test]
fn initializers_are_the_tensors_in_stored_order() {
    let summary = model_summary(summarize("tensors.onnx", &model(&graph(&small_graph()))).unwrap());

    assert_eq!(
        summary.tensors,
        vec![
            TensorInfo {
                name: "conv.weight".to_string(),
                dtype: DType::F32,
                shape: vec![8, 3, 3, 3],
            },
            TensorInfo {
                name: "fc.weight".to_string(),
                dtype: DType::F16,
                shape: vec![10, 8],
            },
        ]
    );
    assert_eq!(totals(&summary.tensors).total, Some(296));
}

#[test]
fn stored_weights_matching_their_shapes_are_consistent() {
    let summary =
        model_summary(summarize("consistent.onnx", &model(&graph(&small_graph()))).unwrap());

    assert_eq!(summary.size_check, SizeCheck::Consistent);
}

#[test]
fn stored_weights_of_the_wrong_length_are_bad_offsets() {
    let mut bad = small_graph();
    bad.initializers[1] = tensor("fc.weight", FLOAT16, &[10, 8], 7);

    let summary = model_summary(summarize("bad-length.onnx", &model(&graph(&bad))).unwrap());

    assert_eq!(
        summary.size_check,
        SizeCheck::BadOffsets {
            tensor: "fc.weight".to_string()
        }
    );
}

#[test]
fn external_weights_are_counted_but_not_opened() {
    let mut split = small_graph();
    split
        .initializers
        .push(external_tensor("big.weight", FLOAT, &[1000, 1000]));

    let summary = model_summary(summarize("external.onnx", &model(&graph(&split))).unwrap());

    assert_eq!(summary.tensors[2].shape, vec![1000, 1000]);
    assert!(
        summary
            .warnings
            .contains(&FileWarning::ExternalData { tensors: 1 })
    );
    assert_eq!(summary.size_check, SizeCheck::Unknown);
}

#[test]
fn operators_are_counted_most_first() {
    let summary =
        model_summary(summarize("operators.onnx", &model(&graph(&small_graph()))).unwrap());

    assert_eq!(
        onnx_info(&summary).operators,
        vec![
            ("Conv".to_string(), 2),
            ("Gemm".to_string(), 1),
            ("Relu".to_string(), 1),
        ]
    );
}

#[test]
fn inputs_leave_out_initializers_and_keep_named_sizes() {
    let summary = model_summary(summarize("inputs.onnx", &model(&graph(&small_graph()))).unwrap());

    let info = onnx_info(&summary);
    assert_eq!(
        info.inputs,
        vec![ValueInfo {
            name: "pixels".to_string(),
            dtype: Some(DType::F32),
            shape: vec![
                Dimension::Named("batch".to_string()),
                Dimension::Fixed(3),
                Dimension::Unknown,
            ],
        }]
    );
    assert_eq!(info.outputs[0].name, "logits");
}

#[test]
fn unpacked_dims_are_read_too() {
    let mut unpacked = varint_field(1, 4);
    unpacked.extend(varint_field(1, 5));
    unpacked.extend(varint_field(2, INT64));
    unpacked.extend(text_field(8, "table"));
    unpacked.extend(bytes_field(9, &[0; 160]));
    let graph_bytes = graph(&Graph {
        nodes: Vec::new(),
        initializers: vec![unpacked],
        inputs: Vec::new(),
        outputs: Vec::new(),
    });

    let summary = model_summary(summarize("unpacked.onnx", &model(&graph_bytes)).unwrap());

    assert_eq!(summary.tensors[0].shape, vec![4, 5]);
    assert_eq!(summary.size_check, SizeCheck::Consistent);
}

#[test]
fn a_file_without_a_graph_is_rejected() {
    let result = summarize("no-graph.onnx", &varint_field(1, 8));

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn a_cut_file_is_rejected() {
    let mut bytes = model(&graph(&small_graph()));
    bytes.truncate(bytes.len() / 2);

    let result = summarize("cut.onnx", &bytes);

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn a_file_that_is_not_protobuf_is_rejected() {
    let result = summarize("text.onnx", b"this is not a model at all");

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}
