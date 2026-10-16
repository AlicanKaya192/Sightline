//! Pickle: lists what a pickle would import and call, from its opcodes alone.
//!
//! No unpickler is used and no object is built. The opcodes are walked with a stack that
//! only knows "a short string", "something else" and "a mark", which is enough to learn
//! the names `STACK_GLOBAL` imports. Opcode table: CPython's `pickletools` (protocols 0–5).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek};
use std::path::Path;

use super::{Limits, ReadError, Reader, has_extension};
use crate::summary::{FileWarning, PickleImport, PickleSummary, Summary};

const FORMAT: &str = "pickle";
const MAX_OPCODES: u64 = 10_000_000;
const MAX_STACK: usize = 1_000_000;
const MAX_MEMO_TEXTS: usize = 100_000;
/// Longer strings cannot be the names of modules or functions; they are skipped, not kept.
const MAX_NAME_BYTES: u64 = 256;
const UNKNOWN_NAME: &str = "?";

/// Every function of these modules can run commands, touch files or the network, or load code.
const DANGEROUS_MODULES: [&str; 24] = [
    "os",
    "posix",
    "nt",
    "subprocess",
    "_posixsubprocess",
    "sys",
    "socket",
    "shutil",
    "runpy",
    "importlib",
    "ctypes",
    "pty",
    "webbrowser",
    "commands",
    "multiprocessing",
    "code",
    "codeop",
    "pdb",
    "timeit",
    "pickle",
    "_pickle",
    "marshal",
    "asyncio",
    "urllib",
];
const DANGEROUS_BUILTINS: [&str; 17] = [
    "eval",
    "exec",
    "compile",
    "open",
    "__import__",
    "getattr",
    "setattr",
    "delattr",
    "globals",
    "locals",
    "vars",
    "breakpoint",
    "input",
    "execfile",
    "file",
    "apply",
    "reload",
];
/// Functions that reach any attribute or method, and so any function.
const DANGEROUS_FUNCTIONS: [(&str, &str); 2] =
    [("operator", "attrgetter"), ("operator", "methodcaller")];

pub struct PickleReader;

impl Reader for PickleReader {
    fn can_read(&self, path: &Path) -> bool {
        has_extension(path, "pkl") || has_extension(path, "pickle")
    }

    fn summarize(&self, path: &Path, _limits: &Limits) -> Result<Summary, ReadError> {
        let file = File::open(path)?;
        let file_bytes = file.metadata()?.len();
        let mut walk = Walk::default();
        let finished = walk.run(&mut Input::new(file, file_bytes))?;
        if walk.opcodes == 0 {
            return Err(malformed("the file is empty"));
        }
        let mut warnings = vec![FileWarning::RunsCodeWhenLoaded];
        warnings.extend(
            walk.imports
                .iter()
                .filter(|import| import.dangerous)
                .map(|import| FileWarning::DangerousImport {
                    module: import.module.clone(),
                    name: import.name.clone(),
                }),
        );
        if !finished {
            warnings.push(FileWarning::StoppedEarly);
        }
        Ok(Summary::Pickle(PickleSummary {
            protocol: walk.protocol,
            opcodes: walk.opcodes,
            imports: walk.imports,
            calls: walk.calls,
            warnings,
        }))
    }
}

fn malformed(reason: impl Into<String>) -> ReadError {
    ReadError::Malformed {
        format: FORMAT,
        reason: reason.into(),
    }
}

/// Why the walk ended before the end of the file.
enum Halt {
    /// The file ends inside an opcode.
    Cut,
    UnknownOpcode,
    /// An opcode needs more on the stack than there is; loading would fail here too.
    StackUnderflow,
    Limit,
}

impl From<std::io::Error> for Halt {
    fn from(_: std::io::Error) -> Self {
        Self::Cut
    }
}

#[derive(Clone)]
enum Item {
    Mark,
    Text(String),
    Other,
}

#[derive(Default)]
struct Walk {
    protocol: Option<u8>,
    opcodes: u64,
    imports: Vec<PickleImport>,
    calls: u64,
    stack: Vec<Item>,
    memo_texts: HashMap<u64, String>,
    /// Entries the memo would hold; `MEMOIZE` stores at this index.
    memo_len: u64,
}

impl Walk {
    /// Walks every pickle in the file; `false` when it stopped before the end.
    fn run(&mut self, input: &mut Input) -> Result<bool, ReadError> {
        let mut after_stop = true;
        loop {
            let code = match input.byte() {
                Ok(code) => code,
                Err(_) => return Ok(after_stop),
            };
            if self.opcodes == MAX_OPCODES {
                return Ok(false);
            }
            match self.step(code, input) {
                Ok(stopped) => {
                    self.opcodes += 1;
                    after_stop = stopped;
                    if stopped {
                        self.stack.clear();
                        self.memo_texts.clear();
                        self.memo_len = 0;
                    }
                }
                Err(Halt::UnknownOpcode) if self.opcodes == 0 => {
                    return Err(malformed("the file does not start with a pickle opcode"));
                }
                Err(_) => return Ok(false),
            }
        }
    }

    /// Runs one opcode on the stack; `true` for `STOP`.
    fn step(&mut self, code: u8, input: &mut Input) -> Result<bool, Halt> {
        match code {
            b'.' => return Ok(true),
            b'(' => self.push(Item::Mark)?,
            // Numbers, bytes and constants: skipped, something else on the stack.
            b'I' | b'L' | b'F' => {
                input.line()?;
                self.push(Item::Other)?;
            }
            b'J' => self.skip_and_push(input, 4)?,
            b'K' => self.skip_and_push(input, 1)?,
            b'M' => self.skip_and_push(input, 2)?,
            b'G' => self.skip_and_push(input, 8)?,
            0x8a => {
                let length = input.unsigned(1)?;
                self.skip_and_push(input, length)?;
            }
            0x8b => {
                let length = input.signed4()?;
                self.skip_and_push(input, length)?;
            }
            b'B' => {
                let length = input.unsigned(4)?;
                self.skip_and_push(input, length)?;
            }
            b'C' => {
                let length = input.unsigned(1)?;
                self.skip_and_push(input, length)?;
            }
            0x8e | 0x96 => {
                let length = input.unsigned(8)?;
                self.skip_and_push(input, length)?;
            }
            b'N' | 0x88 | 0x89 | b']' | b')' | b'}' | 0x8f | 0x97 => self.push(Item::Other)?,
            // Strings: kept when short enough to be a name.
            b'S' => {
                let line = input.line()?;
                let text = line.map(|line| line.trim_matches(['\'', '"']).to_string());
                self.push(text.map_or(Item::Other, Item::Text))?;
            }
            b'V' => {
                let text = input.line()?;
                self.push(text.map_or(Item::Other, Item::Text))?;
            }
            b'T' => {
                let length = input.signed4()?;
                self.push_text(input, length)?;
            }
            b'U' | 0x8c => {
                let length = input.unsigned(1)?;
                self.push_text(input, length)?;
            }
            b'X' => {
                let length = input.unsigned(4)?;
                self.push_text(input, length)?;
            }
            0x8d => {
                let length = input.unsigned(8)?;
                self.push_text(input, length)?;
            }
            // Containers.
            b'l' | b't' | b'd' | 0x91 => {
                self.pop_to_mark()?;
                self.push(Item::Other)?;
            }
            b'e' | b'u' | 0x90 | b'1' => self.pop_to_mark()?,
            b'a' | b'0' => self.pop(1)?,
            b's' => self.pop(2)?,
            b'2' => {
                let top = self.stack.last().cloned().ok_or(Halt::StackUnderflow)?;
                self.push(top)?;
            }
            0x85 => self.replace(1)?,
            0x86 => self.replace(2)?,
            0x87 => self.replace(3)?,
            // Memo.
            b'g' => {
                let index = input.decimal_line()?;
                self.push_from_memo(index)?;
            }
            b'h' => {
                let index = input.unsigned(1)?;
                self.push_from_memo(index)?;
            }
            b'j' => {
                let index = input.unsigned(4)?;
                self.push_from_memo(index)?;
            }
            b'p' => {
                let index = input.decimal_line()?;
                self.memoize(index)?;
            }
            b'q' => {
                let index = input.unsigned(1)?;
                self.memoize(index)?;
            }
            b'r' => {
                let index = input.unsigned(4)?;
                self.memoize(index)?;
            }
            0x94 => self.memoize(self.memo_len)?,
            // Imports and calls.
            b'c' => {
                let (module, name) = input.name_pair()?;
                self.import(&module, &name);
                self.push(Item::Other)?;
            }
            b'i' => {
                let (module, name) = input.name_pair()?;
                self.import(&module, &name);
                self.pop_to_mark()?;
                self.call()?;
            }
            0x93 => {
                let name = self.pop_item()?;
                let module = self.pop_item()?;
                match (module, name) {
                    (Item::Text(module), Item::Text(name)) => self.import(&module, &name),
                    _ => self.import(UNKNOWN_NAME, UNKNOWN_NAME),
                }
                self.push(Item::Other)?;
            }
            0x82 => self.extension(input, 1)?,
            0x83 => self.extension(input, 2)?,
            0x84 => self.extension(input, 4)?,
            b'R' | 0x81 => {
                self.pop(2)?;
                self.call()?;
            }
            0x92 => {
                self.pop(3)?;
                self.call()?;
            }
            b'o' => {
                self.pop_to_mark()?;
                self.call()?;
            }
            b'b' => {
                self.pop(1)?;
                self.calls += 1;
            }
            // Framing, protocol and persistent ids: nothing to learn.
            0x80 => {
                let protocol = input.unsigned(1)? as u8;
                self.protocol.get_or_insert(protocol);
            }
            0x95 => input.skip(8)?,
            b'P' => {
                input.line()?;
                self.push(Item::Other)?;
            }
            b'Q' | 0x98 => self.replace(1)?,
            _ => return Err(Halt::UnknownOpcode),
        }
        Ok(false)
    }

    /// `EXT1`, `EXT2`, `EXT4`: the copyreg extension registry can map the number to any function.
    fn extension(&mut self, input: &mut Input, bytes: u64) -> Result<(), Halt> {
        let number = input.unsigned(bytes)?;
        self.import("copyreg", &format!("extension {number}"));
        self.push(Item::Other)
    }

    fn push(&mut self, item: Item) -> Result<(), Halt> {
        if self.stack.len() == MAX_STACK {
            return Err(Halt::Limit);
        }
        self.stack.push(item);
        Ok(())
    }

    fn pop_item(&mut self) -> Result<Item, Halt> {
        self.stack.pop().ok_or(Halt::StackUnderflow)
    }

    fn pop(&mut self, count: usize) -> Result<(), Halt> {
        for _ in 0..count {
            self.pop_item()?;
        }
        Ok(())
    }

    fn pop_to_mark(&mut self) -> Result<(), Halt> {
        while !matches!(self.pop_item()?, Item::Mark) {}
        Ok(())
    }

    /// Pops `count` items and pushes the one value built from them.
    fn replace(&mut self, count: usize) -> Result<(), Halt> {
        self.pop(count)?;
        self.push(Item::Other)
    }

    fn call(&mut self) -> Result<(), Halt> {
        self.calls += 1;
        self.push(Item::Other)
    }

    fn skip_and_push(&mut self, input: &mut Input, bytes: u64) -> Result<(), Halt> {
        input.skip(bytes)?;
        self.push(Item::Other)
    }

    fn push_text(&mut self, input: &mut Input, length: u64) -> Result<(), Halt> {
        let item = if length <= MAX_NAME_BYTES {
            let bytes = input.bytes(length)?;
            Item::Text(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            input.skip(length)?;
            Item::Other
        };
        self.push(item)
    }

    fn push_from_memo(&mut self, index: u64) -> Result<(), Halt> {
        let item = self
            .memo_texts
            .get(&index)
            .cloned()
            .map_or(Item::Other, Item::Text);
        self.push(item)
    }

    fn memoize(&mut self, index: u64) -> Result<(), Halt> {
        let top = self.stack.last().ok_or(Halt::StackUnderflow)?;
        match top {
            Item::Text(text) if self.memo_texts.len() < MAX_MEMO_TEXTS => {
                self.memo_texts.insert(index, text.clone());
            }
            _ => {
                self.memo_texts.remove(&index);
            }
        }
        self.memo_len += 1;
        Ok(())
    }

    fn import(&mut self, module: &str, name: &str) {
        let known = self
            .imports
            .iter()
            .any(|import| import.module == module && import.name == name);
        if !known {
            self.imports.push(PickleImport {
                module: module.to_string(),
                name: name.to_string(),
                dangerous: is_dangerous(module, name),
            });
        }
    }
}

/// Unresolved names count as dangerous: a pickle that hides a name has a reason to.
fn is_dangerous(module: &str, name: &str) -> bool {
    let in_module = |listed: &str| {
        module == listed
            || module
                .strip_prefix(listed)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    module == UNKNOWN_NAME
        || name == UNKNOWN_NAME
        || DANGEROUS_MODULES.into_iter().any(in_module)
        || (matches!(module, "builtins" | "__builtin__") && DANGEROUS_BUILTINS.contains(&name))
        || DANGEROUS_FUNCTIONS.contains(&(module, name))
}

/// The file, read forward; long arguments are skipped without being read.
struct Input {
    reader: BufReader<File>,
    file_bytes: u64,
}

impl Input {
    fn new(file: File, file_bytes: u64) -> Self {
        Self {
            reader: BufReader::new(file),
            file_bytes,
        }
    }

    fn byte(&mut self) -> std::io::Result<u8> {
        let mut byte = [0];
        self.reader.read_exact(&mut byte)?;
        Ok(byte[0])
    }

    fn bytes(&mut self, length: u64) -> Result<Vec<u8>, Halt> {
        let mut bytes = vec![0; length as usize];
        self.reader.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// Little-endian unsigned number of `bytes` bytes (1 to 8).
    fn unsigned(&mut self, bytes: u64) -> Result<u64, Halt> {
        let mut raw = [0; 8];
        self.reader.read_exact(&mut raw[..bytes as usize])?;
        Ok(u64::from_le_bytes(raw))
    }

    /// A signed 4-byte length; a negative one cannot be valid.
    fn signed4(&mut self) -> Result<u64, Halt> {
        let value = self.unsigned(4)? as u32 as i32;
        u64::try_from(value).map_err(|_| Halt::Cut)
    }

    fn skip(&mut self, length: u64) -> Result<(), Halt> {
        let position = self.reader.stream_position().map_err(|_| Halt::Cut)?;
        let end = position.checked_add(length).ok_or(Halt::Cut)?;
        if end > self.file_bytes {
            return Err(Halt::Cut);
        }
        let offset = i64::try_from(length).map_err(|_| Halt::Cut)?;
        self.reader.seek_relative(offset)?;
        Ok(())
    }

    /// One line without its newline; `None` when longer than a name can be. The whole
    /// line is consumed either way.
    fn line(&mut self) -> Result<Option<String>, Halt> {
        let mut kept = Vec::new();
        let mut too_long = false;
        loop {
            let buffer = match self.reader.fill_buf() {
                Ok(buffer) => buffer,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(Halt::Cut),
            };
            if buffer.is_empty() {
                return Err(Halt::Cut);
            }
            let (part, found_end) = match buffer.iter().position(|&b| b == b'\n') {
                Some(end) => (&buffer[..end], true),
                None => (buffer, false),
            };
            if kept.len() + part.len() <= MAX_NAME_BYTES as usize {
                kept.extend_from_slice(part);
            } else {
                too_long = true;
            }
            let consumed = part.len() + usize::from(found_end);
            self.reader.consume(consumed);
            if found_end {
                break;
            }
        }
        let line = String::from_utf8_lossy(&kept)
            .trim_end_matches('\r')
            .to_string();
        Ok((!too_long).then_some(line))
    }

    fn decimal_line(&mut self) -> Result<u64, Halt> {
        self.line()?
            .and_then(|line| line.trim().parse().ok())
            .ok_or(Halt::Cut)
    }

    /// `module\nname\n`, as `GLOBAL` and `INST` write them.
    fn name_pair(&mut self) -> Result<(String, String), Halt> {
        let module = self.line()?.unwrap_or_else(|| UNKNOWN_NAME.to_string());
        let name = self.line()?.unwrap_or_else(|| UNKNOWN_NAME.to_string());
        Ok((module, name))
    }
}
