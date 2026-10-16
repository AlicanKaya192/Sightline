//! A small Protocol Buffers writer for ONNX test files. Field numbers from onnx.proto.

pub fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let low = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            bytes.push(low);
            return bytes;
        }
        bytes.push(low | 0x80);
    }
}

pub fn varint_field(number: u64, value: u64) -> Vec<u8> {
    let mut bytes = varint(number << 3);
    bytes.extend(varint(value));
    bytes
}

pub fn bytes_field(number: u64, content: &[u8]) -> Vec<u8> {
    let mut bytes = varint(number << 3 | 2);
    bytes.extend(varint(content.len() as u64));
    bytes.extend_from_slice(content);
    bytes
}

pub fn text_field(number: u64, text: &str) -> Vec<u8> {
    bytes_field(number, text.as_bytes())
}

/// `TensorProto` with packed dims and `raw_bytes` zero bytes of `raw_data`.
pub fn tensor(name: &str, data_type: u64, dims: &[u64], raw_bytes: usize) -> Vec<u8> {
    let packed: Vec<u8> = dims.iter().flat_map(|&d| varint(d)).collect();
    let mut bytes = bytes_field(1, &packed);
    bytes.extend(varint_field(2, data_type));
    bytes.extend(text_field(8, name));
    bytes.extend(bytes_field(9, &vec![0; raw_bytes]));
    bytes
}

/// `TensorProto` whose weights are in another file (`data_location = EXTERNAL`).
pub fn external_tensor(name: &str, data_type: u64, dims: &[u64]) -> Vec<u8> {
    let mut bytes: Vec<u8> = dims.iter().flat_map(|&d| varint_field(1, d)).collect();
    bytes.extend(varint_field(2, data_type));
    bytes.extend(text_field(8, name));
    let mut location = text_field(1, "location");
    location.extend(text_field(2, "weights.bin"));
    bytes.extend(bytes_field(13, &location));
    bytes.extend(varint_field(14, 1));
    bytes
}

pub enum Dim {
    Value(u64),
    Param(&'static str),
    Unknown,
}

/// `ValueInfoProto` of a tensor.
pub fn value_info(name: &str, elem_type: u64, dims: &[Dim]) -> Vec<u8> {
    let mut shape = Vec::new();
    for dim in dims {
        let content = match dim {
            Dim::Value(value) => varint_field(1, *value),
            Dim::Param(param) => text_field(2, param),
            Dim::Unknown => Vec::new(),
        };
        shape.extend(bytes_field(1, &content));
    }
    let mut tensor_type = varint_field(1, elem_type);
    tensor_type.extend(bytes_field(2, &shape));
    let mut bytes = text_field(1, name);
    bytes.extend(bytes_field(2, &bytes_field(1, &tensor_type)));
    bytes
}

pub fn node(op_type: &str) -> Vec<u8> {
    let mut bytes = text_field(1, "x");
    bytes.extend(text_field(2, "y"));
    bytes.extend(text_field(4, op_type));
    bytes
}

pub struct Graph {
    pub nodes: Vec<Vec<u8>>,
    pub initializers: Vec<Vec<u8>>,
    pub inputs: Vec<Vec<u8>>,
    pub outputs: Vec<Vec<u8>>,
}

pub fn graph(graph: &Graph) -> Vec<u8> {
    let mut bytes = text_field(2, "main");
    for node in &graph.nodes {
        bytes.extend(bytes_field(1, node));
    }
    for tensor in &graph.initializers {
        bytes.extend(bytes_field(5, tensor));
    }
    for input in &graph.inputs {
        bytes.extend(bytes_field(11, input));
    }
    for output in &graph.outputs {
        bytes.extend(bytes_field(12, output));
    }
    bytes
}

/// `ModelProto` with IR version 8, opsets `ai.onnx` 17 and `ai.onnx.ml` 3, made by pytorch 2.1.
pub fn model(graph_bytes: &[u8]) -> Vec<u8> {
    let mut bytes = varint_field(1, 8);
    bytes.extend(text_field(2, "pytorch"));
    bytes.extend(text_field(3, "2.1"));
    bytes.extend(bytes_field(7, graph_bytes));
    let mut default_set = text_field(1, "");
    default_set.extend(varint_field(2, 17));
    bytes.extend(bytes_field(8, &default_set));
    let mut ml_set = text_field(1, "ai.onnx.ml");
    ml_set.extend(varint_field(2, 3));
    bytes.extend(bytes_field(8, &ml_set));
    bytes
}
