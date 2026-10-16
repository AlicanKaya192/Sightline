mod common;

use common::{model, write_bytes, write_safetensors};
use sightline_core::{
    DType, Limits, ModelFormat, ReadError, Reader, SafetensorsReader, SizeCheck, TensorInfo,
};

fn summarize(path: &std::path::Path) -> Result<sightline_core::Summary, ReadError> {
    SafetensorsReader.summarize(path, &Limits::default())
}

fn assert_malformed(result: Result<sightline_core::Summary, ReadError>, reason_part: &str) {
    match result {
        Err(ReadError::Malformed { reason, .. }) => {
            assert!(reason.contains(reason_part), "{reason}");
        }
        other => panic!("expected a malformed file, got {other:?}"),
    }
}

fn tensor(name: &str, dtype: DType, shape: &[u64]) -> TensorInfo {
    TensorInfo {
        name: name.to_string(),
        dtype,
        shape: shape.to_vec(),
    }
}

#[test]
fn accepts_the_safetensors_extension_only() {
    assert!(SafetensorsReader.can_read("model.SAFETENSORS".as_ref()));
    assert!(!SafetensorsReader.can_read("model.bin".as_ref()));
}

#[test]
fn lists_tensors_in_stored_order_with_type_and_shape() {
    let header = r#"{
        "z.weight": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 24]},
        "a.bias": {"dtype": "BF16", "shape": [3], "data_offsets": [24, 30]}
    }"#;
    let path = write_safetensors("order.safetensors", header, 30);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(
        summary.tensors,
        vec![
            tensor("z.weight", DType::F32, &[2, 3]),
            tensor("a.bias", DType::BF16, &[3]),
        ]
    );
}

#[test]
fn keeps_the_metadata_sorted_by_key() {
    let header = r#"{
        "__metadata__": {"format": "pt", "author": "test"},
        "w": {"dtype": "U8", "shape": [1], "data_offsets": [0, 1]}
    }"#;
    let path = write_safetensors("metadata.safetensors", header, 1);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(
        summary.format,
        ModelFormat::Safetensors {
            metadata: vec![
                ("author".to_string(), "test".to_string()),
                ("format".to_string(), "pt".to_string()),
            ]
        }
    );
    assert_eq!(summary.tensors.len(), 1);
}

#[test]
fn a_file_as_long_as_its_header_says_is_consistent() {
    let header = r#"{"w": {"dtype": "F16", "shape": [4], "data_offsets": [0, 8]}}"#;
    let path = write_safetensors("consistent.safetensors", header, 8);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(summary.size_check, SizeCheck::Consistent);
}

#[test]
fn extra_bytes_after_the_weights_are_a_size_mismatch() {
    let header = r#"{"w": {"dtype": "F16", "shape": [4], "data_offsets": [0, 8]}}"#;
    let path = write_safetensors("extra.safetensors", header, 10);

    let summary = model(summarize(&path).expect("summary"));

    let header_end = 8 + header.len() as u64;
    assert_eq!(
        summary.size_check,
        SizeCheck::Mismatch {
            expected: header_end + 8,
            actual: header_end + 10,
        }
    );
}

#[test]
fn a_tensor_whose_bytes_do_not_match_its_shape_has_bad_offsets() {
    let header = r#"{
        "a": {"dtype": "F32", "shape": [2], "data_offsets": [0, 4]},
        "b": {"dtype": "F32", "shape": [1], "data_offsets": [4, 12]}
    }"#;
    let path = write_safetensors("bad-offsets.safetensors", header, 12);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(
        summary.size_check,
        SizeCheck::BadOffsets {
            tensor: "a".to_string()
        }
    );
}

#[test]
fn overlapping_tensors_have_bad_offsets() {
    let header = r#"{
        "a": {"dtype": "U8", "shape": [4], "data_offsets": [0, 4]},
        "b": {"dtype": "U8", "shape": [4], "data_offsets": [2, 6]}
    }"#;
    let path = write_safetensors("overlap.safetensors", header, 8);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(
        summary.size_check,
        SizeCheck::BadOffsets {
            tensor: "b".to_string()
        }
    );
}

#[test]
fn an_unknown_type_is_listed_and_leaves_the_size_unknown() {
    let header = r#"{"w": {"dtype": "F4_NEW", "shape": [2, 2], "data_offsets": [0, 2]}}"#;
    let path = write_safetensors("unknown-type.safetensors", header, 2);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(
        summary.tensors,
        vec![tensor("w", DType::Other("F4_NEW".to_string()), &[2, 2])]
    );
    assert_eq!(summary.tensors[0].elements(), Some(4));
    assert_eq!(summary.size_check, SizeCheck::Unknown);
}

#[test]
fn a_shape_beyond_64_bits_has_no_element_count() {
    let header = format!(
        r#"{{"w": {{"dtype": "U8", "shape": [{}, 2], "data_offsets": [0, 0]}}}}"#,
        u64::MAX
    );
    let path = write_safetensors("huge-shape.safetensors", &header, 0);

    let summary = model(summarize(&path).expect("summary"));

    assert_eq!(summary.tensors[0].elements(), None);
    assert_eq!(summary.size_check, SizeCheck::Unknown);
}

#[test]
fn a_header_length_beyond_the_file_is_rejected_without_reading_it() {
    let mut bytes = 1_000u64.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{}");
    let path = write_bytes("long-header.safetensors", &bytes);

    let result = summarize(&path);

    assert_malformed(result, "beyond the end of the file");
}

#[test]
fn a_header_over_the_limit_is_rejected() {
    let mut bytes = u64::MAX.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"{}");
    let path = write_bytes("over-limit.safetensors", &bytes);

    let result = summarize(&path);

    assert_malformed(result, "limit");
}

#[test]
fn a_header_that_is_not_json_is_rejected() {
    let path = write_safetensors("not-json.safetensors", "not json", 0);

    let result = summarize(&path);

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn a_tensor_without_offsets_is_rejected() {
    let path = write_safetensors(
        "no-offsets.safetensors",
        r#"{"w": {"dtype": "F32", "shape": [1]}}"#,
        4,
    );

    let result = summarize(&path);

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn a_file_shorter_than_the_length_field_is_rejected() {
    let path = write_bytes("short.safetensors", b"\x02\x00");

    let result = summarize(&path);

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}
