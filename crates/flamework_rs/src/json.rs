//! JSON as JavaScript holds it: objects keep JavaScript's property order (array-index keys first, ascending, then
//! the rest in insertion order), [`parse`] is `JSON.parse` with V8's error messages, and [`stringify`] is
//! `JSON.stringify`, so that a file the TS transformer wrote reads and writes back to the same bytes.
//!
//! Parsing and writing are iterative: nesting costs heap, not stack, as in V8 (whose `JSON.parse` has no depth
//! limit either).
//!
//! One known difference: a lone UTF-16 surrogate (`"\ud800"`, which JavaScript strings can hold) cannot be held in a
//! Rust `String`. It is read as U+FFFD and written back as that character, where the TS transformer writes
//! `"\ud800"`. Only a hand-edited `flamework.build` can have one: every string the transformer puts there comes from
//! source text, and tsrs, whose strings are Rust strings too, cannot produce one either.

use crate::js;

#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    /// A property set to `undefined`: kept in its place, but `JSON.stringify` leaves it out (and writes `null` for it
    /// in an array). The TS build info has one: `identifierPrefix` set to `undefined` in a game.
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    Object(JsObject),
}

impl JsValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&JsObject> {
        match self {
            JsValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut JsObject> {
        match self {
            JsValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<JsValue>> {
        match self {
            JsValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// `JSON.stringify(value)`.
    pub fn to_json(&self) -> String {
        stringify(self, None).unwrap_or_default()
    }

    /// A property of an object, `None` for anything else.
    pub fn get(&self, key: &str) -> Option<&JsValue> {
        self.as_object()?.get(key)
    }

    /// The `typeof value === "object" && !Array.isArray(value) && value !== null` test.
    pub fn is_plain_object(&self) -> bool {
        matches!(self, JsValue::Object(_))
    }
}

/// An object's own properties in JavaScript's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsObject {
    entries: Vec<(String, JsValue)>,
}

impl JsObject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|(k, _)| k == key)
    }

    pub fn get(&self, key: &str) -> Option<&JsValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsValue> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    /// `object[key] = value`: an existing key keeps its place; a new array-index key goes among the index keys in
    /// numeric order; any other new key goes last.
    pub fn insert(&mut self, key: impl Into<String>, value: JsValue) {
        let key = key.into();
        if let Some(i) = self.position(&key) {
            self.entries[i].1 = value;
            return;
        }
        if js::is_array_index(&key) {
            let n: u64 = key.parse().unwrap();
            let at = self
                .entries
                .iter()
                .position(|(k, _)| !js::is_array_index(k) || k.parse::<u64>().unwrap() > n)
                .unwrap_or(self.entries.len());
            self.entries.insert(at, (key, value));
        } else {
            self.entries.push((key, value));
        }
    }

    /// `object[key] ??= value`, returning the property.
    pub fn entry_or_insert(&mut self, key: &str, value: impl FnOnce() -> JsValue) -> &mut JsValue {
        if !self.contains_key(key) {
            self.insert(key, value());
        }
        self.get_mut(key).unwrap()
    }

    /// `delete object[key]`.
    pub fn remove(&mut self, key: &str) -> Option<JsValue> {
        let i = self.position(key)?;
        Some(self.entries.remove(i).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &JsValue)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl FromIterator<(String, JsValue)> for JsObject {
    fn from_iter<T: IntoIterator<Item = (String, JsValue)>>(iter: T) -> Self {
        let mut object = JsObject::new();
        for (k, v) in iter {
            object.insert(k, v);
        }
        object
    }
}

// ------------------------------------------------------------------------------------------------ stringify

/// `JSON.stringify(value, undefined, indent)`. `None` for a top-level `undefined`.
pub fn stringify(value: &JsValue, indent: Option<&str>) -> Option<String> {
    if matches!(value, JsValue::Undefined) {
        return None;
    }
    let mut out = String::new();
    write_value(&mut out, value, indent.filter(|s| !s.is_empty()), 0);
    Some(out)
}

fn newline(out: &mut String, indent: &str, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str(indent);
    }
}

/// Writes a value without recursion: the containers being written are on a heap stack.
fn write_value(out: &mut String, value: &JsValue, indent: Option<&str>, depth: usize) {
    enum Item<'v> {
        Array(std::slice::Iter<'v, JsValue>, bool),
        Object(
            Box<dyn Iterator<Item = (&'v String, &'v JsValue)> + 'v>,
            bool,
        ),
    }
    let mut stack: Vec<Item> = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next.take() {
            match value {
                JsValue::Undefined | JsValue::Null => out.push_str("null"),
                JsValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                JsValue::Number(n) => {
                    if n.is_finite() {
                        out.push_str(&js::number_to_string(*n));
                    } else {
                        out.push_str("null");
                    }
                }
                JsValue::String(s) => write_string(out, s),
                JsValue::Array(items) if items.is_empty() => out.push_str("[]"),
                JsValue::Array(items) => {
                    out.push('[');
                    stack.push(Item::Array(items.iter(), true));
                }
                JsValue::Object(object) => {
                    out.push('{');
                    let defined = object
                        .iter()
                        .filter(|(_, v)| !matches!(v, JsValue::Undefined));
                    stack.push(Item::Object(Box::new(defined), true));
                }
            }
        }
        let level = depth + stack.len();
        let Some(top) = stack.last_mut() else { return };
        match top {
            Item::Array(items, first) => match items.next() {
                Some(item) => {
                    if !*first {
                        out.push(',');
                    }
                    *first = false;
                    if let Some(indent) = indent {
                        newline(out, indent, level);
                    }
                    next = Some(item);
                }
                None => {
                    stack.pop();
                    if let Some(indent) = indent {
                        newline(out, indent, level - 1);
                    }
                    out.push(']');
                }
            },
            Item::Object(entries, first) => match entries.next() {
                Some((key, item)) => {
                    if !*first {
                        out.push(',');
                    }
                    *first = false;
                    if let Some(indent) = indent {
                        newline(out, indent, level);
                    }
                    write_string(out, key);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    next = Some(item);
                }
                None => {
                    let empty = *first;
                    stack.pop();
                    if !empty && let Some(indent) = indent {
                        newline(out, indent, level - 1);
                    }
                    out.push('}');
                }
            },
        }
    }
}

/// `JSON.stringify` of a string: `"` and `\` escaped, the control characters as `\b \t \n \f \r` or `\u00XX`,
/// everything else (U+2028 and U+2029 included) as it is.
pub fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ------------------------------------------------------------------------------------------------ parse

/// `JSON.parse(text)`, failing with V8's message (Node 20+): `Unexpected end of JSON input`, `Expected ',' or '}'
/// after property value in JSON at position 15 (line 1 column 16)`, and so on.
pub fn parse(text: &str) -> Result<JsValue, String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut parser = Parser { s: &units, i: 0 };
    let value = parser.value()?;
    parser.skip_ws();
    if parser.i < units.len() {
        return Err(format!(
            "Unexpected non-whitespace character after JSON {}",
            parser.location(parser.i)
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [u16],
    i: usize,
}

const MAX_CONTEXT: usize = 10;

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(0x20 | 0x09 | 0x0a | 0x0d)) {
            self.i += 1;
        }
    }

    /// `<message> in JSON at position N (line L column C)`.
    fn at(&self, message: &str, pos: usize) -> String {
        format!("{message} in JSON {}", self.location(pos))
    }

    /// `at position N (line L column C)`, in UTF-16 units, the line and column one-based.
    fn location(&self, pos: usize) -> String {
        let (mut line, mut line_start) = (1, 0);
        let mut k = 0;
        while k < pos.min(self.s.len()) {
            let c = self.s[k];
            if c == 0x0a
                || c == 0x2028
                || c == 0x2029
                || (c == 0x0d && self.s.get(k + 1) != Some(&0x0a))
            {
                line += 1;
                line_start = k + 1;
            }
            k += 1;
        }
        format!(
            "at position {pos} (line {line} column {})",
            pos - line_start + 1
        )
    }

    /// V8's message for an unexpected character at `pos` (or the end of the input).
    fn unexpected(&self, pos: usize) -> String {
        let Some(&c) = self.s.get(pos) else {
            return "Unexpected end of JSON input".into();
        };
        let source = String::from_utf16_lossy(self.s);
        if matches!(
            source.as_str(),
            "undefined" | "NaN" | "Infinity" | "[object Object]"
        ) {
            return format!("\"{source}\" is not valid JSON");
        }
        let token = String::from_utf16_lossy(&[c]);
        let len = self.s.len();
        if len < MAX_CONTEXT * 2 + 1 {
            return format!("Unexpected token '{token}', \"{source}\" is not valid JSON");
        }
        let sub = |a: usize, b: usize| String::from_utf16_lossy(&self.s[a..b.min(len)]);
        if pos < MAX_CONTEXT {
            format!(
                "Unexpected token '{token}', \"{}\"... is not valid JSON",
                sub(0, pos + MAX_CONTEXT)
            )
        } else if pos < len - MAX_CONTEXT {
            format!(
                "Unexpected token '{token}', ...\"{}\"... is not valid JSON",
                sub(pos - MAX_CONTEXT, pos + MAX_CONTEXT)
            )
        } else {
            format!(
                "Unexpected token '{token}', ...\"{}\" is not valid JSON",
                sub(pos - MAX_CONTEXT, len)
            )
        }
    }

    /// One value, nested arrays and objects included, without recursion: the open containers are on a heap stack.
    fn value(&mut self) -> Result<JsValue, String> {
        let mut stack: Vec<Open> = Vec::new();
        'value: loop {
            self.skip_ws();
            let mut value = match self.peek() {
                Some(0x7b) => {
                    self.i += 1;
                    self.skip_ws();
                    match self.peek() {
                        Some(0x7d) => {
                            self.i += 1;
                            JsValue::Object(JsObject::new())
                        }
                        Some(0x22) => {
                            let key = self.property_name()?;
                            stack.push(Open::Object(JsObject::new(), key));
                            continue 'value;
                        }
                        _ => return Err(self.at("Expected property name or '}'", self.i)),
                    }
                }
                Some(0x5b) => {
                    self.i += 1;
                    self.skip_ws();
                    if self.peek() == Some(0x5d) {
                        self.i += 1;
                        JsValue::Array(Vec::new())
                    } else {
                        stack.push(Open::Array(Vec::new()));
                        continue 'value;
                    }
                }
                Some(0x22) => JsValue::String(self.string()?),
                Some(0x74) => self.literal("true", JsValue::Bool(true))?,
                Some(0x66) => self.literal("false", JsValue::Bool(false))?,
                Some(0x6e) => self.literal("null", JsValue::Null)?,
                Some(c) if c == 0x2d || (0x30..=0x39).contains(&c) => self.number()?,
                _ => return Err(self.unexpected(self.i)),
            };
            // Hand the finished value to the containers it closes.
            loop {
                let Some(open) = stack.last_mut() else {
                    return Ok(value);
                };
                self.skip_ws();
                match open {
                    Open::Array(items) => {
                        items.push(value);
                        match self.peek() {
                            Some(0x2c) => {
                                self.i += 1;
                                continue 'value;
                            }
                            Some(0x5d) => {
                                self.i += 1;
                                let Some(Open::Array(items)) = stack.pop() else {
                                    unreachable!()
                                };
                                value = JsValue::Array(items);
                            }
                            _ => {
                                return Err(
                                    self.at("Expected ',' or ']' after array element", self.i)
                                );
                            }
                        }
                    }
                    Open::Object(object, key) => {
                        object.insert(std::mem::take(key), value);
                        match self.peek() {
                            Some(0x2c) => {
                                self.i += 1;
                                self.skip_ws();
                                if self.peek() != Some(0x22) {
                                    return Err(
                                        self.at("Expected double-quoted property name", self.i)
                                    );
                                }
                                *key = self.property_name()?;
                                continue 'value;
                            }
                            Some(0x7d) => {
                                self.i += 1;
                                let Some(Open::Object(object, _)) = stack.pop() else {
                                    unreachable!()
                                };
                                value = JsValue::Object(object);
                            }
                            _ => {
                                return Err(
                                    self.at("Expected ',' or '}' after property value", self.i)
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// A property name and its `:`.
    fn property_name(&mut self) -> Result<String, String> {
        let key = self.string()?;
        self.skip_ws();
        if self.peek() != Some(0x3a) {
            return Err(self.at("Expected ':' after property name", self.i));
        }
        self.i += 1;
        Ok(key)
    }

    fn literal(&mut self, word: &str, value: JsValue) -> Result<JsValue, String> {
        for expected in word.encode_utf16() {
            if self.peek() != Some(expected) {
                return Err(self.unexpected(self.i));
            }
            self.i += 1;
        }
        Ok(value)
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(0x30..=0x39)) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<JsValue, String> {
        let start = self.i;
        if self.peek() == Some(0x2d) {
            self.i += 1;
            if !matches!(self.peek(), Some(0x30..=0x39)) {
                return Err(self.at("No number after minus sign", self.i));
            }
        }
        if self.peek() == Some(0x30) {
            self.i += 1;
            if matches!(self.peek(), Some(0x30..=0x39)) {
                return Err(self.at("Unexpected number", self.i));
            }
        } else {
            self.digits();
        }
        if self.peek() == Some(0x2e) {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.at("Unterminated fractional number", self.i));
            }
        }
        if matches!(self.peek(), Some(0x65 | 0x45)) {
            self.i += 1;
            if matches!(self.peek(), Some(0x2b | 0x2d)) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.at("Exponent part is missing a number", self.i));
            }
        }
        let text = String::from_utf16_lossy(&self.s[start..self.i]);
        Ok(JsValue::Number(text.parse().unwrap_or(f64::NAN)))
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // the opening quote
        let mut out: Vec<u16> = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.at("Unterminated string", self.i));
            };
            match c {
                0x22 => {
                    self.i += 1;
                    return Ok(String::from_utf16_lossy(&out));
                }
                0x5c => {
                    self.i += 1;
                    let Some(e) = self.peek() else {
                        return Err(self.at("Unterminated string", self.i));
                    };
                    let unit = match e {
                        0x22 => 0x22,
                        0x5c => 0x5c,
                        0x2f => 0x2f,
                        0x62 => 0x08,
                        0x66 => 0x0c,
                        0x6e => 0x0a,
                        0x72 => 0x0d,
                        0x74 => 0x09,
                        0x75 => {
                            let mut v: u16 = 0;
                            for _ in 0..4 {
                                self.i += 1;
                                let d = self
                                    .peek()
                                    .and_then(|d| char::from_u32(d as u32))
                                    .and_then(|d| d.to_digit(16));
                                let Some(d) = d else {
                                    return Err(self.at("Bad Unicode escape", self.i));
                                };
                                v = v * 16 + d as u16;
                            }
                            v
                        }
                        _ => return Err(self.at("Bad escaped character", self.i)),
                    };
                    out.push(unit);
                    self.i += 1;
                }
                c if c < 0x20 => {
                    return Err(self.at("Bad control character in string literal", self.i));
                }
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

/// A container being parsed: an array's items so far, or an object's properties and the key of the value to come.
enum Open {
    Array(Vec<JsValue>),
    Object(JsObject, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_order_is_javascripts() {
        let v =
            parse(r#"{"b":1,"10":2,"2":3,"01":4,"4294967295":5,"4294967294":6,"b":7}"#).unwrap();
        assert_eq!(
            v.to_json(),
            r#"{"2":3,"10":2,"4294967294":6,"b":7,"01":4,"4294967295":5}"#
        );
    }

    #[test]
    fn stringify_with_tabs() {
        let v = parse(r#"{"a":[1,{"b":[]}],"c":{},"d":"\u0001\u2028"}"#).unwrap();
        assert_eq!(
            stringify(&v, Some("\t")).unwrap(),
            "{\n\t\"a\": [\n\t\t1,\n\t\t{\n\t\t\t\"b\": []\n\t\t}\n\t],\n\t\"c\": {},\n\t\"d\": \"\\u0001\u{2028}\"\n}"
        );
    }
}
