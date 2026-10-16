//! ONNX: a Protocol Buffers `ModelProto`, read field by field. Weights (`raw_data` and the
//! typed data fields) are skipped, never read; weights in external files are not opened.
//! Field numbers from the official `onnx/onnx.proto`.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use super::protobuf::{Message, Value, Wire, WireError};
use super::{Limits, ReadError, Reader, has_extension};
use crate::summary::{
    DType, Dimension, FileWarning, ModelFormat, ModelSummary, OnnxInfo, SizeCheck, Summary,
    TensorInfo, ValueInfo,
};

const FORMAT: &str = "ONNX";
/// Longer names than this are not real tensor, operator or domain names.
const MAX_TEXT_BYTES: u64 = 65_536;
/// More dimensions than any tensor has.
const MAX_DIMS: usize = 64;
/// `TensorProto.DataLocation.EXTERNAL`.
const EXTERNAL_LOCATION: u64 = 1;

mod field {
    pub const MODEL_IR_VERSION: u64 = 1;
    pub const MODEL_PRODUCER_NAME: u64 = 2;
    pub const MODEL_PRODUCER_VERSION: u64 = 3;
    pub const MODEL_GRAPH: u64 = 7;
    pub const MODEL_OPSET_IMPORT: u64 = 8;
    pub const OPSET_DOMAIN: u64 = 1;
    pub const OPSET_VERSION: u64 = 2;
    pub const GRAPH_NODE: u64 = 1;
    pub const GRAPH_INITIALIZER: u64 = 5;
    pub const GRAPH_INPUT: u64 = 11;
    pub const GRAPH_OUTPUT: u64 = 12;
    pub const NODE_OP_TYPE: u64 = 4;
    pub const TENSOR_DIMS: u64 = 1;
    pub const TENSOR_DATA_TYPE: u64 = 2;
    pub const TENSOR_NAME: u64 = 8;
    pub const TENSOR_RAW_DATA: u64 = 9;
    pub const TENSOR_EXTERNAL_DATA: u64 = 13;
    pub const TENSOR_DATA_LOCATION: u64 = 14;
    pub const VALUE_NAME: u64 = 1;
    pub const VALUE_TYPE: u64 = 2;
    pub const TYPE_TENSOR: u64 = 1;
    pub const TENSOR_TYPE_ELEM_TYPE: u64 = 1;
    pub const TENSOR_TYPE_SHAPE: u64 = 2;
    pub const SHAPE_DIM: u64 = 1;
    pub const DIM_VALUE: u64 = 1;
    pub const DIM_PARAM: u64 = 2;
}

pub struct OnnxReader;

impl Reader for OnnxReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "onnx")
    }

    fn summarize(&self, path: &Path, _limits: &Limits) -> Result<Summary, ReadError> {
        let file = File::open(path)?;
        let file_bytes = file.metadata()?.len();
        let mut wire = Wire::new(file);
        let root = wire.root(file_bytes);
        let model = read_model(&mut wire, &root).map_err(|error| malformed(error.0))?;
        let graph = model
            .graph
            .ok_or_else(|| malformed("the file has no graph"))?;
        Ok(Summary::Model(summary(model.info, graph)))
    }
}

fn malformed(reason: impl Into<String>) -> ReadError {
    ReadError::Malformed {
        format: FORMAT,
        reason: reason.into(),
    }
}

struct Model {
    info: OnnxInfo,
    graph: Option<Graph>,
}

#[derive(Default)]
struct Graph {
    operators: HashMap<String, u64>,
    initializers: Vec<Initializer>,
    inputs: Vec<ValueInfo>,
    outputs: Vec<ValueInfo>,
}

struct Initializer {
    info: TensorInfo,
    /// Length of `raw_data`, when the weights are stored that way.
    raw_bytes: Option<u64>,
    external: bool,
}

type Wired = Wire<File>;

fn read_model(wire: &mut Wired, root: &Message) -> Result<Model, WireError> {
    let mut info = OnnxInfo {
        ir_version: 0,
        opsets: Vec::new(),
        producer: String::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        operators: Vec::new(),
    };
    let (mut producer_name, mut producer_version) = (String::new(), String::new());
    let mut graph = None;
    while let Some((number, value)) = wire.next(root)? {
        match (number, value) {
            (field::MODEL_IR_VERSION, Value::Varint(version)) => info.ir_version = version as i64,
            (field::MODEL_PRODUCER_NAME, Value::Bytes(length)) => {
                producer_name = wire.text(length, MAX_TEXT_BYTES)?;
            }
            (field::MODEL_PRODUCER_VERSION, Value::Bytes(length)) => {
                producer_version = wire.text(length, MAX_TEXT_BYTES)?;
            }
            (field::MODEL_OPSET_IMPORT, Value::Bytes(length)) => {
                let message = wire.enter(root, length)?;
                info.opsets.push(read_opset(wire, &message)?);
            }
            (field::MODEL_GRAPH, Value::Bytes(length)) => {
                let message = wire.enter(root, length)?;
                graph = Some(read_graph(wire, &message)?);
            }
            _ => {}
        }
    }
    info.producer = format!("{producer_name} {producer_version}")
        .trim()
        .to_string();
    Ok(Model { info, graph })
}

fn read_opset(wire: &mut Wired, message: &Message) -> Result<(String, i64), WireError> {
    let (mut domain, mut version) = (String::new(), 0);
    while let Some((number, value)) = wire.next(message)? {
        match (number, value) {
            (field::OPSET_DOMAIN, Value::Bytes(length)) => {
                domain = wire.text(length, MAX_TEXT_BYTES)?;
            }
            (field::OPSET_VERSION, Value::Varint(number)) => version = number as i64,
            _ => {}
        }
    }
    Ok((domain, version))
}

fn read_graph(wire: &mut Wired, message: &Message) -> Result<Graph, WireError> {
    let mut graph = Graph::default();
    while let Some((number, value)) = wire.next(message)? {
        let Value::Bytes(length) = value else {
            continue;
        };
        match number {
            field::GRAPH_NODE => {
                let node = wire.enter(message, length)?;
                if let Some(op_type) = read_op_type(wire, &node)? {
                    *graph.operators.entry(op_type).or_insert(0) += 1;
                }
            }
            field::GRAPH_INITIALIZER => {
                let tensor = wire.enter(message, length)?;
                graph.initializers.push(read_initializer(wire, &tensor)?);
            }
            field::GRAPH_INPUT => {
                let value_info = wire.enter(message, length)?;
                graph.inputs.push(read_value_info(wire, &value_info)?);
            }
            field::GRAPH_OUTPUT => {
                let value_info = wire.enter(message, length)?;
                graph.outputs.push(read_value_info(wire, &value_info)?);
            }
            _ => {}
        }
    }
    Ok(graph)
}

fn read_op_type(wire: &mut Wired, node: &Message) -> Result<Option<String>, WireError> {
    let mut op_type = None;
    while let Some((number, value)) = wire.next(node)? {
        if let (field::NODE_OP_TYPE, Value::Bytes(length)) = (number, value) {
            op_type = Some(wire.text(length, MAX_TEXT_BYTES)?);
        }
    }
    Ok(op_type)
}

fn read_initializer(wire: &mut Wired, tensor: &Message) -> Result<Initializer, WireError> {
    let mut info = TensorInfo {
        name: String::new(),
        dtype: DType::Other("UNDEFINED".to_string()),
        shape: Vec::new(),
    };
    let (mut raw_bytes, mut external) = (None, false);
    while let Some((number, value)) = wire.next(tensor)? {
        match (number, value) {
            // `dims` may be packed or written one varint at a time.
            (field::TENSOR_DIMS, Value::Varint(size)) => push_dim(&mut info.shape, size)?,
            (field::TENSOR_DIMS, Value::Bytes(length)) => {
                for size in wire.packed_varints(length, MAX_DIMS)? {
                    push_dim(&mut info.shape, size)?;
                }
            }
            (field::TENSOR_DATA_TYPE, Value::Varint(code)) => info.dtype = dtype(code),
            (field::TENSOR_NAME, Value::Bytes(length)) => {
                info.name = wire.text(length, MAX_TEXT_BYTES)?;
            }
            (field::TENSOR_RAW_DATA, Value::Bytes(length)) => raw_bytes = Some(length),
            (field::TENSOR_EXTERNAL_DATA, Value::Bytes(_)) => external = true,
            (field::TENSOR_DATA_LOCATION, Value::Varint(location)) => {
                external |= location == EXTERNAL_LOCATION;
            }
            _ => {}
        }
    }
    Ok(Initializer {
        info,
        raw_bytes,
        external,
    })
}

fn push_dim(shape: &mut Vec<u64>, size: u64) -> Result<(), WireError> {
    if shape.len() == MAX_DIMS {
        return Err(WireError(format!(
            "a tensor has more than {MAX_DIMS} dimensions"
        )));
    }
    shape.push(size);
    Ok(())
}

fn read_value_info(wire: &mut Wired, message: &Message) -> Result<ValueInfo, WireError> {
    let mut value_info = ValueInfo {
        name: String::new(),
        dtype: None,
        shape: Vec::new(),
    };
    while let Some((number, value)) = wire.next(message)? {
        match (number, value) {
            (field::VALUE_NAME, Value::Bytes(length)) => {
                value_info.name = wire.text(length, MAX_TEXT_BYTES)?;
            }
            (field::VALUE_TYPE, Value::Bytes(length)) => {
                let type_proto = wire.enter(message, length)?;
                read_type(wire, &type_proto, &mut value_info)?;
            }
            _ => {}
        }
    }
    Ok(value_info)
}

/// Element type and shape of a tensor type; other kinds (sequences, maps) leave both empty.
fn read_type(
    wire: &mut Wired,
    message: &Message,
    value_info: &mut ValueInfo,
) -> Result<(), WireError> {
    while let Some((number, value)) = wire.next(message)? {
        let (field::TYPE_TENSOR, Value::Bytes(length)) = (number, value) else {
            continue;
        };
        let tensor_type = wire.enter(message, length)?;
        while let Some((number, value)) = wire.next(&tensor_type)? {
            match (number, value) {
                (field::TENSOR_TYPE_ELEM_TYPE, Value::Varint(code)) => {
                    value_info.dtype = Some(dtype(code));
                }
                (field::TENSOR_TYPE_SHAPE, Value::Bytes(length)) => {
                    let shape = wire.enter(&tensor_type, length)?;
                    value_info.shape = read_shape(wire, &shape)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn read_shape(wire: &mut Wired, message: &Message) -> Result<Vec<Dimension>, WireError> {
    let mut dimensions = Vec::new();
    while let Some((number, value)) = wire.next(message)? {
        let (field::SHAPE_DIM, Value::Bytes(length)) = (number, value) else {
            continue;
        };
        if dimensions.len() == MAX_DIMS {
            return Err(WireError(format!(
                "a shape has more than {MAX_DIMS} dimensions"
            )));
        }
        let dimension = wire.enter(message, length)?;
        let mut found = Dimension::Unknown;
        while let Some((number, value)) = wire.next(&dimension)? {
            match (number, value) {
                (field::DIM_VALUE, Value::Varint(size)) => found = Dimension::Fixed(size),
                (field::DIM_PARAM, Value::Bytes(length)) => {
                    found = Dimension::Named(wire.text(length, MAX_TEXT_BYTES)?);
                }
                _ => {}
            }
        }
        dimensions.push(found);
    }
    Ok(dimensions)
}

/// `TensorProto.DataType` codes.
fn dtype(code: u64) -> DType {
    match code {
        1 => DType::F32,
        2 => DType::U8,
        3 => DType::I8,
        4 => DType::U16,
        5 => DType::I16,
        6 => DType::I32,
        7 => DType::I64,
        8 => DType::Text,
        9 => DType::Bool,
        10 => DType::F16,
        11 => DType::F64,
        12 => DType::U32,
        13 => DType::U64,
        14 => DType::Complex64,
        15 => DType::Complex128,
        16 => DType::BF16,
        17 => DType::F8E4M3,
        19 => DType::F8E5M2,
        18 => DType::Other("FLOAT8E4M3FNUZ".to_string()),
        20 => DType::Other("FLOAT8E5M2FNUZ".to_string()),
        21 => DType::Other("UINT4".to_string()),
        22 => DType::Other("INT4".to_string()),
        23 => DType::Other("FLOAT4E2M1".to_string()),
        24 => DType::Other("FLOAT8E8M0".to_string()),
        25 => DType::Other("UINT2".to_string()),
        26 => DType::Other("INT2".to_string()),
        27 => DType::Other("FLOAT6E2M3".to_string()),
        28 => DType::Other("FLOAT6E3M2".to_string()),
        other => DType::Other(format!("type {other}")),
    }
}

fn summary(mut info: OnnxInfo, graph: Graph) -> ModelSummary {
    let initializer_names: HashSet<&str> = graph
        .initializers
        .iter()
        .map(|tensor| tensor.info.name.as_str())
        .collect();
    // Before IR version 4, graph inputs also list every initializer.
    info.inputs = graph
        .inputs
        .into_iter()
        .filter(|input| !initializer_names.contains(input.name.as_str()))
        .collect();
    info.outputs = graph.outputs;
    let mut operators: Vec<(String, u64)> = graph.operators.into_iter().collect();
    operators.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    info.operators = operators;
    let external = graph.initializers.iter().filter(|t| t.external).count() as u64;
    let size_check = size_check(&graph.initializers);
    ModelSummary {
        format: ModelFormat::Onnx(info),
        tensors: graph.initializers.into_iter().map(|t| t.info).collect(),
        size_check,
        warnings: if external > 0 {
            vec![FileWarning::ExternalData { tensors: external }]
        } else {
            Vec::new()
        },
    }
}

/// Weights stored as `raw_data` must be as long as their shape and type say. Weights kept
/// elsewhere (typed fields, external files) cannot be checked this way.
fn size_check(initializers: &[Initializer]) -> SizeCheck {
    let mut all_checked = true;
    for tensor in initializers {
        match (tensor.raw_bytes, tensor.info.bytes()) {
            (Some(stored), Some(expected)) if !tensor.external => {
                if stored != expected {
                    return SizeCheck::BadOffsets {
                        tensor: tensor.info.name.clone(),
                    };
                }
            }
            _ => all_checked = false,
        }
    }
    if all_checked {
        SizeCheck::Consistent
    } else {
        SizeCheck::Unknown
    }
}
