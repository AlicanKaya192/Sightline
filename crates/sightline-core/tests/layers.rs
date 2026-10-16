use sightline_core::layers::{totals, tree};
use sightline_core::{DType, LayerNode, ParameterTotals, TensorInfo};

fn tensor(name: &str, dtype: DType, shape: &[u64]) -> TensorInfo {
    TensorInfo {
        name: name.to_string(),
        dtype,
        shape: shape.to_vec(),
    }
}

fn node(path: &str, depth: usize, tensors: u64, parameters: u64) -> LayerNode {
    LayerNode {
        path: path.to_string(),
        depth,
        tensors,
        parameters: Some(parameters),
    }
}

#[test]
fn modules_come_from_dotted_names_depth_first_in_stored_order() {
    let tensors = [
        tensor("model.layers.0.attn.weight", DType::F32, &[4, 4]),
        tensor("model.layers.0.attn.bias", DType::F32, &[4]),
        tensor("model.layers.1.attn.weight", DType::F32, &[4, 4]),
        tensor("lm_head.weight", DType::F32, &[2, 4]),
    ];

    let nodes = tree(&tensors);

    assert_eq!(
        nodes,
        vec![
            node("model", 0, 3, 36),
            node("model.layers", 1, 3, 36),
            node("model.layers.0", 2, 2, 20),
            node("model.layers.0.attn", 3, 2, 20),
            node("model.layers.1", 2, 1, 16),
            node("model.layers.1.attn", 3, 1, 16),
            node("lm_head", 0, 1, 8),
        ]
    );
}

#[test]
fn a_name_without_dots_is_its_own_module() {
    let nodes = tree(&[tensor("embedding", DType::F16, &[10, 3])]);

    assert_eq!(nodes, vec![node("embedding", 0, 1, 30)]);
}

#[test]
fn a_module_with_a_count_beyond_64_bits_has_no_parameter_count() {
    let nodes = tree(&[
        tensor("a.x", DType::U8, &[u64::MAX]),
        tensor("a.y", DType::U8, &[2]),
    ]);

    assert_eq!(nodes[0].parameters, None);
    assert_eq!(nodes[0].tensors, 2);
}

#[test]
fn totals_count_every_value_and_split_them_by_type_most_first() {
    let tensors = [
        tensor("a", DType::F16, &[2, 3]),
        tensor("b", DType::F32, &[10]),
        tensor("c", DType::F16, &[1]),
    ];

    assert_eq!(
        totals(&tensors),
        ParameterTotals {
            total: Some(17),
            by_dtype: vec![(DType::F32, 10), (DType::F16, 7)],
        }
    );
}

#[test]
fn a_scalar_tensor_holds_one_value() {
    assert_eq!(totals(&[tensor("step", DType::I64, &[])]).total, Some(1));
}

#[test]
fn totals_beyond_64_bits_are_unknown() {
    let tensors = [
        tensor("a", DType::U8, &[u64::MAX]),
        tensor("b", DType::U8, &[1]),
    ];

    assert_eq!(totals(&tensors).total, None);
}

#[test]
fn no_tensors_means_no_modules_and_zero_parameters() {
    assert!(tree(&[]).is_empty());
    assert_eq!(
        totals(&[]),
        ParameterTotals {
            total: Some(0),
            by_dtype: Vec::new(),
        }
    );
}
