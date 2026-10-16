//! A small XML tokenizer for the parts of Excel workbooks. It reads well-formed XML as
//! Excel writes it; it does not validate, and it ignores declarations and comments.

/// One piece of an XML document. Names keep their namespace prefix; compare them with
/// [`local_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    Start {
        name: &'a str,
        attributes: &'a str,
        /// `<name/>`: no content and no end tag follow.
        empty: bool,
    },
    End {
        name: &'a str,
    },
    /// Text between tags, still escaped; see [`unescape`].
    Text(&'a str),
}

pub fn tokens(text: &str) -> Tokens<'_> {
    Tokens { rest: text }
}

pub struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            if !self.rest.starts_with('<') {
                let end = self.rest.find('<').unwrap_or(self.rest.len());
                let (text, rest) = self.rest.split_at(end);
                self.rest = rest;
                return Some(Token::Text(text));
            }
            if let Some(skipped) = self.skip_markup() {
                self.rest = skipped;
                continue;
            }
            if let Some(after) = self.rest.strip_prefix("</") {
                let end = after.find('>')?;
                self.rest = &after[end + 1..];
                return Some(Token::End {
                    name: after[..end].trim(),
                });
            }
            return self.start_tag();
        }
    }
}

impl<'a> Tokens<'a> {
    /// The rest after a declaration, comment or DOCTYPE at the start; `None` for a tag.
    /// Unterminated markup ends the document.
    fn skip_markup(&self) -> Option<&'a str> {
        let rest = self.rest;
        let skip_past = |end: &str| rest.find(end).map_or("", |at| &rest[at + end.len()..]);
        if rest.starts_with("<?") {
            Some(skip_past("?>"))
        } else if rest.starts_with("<!--") {
            Some(skip_past("-->"))
        } else if rest.starts_with("<!") && !rest.starts_with("<![CDATA[") {
            Some(skip_past(">"))
        } else {
            None
        }
    }

    fn start_tag(&mut self) -> Option<Token<'a>> {
        if let Some(after) = self.rest.strip_prefix("<![CDATA[") {
            // Excel does not write CDATA; its text is passed on as it is.
            let end = after.find("]]>")?;
            self.rest = &after[end + 3..];
            return Some(Token::Text(&after[..end]));
        }
        let inside = &self.rest[1..];
        let end = tag_end(inside)?;
        self.rest = &inside[end + 1..];
        let tag = &inside[..end];
        let (tag, empty) = match tag.strip_suffix('/') {
            Some(tag) => (tag, true),
            None => (tag, false),
        };
        let (name, attributes) = tag
            .split_once(|c: char| c.is_ascii_whitespace())
            .unwrap_or((tag, ""));
        Some(Token::Start {
            name,
            attributes,
            empty,
        })
    }
}

/// Index of the `>` closing a tag; a `>` inside a quoted attribute value does not count.
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in tag.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// `x:row` and `row` are the same element in a document with a default namespace.
pub fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// The unescaped value of the attribute whose local name is `name`.
pub fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    loop {
        rest = rest.trim_start();
        let equals = rest.find('=')?;
        let key = rest[..equals].trim();
        let after = rest[equals + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value_end = after[1..].find(quote)?;
        let value = &after[1..1 + value_end];
        if local_name(key) == name {
            return Some(unescape(value));
        }
        rest = &after[value_end + 2..];
    }
}

/// Replaces the five named entities and numeric character references.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            result.push_str(&rest[start..]);
            return result;
        };
        match entity(&after[..end]) {
            Some(c) => {
                result.push(c);
                rest = &after[end + 1..];
            }
            None => {
                result.push('&');
                rest = after;
            }
        }
    }
    result.push_str(rest);
    result
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_tags_and_text_and_skips_declarations_and_comments() {
        let found: Vec<Token> =
            tokens(r#"<?xml version="1.0"?><!-- note --><a x="1"><b/>hi</a>"#).collect();

        assert_eq!(
            found,
            [
                Token::Start {
                    name: "a",
                    attributes: r#"x="1""#,
                    empty: false
                },
                Token::Start {
                    name: "b",
                    attributes: "",
                    empty: true
                },
                Token::Text("hi"),
                Token::End { name: "a" },
            ]
        );
    }

    #[test]
    fn a_closing_bracket_inside_an_attribute_value_does_not_end_the_tag() {
        let found: Vec<Token> = tokens(r#"<f ref="a>b"/>"#).collect();

        assert_eq!(
            found,
            [Token::Start {
                name: "f",
                attributes: r#"ref="a>b""#,
                empty: true
            }]
        );
    }

    #[test]
    fn attributes_are_found_by_local_name_and_unescaped() {
        let attributes = r#"name="A &amp; B" r:id='rId2'"#;

        assert_eq!(attribute(attributes, "name").as_deref(), Some("A & B"));
        assert_eq!(attribute(attributes, "id").as_deref(), Some("rId2"));
        assert_eq!(attribute(attributes, "missing"), None);
    }

    #[test]
    fn unescapes_named_and_numeric_references_and_keeps_stray_ampersands() {
        assert_eq!(unescape("a &lt;b&gt; &#246;&#x15F; & c"), "a <b> öş & c");
    }

    #[test]
    fn unterminated_markup_ends_the_document() {
        let found: Vec<Token> = tokens("<a>text<b").collect();

        assert_eq!(
            found,
            [
                Token::Start {
                    name: "a",
                    attributes: "",
                    empty: false
                },
                Token::Text("text"),
            ]
        );
    }
}
