use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, wanted: &str) -> Option<&Value> {
        let Value::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value)
    }

    pub fn string(&self, wanted: &str) -> Option<&str> {
        match self.get(wanted)? {
            Value::String(value) => Some(value),
            Value::Object(_) => None,
        }
    }

    pub fn entries(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries),
            Value::String(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "VDF parse error at byte {}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for Error {}

pub fn parse(input: &str) -> Result<Value, Error> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        offset: 0,
    };
    let entries = parser.parse_entries(false)?;
    parser.skip_trivia();
    if parser.offset != parser.bytes.len() {
        return Err(parser.error("unexpected trailing input"));
    }
    Ok(Value::Object(entries))
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn parse_entries(&mut self, nested: bool) -> Result<Vec<(String, Value)>, Error> {
        let mut entries = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                None if nested => return Err(self.error("missing closing brace")),
                None => return Ok(entries),
                Some(b'}') if nested => {
                    self.offset += 1;
                    return Ok(entries);
                }
                Some(b'}') => return Err(self.error("unexpected closing brace")),
                _ => {}
            }

            let key = self.token()?;
            self.skip_trivia();
            let value = if self.peek() == Some(b'{') {
                self.offset += 1;
                Value::Object(self.parse_entries(true)?)
            } else {
                Value::String(self.token()?)
            };
            entries.push((key, value));
        }
    }

    fn token(&mut self) -> Result<String, Error> {
        self.skip_trivia();
        if self.peek() == Some(b'"') {
            self.offset += 1;
            let mut output = Vec::new();
            while let Some(byte) = self.peek() {
                self.offset += 1;
                match byte {
                    b'"' => {
                        return String::from_utf8(output)
                            .map_err(|_| self.error("quoted token is not UTF-8"));
                    }
                    b'\\' => {
                        let escaped = self
                            .peek()
                            .ok_or_else(|| self.error("unfinished escape sequence"))?;
                        self.offset += 1;
                        output.push(match escaped {
                            b'n' => b'\n',
                            b't' => b'\t',
                            b'r' => b'\r',
                            other => other,
                        });
                    }
                    other => output.push(other),
                }
            }
            return Err(self.error("unterminated quoted token"));
        }

        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}') {
                break;
            }
            self.offset += 1;
        }
        if start == self.offset {
            return Err(self.error("expected token"));
        }
        std::str::from_utf8(&self.bytes[start..self.offset])
            .map(str::to_owned)
            .map_err(|_| self.error("token is not UTF-8"))
    }

    fn skip_trivia(&mut self) {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.offset += 1;
            }
            if self.peek() == Some(b'/') && self.bytes.get(self.offset + 1) == Some(&b'/') {
                self.offset += 2;
                while self.peek().is_some_and(|byte| byte != b'\n') {
                    self.offset += 1;
                }
                continue;
            }
            break;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error {
            offset: self.offset,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_keyvalues_and_comments() {
        let value = parse(
            r#"
            // comment
            "root"
            {
                "path" "/Games/Steam"
                bare "value"
            }
            "#,
        )
        .unwrap();
        let root = value.get("root").unwrap();
        assert_eq!(root.string("path"), Some("/Games/Steam"));
        assert_eq!(root.string("bare"), Some("value"));
    }

    #[test]
    fn rejects_unclosed_objects() {
        let error = parse(r#""root" { "path" "x""#).unwrap_err();
        assert!(error.message.contains("closing brace"));
    }
}
