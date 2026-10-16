//! Layer tree and parameter totals from tensor names such as `model.layers.0.mlp.weight`.

use std::collections::HashMap;

use crate::summary::{DType, TensorInfo};

/// One module of a model, found from the dotted tensor names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerNode {
    /// Dotted name from the root, such as `model.layers.0`.
    pub path: String,
    pub depth: usize,
    pub tensors: u64,
    /// Values in this module and everything under it; `None` beyond 64 bits.
    pub parameters: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterTotals {
    /// `None` when the count does not fit in 64 bits.
    pub total: Option<u64>,
    /// Values per element type, most first.
    pub by_dtype: Vec<(DType, u64)>,
}

/// Modules in depth-first order, children in the order their first tensor is stored.
pub fn tree(tensors: &[TensorInfo]) -> Vec<LayerNode> {
    let mut root = Module::default();
    for tensor in tensors {
        root.add(module_path(&tensor.name), tensor.elements());
    }
    let mut nodes = Vec::new();
    for child in &root.children {
        child.flatten("", 0, &mut nodes);
    }
    nodes
}

pub fn totals(tensors: &[TensorInfo]) -> ParameterTotals {
    let total = tensors
        .iter()
        .try_fold(0u64, |sum, tensor| sum.checked_add(tensor.elements()?));
    let mut counts: HashMap<&DType, u64> = HashMap::new();
    for tensor in tensors {
        let count = counts.entry(&tensor.dtype).or_insert(0);
        *count = count.saturating_add(tensor.elements().unwrap_or(u64::MAX));
    }
    let mut by_dtype: Vec<(DType, u64)> = counts
        .into_iter()
        .map(|(dtype, count)| (dtype.clone(), count))
        .collect();
    by_dtype.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ParameterTotals { total, by_dtype }
}

/// The module a tensor belongs to: its name without the last part, or the whole name
/// when it has no dots.
fn module_path(name: &str) -> Vec<&str> {
    let parts: Vec<&str> = name.split('.').collect();
    match parts.split_last() {
        Some((_, [])) | None => parts,
        Some((_, module)) => module.to_vec(),
    }
}

#[derive(Default)]
struct Module {
    name: String,
    tensors: u64,
    parameters: Option<u64>,
    children: Vec<Module>,
}

impl Module {
    fn add(&mut self, path: Vec<&str>, elements: Option<u64>) {
        let mut module = self;
        for part in path {
            let index = match module.children.iter().position(|child| child.name == part) {
                Some(index) => index,
                None => {
                    module.children.push(Module {
                        name: part.to_string(),
                        parameters: Some(0),
                        ..Module::default()
                    });
                    module.children.len() - 1
                }
            };
            module = &mut module.children[index];
            module.tensors += 1;
            module.parameters = module
                .parameters
                .zip(elements)
                .and_then(|(sum, count)| sum.checked_add(count));
        }
    }

    fn flatten(&self, parent: &str, depth: usize, nodes: &mut Vec<LayerNode>) {
        let path = if parent.is_empty() {
            self.name.clone()
        } else {
            format!("{parent}.{}", self.name)
        };
        nodes.push(LayerNode {
            path: path.clone(),
            depth,
            tensors: self.tensors,
            parameters: self.parameters,
        });
        for child in &self.children {
            child.flatten(&path, depth + 1, nodes);
        }
    }
}
