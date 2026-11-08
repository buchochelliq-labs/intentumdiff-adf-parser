//! Borrow exact JSON values and cache each traversed container once.
use intentumdiff_plugin_sdk::tree::Position;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

// Preserve duplicate keys until the SDK can reject ambiguous mapping identity.
pub(crate) struct RawFields<'a>(pub(crate) Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for RawFields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldsVisitor;
        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = RawFields<'de>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(RawFields(fields))
            }
        }
        deserializer.deserialize_map(FieldsVisitor)
    }
}

enum Container<'a> {
    Map(BTreeMap<String, &'a RawValue>),
    Seq(Vec<&'a RawValue>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_key_names_in_distinct_objects_have_distinct_key_anchors() {
        let source = r#"{"first":{"x":1},"second":{"x":2}}"#;
        let mut spans = JsonSpans::new(source).unwrap();
        let first = spans.key_position(&["first", "x"]).unwrap();
        let second = spans.key_position(&["second", "x"]).unwrap();
        assert_eq!(
            (
                first.start_line,
                first.start_col,
                first.end_line,
                first.end_col
            ),
            (0, 10, 0, 13)
        );
        assert_eq!(
            (
                second.start_line,
                second.start_col,
                second.end_line,
                second.end_col
            ),
            (0, 27, 0, 30)
        );
    }

    #[test]
    fn escaped_unicode_quote_and_backslash_keys_keep_their_exact_source_tokens() {
        let source = r#"{"\u0061":1,"é/name":2,"quote\"key":3,"slash\\key":4,"":5}"#;
        let mut spans = JsonSpans::new(source).unwrap();
        for (name, token) in [
            ("a", r#""\u0061""#),
            ("é/name", r#""é/name""#),
            ("quote\"key", r#""quote\"key""#),
            ("slash\\key", r#""slash\\key""#),
            ("", "\"\""),
        ] {
            let p = spans.key_position(&[name]).unwrap();
            assert_eq!(&source[p.start_col as usize..p.end_col as usize], token);
        }
    }

    #[test]
    fn multiline_crlf_keys_and_values_have_separate_byte_anchors() {
        let source = "{\r\n  \"é\":\r\n    3\r\n}";
        let mut spans = JsonSpans::new(source).unwrap();
        let key = spans.key_position(&["é"]).unwrap();
        let value = spans.position(&["é"]).unwrap();
        assert_eq!(
            (key.start_line, key.start_col, key.end_line, key.end_col),
            (1, 2, 1, 6)
        );
        assert_eq!(
            (
                value.start_line,
                value.start_col,
                value.end_line,
                value.end_col
            ),
            (2, 4, 2, 5)
        );
    }

    #[test]
    fn foreign_values_and_array_elements_cannot_invent_property_keys() {
        let mut spans = JsonSpans::new(r#"{"array":[1]}"#).unwrap();
        let foreign = RawValue::from_string("1".to_owned()).unwrap();
        assert!(spans.key_position_raw("array", &foreign).is_none());
        assert!(spans.key_position(&["array", "0"]).is_none());
    }
}

pub(crate) struct JsonSpans<'a> {
    source: &'a str,
    root: &'a RawValue,
    containers: HashMap<usize, Container<'a>>,
    lines: Vec<usize>,
}
impl<'a> JsonSpans<'a> {
    pub(crate) fn new(source: &'a str) -> Result<Self, String> {
        let root = serde_json::from_str(source).map_err(|e| e.to_string())?;
        let lines = std::iter::once(0)
            .chain(
                source
                    .bytes()
                    .enumerate()
                    .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
            )
            .collect();
        Ok(Self {
            source,
            root,
            containers: HashMap::new(),
            lines,
        })
    }
    fn container(&mut self, raw: &'a RawValue) -> Option<&Container<'a>> {
        let offset = raw.get().as_ptr() as usize - self.source.as_ptr() as usize;
        if let std::collections::hash_map::Entry::Vacant(entry) = self.containers.entry(offset) {
            let container = match raw.get().as_bytes().first()? {
                b'{' => {
                    let fields: RawFields<'a> = serde_json::from_str(raw.get()).ok()?;
                    let count = fields.0.len();
                    let map: BTreeMap<_, _> = fields.0.into_iter().collect();
                    if map.len() != count {
                        return None;
                    }
                    Container::Map(map)
                }
                b'[' => Container::Seq(serde_json::from_str(raw.get()).ok()?),
                _ => return None,
            };
            entry.insert(container);
        }
        self.containers.get(&offset)
    }
    fn at(&mut self, path: &[&str]) -> Option<&'a RawValue> {
        let mut raw = self.root;
        for part in path {
            raw = match self.container(raw)? {
                Container::Map(map) => *map.get(*part)?,
                Container::Seq(seq) => *seq.get(part.parse::<usize>().ok()?)?,
            };
        }
        Some(raw)
    }
    pub(crate) fn object_keys(&mut self, path: &[&str]) -> Option<Vec<String>> {
        let raw = self.at(path)?;
        match self.container(raw)? {
            Container::Map(map) => Some(map.keys().cloned().collect()),
            Container::Seq(_) => None,
        }
    }
    pub(crate) fn raw_at(&mut self, path: &[&str]) -> Option<&'a RawValue> {
        self.at(path)
    }
    pub(crate) fn key_position(&mut self, path: &[&str]) -> Option<Position> {
        let key = *path.last()?;
        let raw = self.at(path)?;
        self.key_position_raw(key, raw)
    }
    pub(crate) fn key_position_raw(&self, key: &str, raw: &RawValue) -> Option<Position> {
        self.position_raw(raw)?;
        let bytes = self.source.as_bytes();
        let mut end = raw.get().as_ptr() as usize - self.source.as_ptr() as usize;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end == 0 || bytes[end - 1] != b':' {
            return None;
        }
        end -= 1;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end == 0 || bytes[end - 1] != b'"' {
            return None;
        }
        let mut start = end - 1;
        loop {
            start = start.checked_sub(1)?;
            if bytes[start] == b'"' {
                let mut escaped = start;
                while escaped > 0 && bytes[escaped - 1] == b'\\' {
                    escaped -= 1;
                }
                if (start - escaped) % 2 == 0 {
                    break;
                }
            }
        }
        if serde_json::from_str::<String>(&self.source[start..end]).ok()? != key {
            return None;
        }
        let (start_line, start_col) = self.point(start);
        let (end_line, end_col) = self.point(end);
        Some(Position {
            start_line,
            start_col,
            end_line,
            end_col,
        })
    }
    fn point(&self, offset: usize) -> (u32, u32) {
        let line = self.lines.partition_point(|&start| start <= offset) - 1;
        (line as u32, (offset - self.lines[line]) as u32)
    }
    pub(crate) fn position(&mut self, path: &[&str]) -> Option<Position> {
        let raw = self.at(path)?;
        self.position_raw(raw)
    }
    pub(crate) fn position_raw(&self, raw: &RawValue) -> Option<Position> {
        let raw = raw.get();
        let start = (raw.as_ptr() as usize).checked_sub(self.source.as_ptr() as usize)?;
        let end = start.checked_add(raw.len())?;
        if end > self.source.len()
            || !self.source.is_char_boundary(start)
            || !self.source.is_char_boundary(end)
        {
            return None;
        }
        let (start_line, start_col) = self.point(start);
        let (end_line, end_col) = self.point(end);
        Some(Position {
            start_line,
            start_col,
            end_line,
            end_col,
        })
    }
}
