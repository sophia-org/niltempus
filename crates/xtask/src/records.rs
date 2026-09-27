//! The one unambiguous line encoding of the dependency and physical-inputs
//! manifests.
//!
//! A manifest is UTF-8 text, one record per line, each line ending in `\n`:
//!
//! ```text
//! kind key=value key=value ...
//! ```
//!
//! Kinds are `[a-z-]+`, keys `[a-z0-9_]+`, separated by exactly one space. A
//! value is written bare when it is non-empty and every byte is in
//! `[A-Za-z0-9._/+:@,~-]`; otherwise it is double-quoted with `\\` and `\"`
//! as the only escapes. Control characters are refused in any value. The
//! encoding is canonical: a line is accepted only if re-encoding its parsed
//! record reproduces it byte for byte, so every record has exactly one
//! spelling and a manifest's sha256 names exactly one content.
use std::fmt::Write as _;

/// One parsed line: its kind and its fields in written order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: String,
    pub fields: Vec<(String, String)>,
}

impl Record {
    /// A record of `kind` with no fields yet (see `with`).
    pub fn of(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            fields: Vec::new(),
        }
    }

    /// Append one field.
    pub fn with(mut self, key: &str, value: impl AsRef<str>) -> Self {
        self.fields
            .push((key.to_owned(), value.as_ref().to_owned()));
        self
    }

    /// The fields, which must be exactly `keys` in that order.
    pub fn expect(&self, keys: &[&str]) -> Result<Vec<&str>, String> {
        let found = self
            .fields
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>();
        if found != keys {
            return Err(format!(
                "{} record has fields {found:?}; expected exactly {keys:?}",
                self.kind
            ));
        }
        Ok(self.fields.iter().map(|(_, v)| v.as_str()).collect())
    }

    pub fn encode(&self) -> Result<String, String> {
        if !valid_kind(&self.kind) {
            return Err(format!("invalid record kind {:?}", self.kind));
        }
        let mut line = self.kind.clone();
        for (key, value) in &self.fields {
            if !valid_key(key) {
                return Err(format!("invalid {} key {key:?}", self.kind));
            }
            write!(line, " {key}={}", encode_value(value)?).map_err(|e| e.to_string())?;
        }
        Ok(line)
    }
}

fn valid_kind(kind: &str) -> bool {
    !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn bare(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"._/+:@,~-".contains(&byte)
}

pub fn encode_value(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!("value has a control character: {value:?}"));
    }
    if !value.is_empty() && value.bytes().all(bare) {
        return Ok(value.to_owned());
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

/// Parse one line; refused unless it is the canonical encoding.
pub fn parse_line(line: &str) -> Result<Record, String> {
    let bytes = line.as_bytes();
    let kind_end = line.find(' ').unwrap_or(line.len());
    let kind = &line[..kind_end];
    if !valid_kind(kind) {
        return Err(format!("malformed record line: {line:?}"));
    }
    let mut fields = Vec::new();
    let mut at = kind_end;
    while at < bytes.len() {
        if bytes[at] != b' ' {
            return Err(format!("malformed record line: {line:?}"));
        }
        at += 1;
        let key_end = line[at..]
            .find('=')
            .map(|i| at + i)
            .ok_or_else(|| format!("field without '=' in {line:?}"))?;
        let key = &line[at..key_end];
        if !valid_key(key) {
            return Err(format!("malformed key {key:?} in {line:?}"));
        }
        at = key_end + 1;
        let mut value = String::new();
        if bytes.get(at) == Some(&b'"') {
            at += 1;
            let mut closed = false;
            let mut chars = line[at..].char_indices();
            while let Some((offset, c)) = chars.next() {
                match c {
                    '"' => {
                        at += offset + 1;
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some((_, e @ ('"' | '\\'))) => value.push(e),
                        _ => return Err(format!("invalid escape in {line:?}")),
                    },
                    c => value.push(c),
                }
            }
            if !closed {
                return Err(format!("unterminated quoted value in {line:?}"));
            }
        } else {
            let end = line[at..].find(' ').map_or(line.len(), |i| at + i);
            value.push_str(&line[at..end]);
            at = end;
        }
        fields.push((key.to_owned(), value));
    }
    let record = Record {
        kind: kind.to_owned(),
        fields,
    };
    if record.encode()? != line {
        return Err(format!("record line is not in canonical form: {line:?}"));
    }
    Ok(record)
}

/// Every line of a manifest: `\n`-terminated, none empty, all canonical.
pub fn parse_text(text: &str) -> Result<Vec<Record>, String> {
    if text.is_empty() || !text.ends_with('\n') {
        return Err("manifest must be non-empty and end with a newline".into());
    }
    text[..text.len() - 1].split('\n').map(parse_line).collect()
}

pub fn render(records: &[Record]) -> Result<String, String> {
    let mut text = String::new();
    for record in records {
        text.push_str(&record.encode()?);
        text.push('\n');
    }
    Ok(text)
}

/// A relative path that names something strictly inside its root: non-empty
/// `/`-separated normal components, no `.`, `..`, empty component, leading
/// `/` or control character.
pub fn relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "path escapes or is not a plain relative path: {path:?}"
        ));
    }
    Ok(())
}
