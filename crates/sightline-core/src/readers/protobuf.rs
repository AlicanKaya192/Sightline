//! Protocol Buffers wire format, read forward from a file. Length-delimited fields that
//! are not needed (such as model weights) are skipped with a seek instead of being read.

use std::fmt;
use std::io::{self, BufReader, Read, Seek};

/// Deeper than any ONNX model nests; stops a crafted file from recursing without end.
pub const MAX_DEPTH: usize = 64;
const MAX_VARINT_BYTES: usize = 10;

#[derive(Debug)]
pub struct WireError(pub String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<io::Error> for WireError {
    fn from(error: io::Error) -> Self {
        Self(format!("the file ends inside a field ({error})"))
    }
}

/// The value of one field; a `Bytes` value is left in the file until it is read, entered
/// or skipped, and skipped on its own when the next field is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Varint(u64),
    Fixed,
    Bytes(u64),
}

/// The byte range of one message.
#[derive(Debug, Clone, Copy)]
pub struct Message {
    end: u64,
    depth: usize,
}

pub struct Wire<R> {
    reader: BufReader<R>,
    position: u64,
    /// End of the last `Bytes` value, until it is consumed.
    pending: Option<u64>,
}

impl<R: Read + Seek> Wire<R> {
    pub fn new(input: R) -> Self {
        Self {
            reader: BufReader::new(input),
            position: 0,
            pending: None,
        }
    }

    /// The outermost message, which ends where the input does.
    pub fn root(&self, length: u64) -> Message {
        Message {
            end: length,
            depth: 0,
        }
    }

    /// The next field of `message` as (field number, value); `None` at its end.
    pub fn next(&mut self, message: &Message) -> Result<Option<(u64, Value)>, WireError> {
        if let Some(end) = self.pending.take() {
            self.skip_to(end)?;
        }
        if self.position >= message.end {
            return Ok(None);
        }
        let tag = self.varint()?;
        let number = tag >> 3;
        let value = match tag & 0b111 {
            0 => Value::Varint(self.varint()?),
            1 => {
                self.skip(8, message)?;
                Value::Fixed
            }
            5 => {
                self.skip(4, message)?;
                Value::Fixed
            }
            2 => {
                let length = self.varint()?;
                let end = self.end_within(length, message)?;
                self.pending = Some(end);
                Value::Bytes(length)
            }
            other => return Err(WireError(format!("wire type {other} is not supported"))),
        };
        if self.position > message.end {
            return Err(WireError(
                "a field runs past the end of its message".to_string(),
            ));
        }
        Ok(Some((number, value)))
    }

    /// The `Bytes` value just returned, read as a message of its own.
    pub fn enter(&mut self, parent: &Message, length: u64) -> Result<Message, WireError> {
        if parent.depth + 1 > MAX_DEPTH {
            return Err(WireError("messages are nested too deeply".to_string()));
        }
        self.pending = None;
        Ok(Message {
            end: self.position + length,
            depth: parent.depth + 1,
        })
    }

    /// The `Bytes` value just returned as text; longer values than `max` are an error.
    pub fn text(&mut self, length: u64, max: u64) -> Result<String, WireError> {
        if length > max {
            return Err(WireError(format!(
                "a text field of {length} bytes is over the limit of {max}"
            )));
        }
        self.pending = None;
        let mut bytes = vec![0; length as usize];
        self.reader.read_exact(&mut bytes)?;
        self.position += length;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The `Bytes` value just returned as packed varints, at most `max` of them.
    pub fn packed_varints(&mut self, length: u64, max: usize) -> Result<Vec<u64>, WireError> {
        self.pending = None;
        let end = self.position + length;
        let mut values = Vec::new();
        while self.position < end {
            if values.len() == max {
                return Err(WireError(format!("more than {max} packed values")));
            }
            values.push(self.varint()?);
        }
        if self.position != end {
            return Err(WireError("packed values run past their field".to_string()));
        }
        Ok(values)
    }

    fn varint(&mut self) -> Result<u64, WireError> {
        let mut value = 0u64;
        for index in 0..MAX_VARINT_BYTES {
            let mut byte = [0];
            self.reader.read_exact(&mut byte)?;
            self.position += 1;
            let bits = u64::from(byte[0] & 0x7F);
            if index == MAX_VARINT_BYTES - 1 && bits > 1 {
                return Err(WireError("a varint does not fit in 64 bits".to_string()));
            }
            value |= bits << (7 * index);
            if byte[0] & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(WireError("a varint is longer than 10 bytes".to_string()))
    }

    fn end_within(&self, length: u64, message: &Message) -> Result<u64, WireError> {
        self.position
            .checked_add(length)
            .filter(|&end| end <= message.end)
            .ok_or_else(|| WireError("a field runs past the end of its message".to_string()))
    }

    fn skip(&mut self, length: u64, message: &Message) -> Result<(), WireError> {
        let end = self.end_within(length, message)?;
        self.skip_to(end)
    }

    fn skip_to(&mut self, end: u64) -> Result<(), WireError> {
        let offset = i64::try_from(end - self.position)
            .map_err(|_| WireError("a field is too long".to_string()))?;
        self.reader.seek_relative(offset)?;
        self.position = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn varint(mut value: u64) -> Vec<u8> {
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

    fn tag(number: u64, wire_type: u64) -> Vec<u8> {
        varint(number << 3 | wire_type)
    }

    fn bytes_field(number: u64, content: &[u8]) -> Vec<u8> {
        let mut bytes = tag(number, 2);
        bytes.extend(varint(content.len() as u64));
        bytes.extend_from_slice(content);
        bytes
    }

    fn wire(bytes: &[u8]) -> (Wire<Cursor<Vec<u8>>>, Message) {
        let wire = Wire::new(Cursor::new(bytes.to_vec()));
        let root = wire.root(bytes.len() as u64);
        (wire, root)
    }

    #[test]
    fn reads_varint_and_text_fields() {
        let mut bytes = tag(1, 0);
        bytes.extend(varint(300));
        bytes.extend(bytes_field(2, b"onnx"));
        let (mut wire, root) = wire(&bytes);

        assert_eq!(wire.next(&root).unwrap(), Some((1, Value::Varint(300))));
        assert_eq!(wire.next(&root).unwrap(), Some((2, Value::Bytes(4))));
        assert_eq!(wire.text(4, 100).unwrap(), "onnx");
        assert_eq!(wire.next(&root).unwrap(), None);
    }

    #[test]
    fn an_unread_bytes_field_is_skipped() {
        let mut bytes = bytes_field(9, &[7; 1000]);
        bytes.extend(tag(3, 0));
        bytes.extend(varint(5));
        let (mut wire, root) = wire(&bytes);

        assert_eq!(wire.next(&root).unwrap(), Some((9, Value::Bytes(1000))));
        assert_eq!(wire.next(&root).unwrap(), Some((3, Value::Varint(5))));
    }

    #[test]
    fn fixed_width_fields_are_skipped() {
        let mut bytes = tag(1, 1);
        bytes.extend([0; 8]);
        bytes.extend(tag(2, 5));
        bytes.extend([0; 4]);
        bytes.extend(tag(3, 0));
        bytes.extend(varint(1));
        let (mut wire, root) = wire(&bytes);

        assert_eq!(wire.next(&root).unwrap(), Some((1, Value::Fixed)));
        assert_eq!(wire.next(&root).unwrap(), Some((2, Value::Fixed)));
        assert_eq!(wire.next(&root).unwrap(), Some((3, Value::Varint(1))));
    }

    #[test]
    fn a_nested_message_ends_at_its_length() {
        let mut inner = tag(1, 0);
        inner.extend(varint(7));
        let mut bytes = bytes_field(5, &inner);
        bytes.extend(tag(6, 0));
        bytes.extend(varint(8));
        let (mut wire, root) = wire(&bytes);

        let Some((5, Value::Bytes(length))) = wire.next(&root).unwrap() else {
            panic!("expected the nested message");
        };
        let nested = wire.enter(&root, length).unwrap();
        assert_eq!(wire.next(&nested).unwrap(), Some((1, Value::Varint(7))));
        assert_eq!(wire.next(&nested).unwrap(), None);
        assert_eq!(wire.next(&root).unwrap(), Some((6, Value::Varint(8))));
    }

    #[test]
    fn packed_varints_are_read() {
        let content: Vec<u8> = [1u64, 300, 70_000].into_iter().flat_map(varint).collect();
        let bytes = bytes_field(1, &content);
        let (mut wire, root) = wire(&bytes);

        let Some((1, Value::Bytes(length))) = wire.next(&root).unwrap() else {
            panic!("expected packed values");
        };
        assert_eq!(
            wire.packed_varints(length, 10).unwrap(),
            vec![1, 300, 70_000]
        );
    }

    #[test]
    fn a_length_past_the_end_is_an_error() {
        let mut bytes = tag(1, 2);
        bytes.extend(varint(1_000_000));
        bytes.extend([0; 3]);
        let (mut wire, root) = wire(&bytes);

        assert!(wire.next(&root).is_err());
    }

    #[test]
    fn a_varint_longer_than_64_bits_is_an_error() {
        let mut bytes = tag(1, 0);
        bytes.extend([0xFF; 11]);
        let (mut wire, root) = wire(&bytes);

        assert!(wire.next(&root).is_err());
    }

    #[test]
    fn a_ten_byte_varint_with_bits_beyond_64_is_an_error() {
        let mut bytes = tag(1, 0);
        bytes.extend([0xFF; 9]);
        bytes.push(0x02);
        let (mut wire, root) = wire(&bytes);

        assert!(wire.next(&root).is_err());
    }

    #[test]
    fn group_wire_types_are_an_error() {
        let (mut wire, root) = wire(&tag(1, 3));

        assert!(wire.next(&root).is_err());
    }

    #[test]
    fn text_over_the_limit_is_an_error() {
        let bytes = bytes_field(1, b"too long");
        let (mut wire, root) = wire(&bytes);

        wire.next(&root).unwrap();
        assert!(wire.text(8, 4).is_err());
    }

    #[test]
    fn nesting_deeper_than_the_limit_is_an_error() {
        let mut bytes = Vec::new();
        for _ in 0..=MAX_DEPTH {
            bytes = bytes_field(1, &bytes);
        }
        let (mut wire, root) = wire(&bytes);

        let mut message = root;
        let error = loop {
            match wire.next(&message).unwrap() {
                Some((1, Value::Bytes(length))) => match wire.enter(&message, length) {
                    Ok(nested) => message = nested,
                    Err(error) => break error,
                },
                other => panic!("unexpected field {other:?}"),
            }
        };
        assert!(error.0.contains("nested"), "{error}");
    }
}
