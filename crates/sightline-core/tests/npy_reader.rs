mod common;

use common::{array, write_bytes, write_npy};
use sightline_core::{
    Basis, DType, FileWarning, Limits, NpyReader, ReadError, Reader, SizeCheck, Summary,
};

fn summarize(path: &std::path::Path) -> Result<Summary, ReadError> {
    NpyReader.summarize(path, &Limits::default())
}

fn f64_bytes(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn assert_malformed(result: Result<Summary, ReadError>) {
    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn reads_the_type_shape_and_order_from_the_header() {
    let path = write_npy(
        "shape.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (2, 3), }",
        &f64_bytes(&[0.0; 6]),
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.descr, "<f8");
    assert_eq!(summary.dtype, DType::F64);
    assert_eq!(summary.shape, vec![2, 3]);
    assert!(!summary.fortran_order);
    assert_eq!(summary.size_check, SizeCheck::Consistent);
}

#[test]
fn reads_fortran_order() {
    let path = write_npy(
        "fortran.npy",
        1,
        "{'descr': '<f8', 'fortran_order': True, 'shape': (1,), }",
        &f64_bytes(&[1.0]),
    );

    assert!(array(summarize(&path).expect("summary")).fortran_order);
}

#[test]
fn reads_version_2_and_3_headers() {
    let header = "{'descr': '<f8', 'fortran_order': False, 'shape': (1,), }";
    for version in [2, 3] {
        let path = write_npy(
            &format!("version{version}.npy"),
            version,
            header,
            &f64_bytes(&[1.0]),
        );

        let summary = array(summarize(&path).expect("summary"));

        assert_eq!(summary.shape, vec![1], "version {version}");
    }
}

#[test]
fn statistics_cover_every_value_of_a_small_array() {
    let values = [3.0, -1.5, f64::NAN, f64::INFINITY, 7.0, f64::NEG_INFINITY];
    let path = write_npy(
        "stats.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (6,), }",
        &f64_bytes(&values),
    );

    let stats = array(summarize(&path).expect("summary"))
        .stats
        .expect("numeric array has statistics");

    assert_eq!(stats.values, 6);
    assert_eq!(stats.basis, Basis::WholeFile);
    assert_eq!(stats.min, Some(-1.5));
    assert_eq!(stats.max, Some(7.0));
    assert_eq!(stats.nan, 1);
    assert_eq!(stats.infinite, 2);
    assert_eq!(stats.histogram.iter().sum::<u64>(), 3);
}

#[test]
fn big_endian_integers_are_read_in_their_byte_order() {
    let data: Vec<u8> = [-5i32, 300, 70_000]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    let path = write_npy(
        "big-endian.npy",
        1,
        "{'descr': '>i4', 'fortran_order': False, 'shape': (3,), }",
        &data,
    );

    let summary = array(summarize(&path).expect("summary"));
    let stats = summary.stats.expect("statistics");

    assert_eq!(summary.dtype, DType::I32);
    assert_eq!((stats.min, stats.max), (Some(-5.0), Some(70_000.0)));
}

#[test]
fn half_precision_values_are_converted() {
    // 1.0, -2.5, 65504 (the largest half) and NaN.
    let data: Vec<u8> = [0x3C00u16, 0xC100, 0x7BFF, 0x7E00]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let path = write_npy(
        "half.npy",
        1,
        "{'descr': '<f2', 'fortran_order': False, 'shape': (4,), }",
        &data,
    );

    let stats = array(summarize(&path).expect("summary"))
        .stats
        .expect("statistics");

    assert_eq!((stats.min, stats.max), (Some(-2.5), Some(65504.0)));
    assert_eq!(stats.nan, 1);
}

#[test]
fn booleans_and_bytes_have_statistics() {
    let path = write_npy(
        "bool.npy",
        1,
        "{'descr': '|b1', 'fortran_order': False, 'shape': (3,), }",
        &[1, 0, 1],
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.dtype, DType::Bool);
    let stats = summary.stats.expect("statistics");
    assert_eq!((stats.min, stats.max), (Some(0.0), Some(1.0)));
}

#[test]
fn an_object_array_is_not_read_because_it_holds_a_pickle() {
    let path = write_npy(
        "object.npy",
        1,
        "{'descr': '|O', 'fortran_order': False, 'shape': (2,), }",
        b"\x80\x04 any pickle bytes",
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.stats, None);
    assert_eq!(summary.size_check, SizeCheck::Unknown);
    assert!(summary.warnings.contains(&FileWarning::PickleInside));
}

#[test]
fn a_structured_array_has_no_statistics() {
    let path = write_npy(
        "structured.npy",
        1,
        "{'descr': [('x', '<f4'), ('label', '<U3')], 'fortran_order': False, 'shape': (1,), }",
        &[0; 16],
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.stats, None);
    assert!(summary.warnings.contains(&FileWarning::NoStatistics));
}

#[test]
fn a_text_array_has_no_statistics_but_a_known_size() {
    let path = write_npy(
        "text.npy",
        1,
        "{'descr': '<U4', 'fortran_order': False, 'shape': (2,), }",
        &[0; 32],
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.dtype, DType::Text);
    assert_eq!(summary.stats, None);
    assert_eq!(summary.size_check, SizeCheck::Consistent);
    assert!(summary.warnings.contains(&FileWarning::NoStatistics));
}

#[test]
fn a_cut_file_is_a_size_mismatch_and_only_present_values_are_read() {
    let path = write_npy(
        "cut.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (4,), }",
        &f64_bytes(&[1.0, 2.0]),
    );
    let file_bytes = std::fs::metadata(&path).expect("metadata").len();

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(
        summary.size_check,
        SizeCheck::Mismatch {
            expected: file_bytes + 16,
            actual: file_bytes,
        }
    );
    assert_eq!(summary.stats.expect("statistics").values, 2);
}

#[test]
fn a_large_array_is_sampled_across_the_file() {
    let values: Vec<f64> = (0..10_000).map(f64::from).collect();
    let path = write_npy(
        "large.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (10000,), }",
        &f64_bytes(&values),
    );
    let limits = Limits {
        sample_bytes: 8_000,
        ..Limits::default()
    };

    let stats = array(NpyReader.summarize(&path, &limits).expect("summary"))
        .stats
        .expect("statistics");

    assert_eq!(stats.basis, Basis::Sample);
    assert!(stats.values <= 1_000, "{}", stats.values);
    assert_eq!(stats.min, Some(0.0));
    assert!(stats.max.expect("max") > 9_800.0, "{:?}", stats.max);
}

#[test]
fn a_scalar_holds_one_value() {
    let path = write_npy(
        "scalar.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (), }",
        &f64_bytes(&[4.0]),
    );

    let summary = array(summarize(&path).expect("summary"));

    assert_eq!(summary.shape, Vec::<u64>::new());
    assert_eq!(summary.stats.expect("statistics").values, 1);
}

#[test]
fn an_empty_array_has_no_smallest_value() {
    let path = write_npy(
        "empty.npy",
        1,
        "{'descr': '<f8', 'fortran_order': False, 'shape': (0, 5), }",
        &[],
    );

    let stats = array(summarize(&path).expect("summary"))
        .stats
        .expect("statistics");

    assert_eq!(stats.values, 0);
    assert_eq!(stats.min, None);
}

#[test]
fn a_file_without_the_magic_string_is_rejected() {
    assert_malformed(summarize(&write_bytes("no-magic.npy", b"PK\x03\x04 a zip")));
}

#[test]
fn a_header_length_beyond_the_file_is_rejected() {
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend_from_slice(&u16::MAX.to_le_bytes());
    bytes.extend_from_slice(b"{}");
    assert_malformed(summarize(&write_bytes("long-header.npy", &bytes)));
}

#[test]
fn a_header_without_the_required_keys_is_rejected() {
    let path = write_npy("no-shape.npy", 1, "{'descr': '<f8', }", &[]);
    assert_malformed(summarize(&path));
}

#[test]
fn a_header_with_code_instead_of_values_is_rejected() {
    let path = write_npy(
        "code.npy",
        1,
        "{'descr': __import__('os').system('echo'), 'fortran_order': False, 'shape': (1,), }",
        &[0; 8],
    );
    assert_malformed(summarize(&path));
}

#[test]
fn deeply_nested_headers_are_rejected_without_overflowing_the_stack() {
    let nested = format!("{}1{}", "(".repeat(1_000), ")".repeat(1_000));
    let header = format!("{{'descr': '<f8', 'fortran_order': False, 'shape': {nested}, }}");
    let path = write_npy("nested.npy", 2, &header, &[]);
    match summarize(&path) {
        Err(ReadError::Malformed { reason, .. }) => assert!(reason.contains("nested"), "{reason}"),
        other => panic!("expected a malformed file, got {other:?}"),
    }
}
