//! The small part of Python literal syntax that NumPy writes in `.npy` headers: strings,
//! booleans, non-negative integers, tuples, lists and dictionaries. Nothing is evaluated.

use std::fmt;
use std::iter::Peekable;
use std::str::Chars;

/// Deeper nesting than any real header; stops a crafted file from exhausting the stack.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Text(String),
    Bool(bool),
    Int(u64),
    Tuple(Vec<Literal>),
    List(Vec<Literal>),
    Dict(Vec<(Literal, Literal)>),
}

impl Literal {
    pub fn get(&self, key: &str) -> Option<&Literal> {
        let Self::Dict(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(name, _)| *name == Self::Text(key.to_string()))
            .map(|(_, value)| value)
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => write!(f, "'{text}'"),
            Self::Bool(true) => f.write_str("True"),
            Self::Bool(false) => f.write_str("False"),
            Self::Int(number) => write!(f, "{number}"),
            Self::Tuple(items) if items.len() == 1 => write!(f, "({},)", items[0]),
            Self::Tuple(items) => write!(f, "({})", joined(items)),
            Self::List(items) => write!(f, "[{}]", joined(items)),
            Self::Dict(entries) => {
                let pairs: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
        }
    }
}

fn joined(items: &[Literal]) -> String {
    items
        .iter()
        .map(Literal::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses `text` as one literal followed only by white space.
pub fn parse(text: &str) -> Result<Literal, String> {
    let mut parser = Parser {
        chars: text.chars().peekable(),
    };
    let literal = parser.literal(0)?;
    parser.skip_space();
    match parser.chars.next() {
        None => Ok(literal),
        Some(c) => Err(format!("unexpected {c:?} after the header dictionary")),
    }
}

struct Parser<'a> {
    chars: Peekable<Chars<'a>>,
}

impl Parser<'_> {
    fn literal(&mut self, depth: usize) -> Result<Literal, String> {
        if depth > MAX_DEPTH {
            return Err("the header is nested too deeply".to_string());
        }
        self.skip_space();
        match self.chars.peek().copied() {
            Some(quote @ ('\'' | '"')) => self.text(quote),
            Some('(') => self.items(')', depth).map(Literal::Tuple),
            Some('[') => self.items(']', depth).map(Literal::List),
            Some('{') => self.dict(depth),
            Some(c) if c.is_ascii_digit() => self.int(),
            Some(c) if c.is_ascii_alphabetic() => self.word(),
            Some(c) => Err(format!("unexpected {c:?} in the header")),
            None => Err("the header ends too early".to_string()),
        }
    }

    fn text(&mut self, quote: char) -> Result<Literal, String> {
        self.chars.next();
        let mut text = String::new();
        loop {
            match self.chars.next() {
                Some(c) if c == quote => return Ok(Literal::Text(text)),
                Some('\\') => text.extend(self.chars.next()),
                Some(c) => text.push(c),
                None => return Err("a string in the header is not closed".to_string()),
            }
        }
    }

    fn int(&mut self) -> Result<Literal, String> {
        let mut digits = String::new();
        while let Some(&c) = self.chars.peek().filter(|c| c.is_ascii_digit()) {
            digits.push(c);
            self.chars.next();
        }
        // Python 2 wrote long integers with an `L` suffix.
        if self.chars.peek() == Some(&'L') {
            self.chars.next();
        }
        digits
            .parse()
            .map(Literal::Int)
            .map_err(|_| format!("{digits} is too large"))
    }

    fn word(&mut self) -> Result<Literal, String> {
        let mut word = String::new();
        while let Some(&c) = self
            .chars
            .peek()
            .filter(|c| c.is_ascii_alphanumeric() || **c == '_')
        {
            word.push(c);
            self.chars.next();
        }
        match word.as_str() {
            "True" => Ok(Literal::Bool(true)),
            "False" => Ok(Literal::Bool(false)),
            _ => Err(format!("{word:?} is not a value")),
        }
    }

    /// Comma-separated items up to `close`; a trailing comma is allowed.
    fn items(&mut self, close: char, depth: usize) -> Result<Vec<Literal>, String> {
        self.chars.next();
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if self.chars.peek() == Some(&close) {
                self.chars.next();
                return Ok(items);
            }
            items.push(self.literal(depth + 1)?);
            self.end_of_item(close)?;
        }
    }

    fn dict(&mut self, depth: usize) -> Result<Literal, String> {
        self.chars.next();
        let mut entries = Vec::new();
        loop {
            self.skip_space();
            if self.chars.peek() == Some(&'}') {
                self.chars.next();
                return Ok(Literal::Dict(entries));
            }
            let key = self.literal(depth + 1)?;
            self.skip_space();
            if self.chars.next() != Some(':') {
                return Err("a dictionary key has no value".to_string());
            }
            entries.push((key, self.literal(depth + 1)?));
            self.end_of_item('}')?;
        }
    }

    /// After an item: a comma, or the closing bracket left for the caller.
    fn end_of_item(&mut self, close: char) -> Result<(), String> {
        self.skip_space();
        match self.chars.peek().copied() {
            Some(',') => {
                self.chars.next();
                Ok(())
            }
            Some(c) if c == close => Ok(()),
            Some(c) => Err(format!("unexpected {c:?} in the header")),
            None => Err("the header ends too early".to_string()),
        }
    }

    fn skip_space(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }
}
