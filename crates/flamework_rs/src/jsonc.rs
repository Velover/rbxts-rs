//! TypeScript's reader for JSON with comments, as `ts.parseConfigFileTextToJson` and `ts.parseJsonText` read
//! `flamework.config.json`: `//` and `/* */` comments, trailing commas, and the first diagnostic as TypeScript words
//! it (`'}' expected.`, `',' expected.`, `Property assignment expected.`, `String literal with double quotes
//! expected.`, ...).
//!
//! TypeScript parses a JSON file with its expression parser, so a value such as `1 + 2` or `abc` parses and is only
//! refused when it is converted ("Property value can only be ..."). This reader recognises such a value and skips it
//! to the next `,`, `}` or `]` at its depth: diagnostics inside it, and object members that are not plain
//! `name: value` pairs (methods, accessors, spreads, computed names), are approximated.

use crate::json::{JsObject, JsValue};

const PROPERTY_VALUE: &str = "Property value can only be string literal, numeric literal, 'true', 'false', 'null', object literal or array literal.";
const DOUBLE_QUOTES: &str = "String literal with double quotes expected.";

/// A parsed JSON-with-comments file: its root expression (TypeScript's `statements[0].expression`) and the first
/// parse diagnostic, if any. Positions are byte offsets into the text.
#[derive(Debug)]
pub struct JsonFile {
    pub root: Option<Node>,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Node {
    /// Where the node's first token starts (TypeScript's `getStart()`: after whitespace and comments).
    pub start: usize,
    /// Where its last token ends (TypeScript's `end`).
    pub end: usize,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Object(Vec<Member>),
    /// `None` for an omitted element (`[a,,b]`).
    Array(Vec<Option<Node>>),
    String {
        value: String,
        double_quoted: bool,
    },
    Number(f64),
    /// `-` and a numeric literal.
    Negative(f64),
    True,
    False,
    Null,
    /// Any other expression: valid TypeScript, refused when converted.
    Other,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub start: usize,
    pub name: Option<Name>,
    /// `None` for a member that is not `name: value` (a shorthand `{ a }`, a method, a spread...).
    pub value: Option<Node>,
}

#[derive(Debug, Clone)]
pub enum Name {
    String {
        value: String,
        double_quoted: bool,
    },
    /// An identifier, a keyword or a numeric literal as a property name.
    Other(String),
}

impl Name {
    fn text(&self) -> &str {
        match self {
            Name::String { value, .. } | Name::Other(value) => value,
        }
    }
}

/// `ts.parseConfigFileTextToJson(name, text)`: the value, or the first diagnostic's text (the parser's first, else
/// the conversion's first).
pub fn parse_config_text(text: &str) -> Result<JsValue, String> {
    let file = parse_json_text(text);
    if let Some(error) = file.parse_error {
        return Err(error);
    }
    let mut errors = Vec::new();
    let value = match &file.root {
        None => JsValue::Object(JsObject::new()),
        Some(root) => match &root.kind {
            Kind::Object(_) => convert(root, &mut errors),
            other => {
                errors
                    .push("The root value of a 'tsconfig.json' file must be an object.".to_owned());
                match other {
                    Kind::Array(items) => match items
                        .iter()
                        .flatten()
                        .find(|n| matches!(n.kind, Kind::Object(_)))
                    {
                        Some(object) => convert(object, &mut errors),
                        None => JsValue::Object(JsObject::new()),
                    },
                    _ => JsValue::Object(JsObject::new()),
                }
            }
        },
    };
    match errors.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(value),
    }
}

fn convert(node: &Node, errors: &mut Vec<String>) -> JsValue {
    match &node.kind {
        Kind::Object(members) => {
            let mut object = JsObject::new();
            for member in members {
                let Some(value) = &member.value else {
                    errors.push("Property assignment expected.".to_owned());
                    continue;
                };
                let name = member
                    .name
                    .as_ref()
                    .expect("a property assignment has a name");
                if !matches!(
                    name,
                    Name::String {
                        double_quoted: true,
                        ..
                    }
                ) {
                    errors.push(DOUBLE_QUOTES.to_owned());
                }
                let converted = convert(value, errors);
                object.insert(name.text(), converted);
            }
            JsValue::Object(object)
        }
        Kind::Array(items) => JsValue::Array(
            items
                .iter()
                .map(|item| match item {
                    Some(item) => convert(item, errors),
                    None => {
                        errors.push(PROPERTY_VALUE.to_owned());
                        JsValue::Undefined
                    }
                })
                .collect(),
        ),
        Kind::String {
            value,
            double_quoted,
        } => {
            if !double_quoted {
                errors.push(DOUBLE_QUOTES.to_owned());
            }
            JsValue::String(value.clone())
        }
        Kind::Number(n) => JsValue::Number(*n),
        Kind::Negative(n) => JsValue::Number(-n),
        Kind::True => JsValue::Bool(true),
        Kind::False => JsValue::Bool(false),
        Kind::Null => JsValue::Null,
        Kind::Other => {
            errors.push(PROPERTY_VALUE.to_owned());
            JsValue::Undefined
        }
    }
}

/// `ts.parseJsonText(name, text)`, stopping at the first parse diagnostic.
pub fn parse_json_text(text: &str) -> JsonFile {
    let mut p = Parser {
        s: text,
        tok: Tok::Eof,
        start: 0,
        end: 0,
        pos: 0,
        depth: 0,
    };
    let result = p.file();
    match result {
        Ok(root) => JsonFile {
            root,
            parse_error: None,
        },
        Err(error) => JsonFile {
            root: None,
            parse_error: Some(error),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Eof,
    Punct(&'static str),
    Str { value: String, double_quoted: bool },
    Num(f64),
    Ident(String),
}

struct Parser<'a> {
    s: &'a str,
    tok: Tok,
    /// The current token's start and end.
    start: usize,
    end: usize,
    /// Where scanning resumes.
    pos: usize,
    /// How many objects and arrays are open.
    depth: usize,
}

/// How deep a config file may nest objects and arrays. TypeScript itself runs out of stack on a file nested about a
/// thousand levels deep (`Maximum call stack size exceeded`); this reader refuses one deeper than this, with that
/// text, long before its own (recursive) parsing, conversion and validation could run out of stack.
pub const MAX_DEPTH: usize = 256;

const PUNCTUATION: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "<<", ">>", "**", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", "@", "#", ".", "`",
];

fn is_ts_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{b}' | '\u{c}' | '\u{a0}' | '\u{85}' | '\u{1680}' | '\u{2000}'
            ..='\u{200b}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
                | '\n'
                | '\r'
                | '\u{2028}'
                | '\u{2029}'
    )
}

fn is_ident_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    is_ident_start(c) || c.is_alphanumeric() || c == '\u{200c}' || c == '\u{200d}'
}

/// Tokens after which an expression continues: an operator, a member access or a call.
fn continues_expression(tok: &Tok) -> bool {
    match tok {
        Tok::Punct(p) => !matches!(
            *p,
            "," | "}" | "]" | ")" | ";" | ":" | "{" | "@" | "#" | "..." | "~"
        ),
        Tok::Ident(word) => matches!(word.as_str(), "in" | "instanceof" | "as" | "satisfies"),
        _ => false,
    }
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    /// Scans the next token, skipping whitespace and comments.
    fn next(&mut self) -> Result<(), String> {
        loop {
            let rest = self.rest();
            let Some(c) = rest.chars().next() else {
                self.tok = Tok::Eof;
                self.start = self.pos;
                self.end = self.pos;
                return Ok(());
            };
            if is_ts_whitespace(c) {
                self.pos += c.len_utf8();
            } else if rest.starts_with("//") {
                self.pos += rest
                    .find(['\n', '\r', '\u{2028}', '\u{2029}'])
                    .unwrap_or(rest.len());
            } else if let Some(comment) = rest.strip_prefix("/*") {
                match comment.find("*/") {
                    Some(i) => self.pos += i + 4,
                    None => return Err("'*/' expected.".into()),
                }
            } else {
                break;
            }
        }
        self.start = self.pos;
        let rest = self.rest();
        let c = rest.chars().next().unwrap();
        self.tok = if c == '"' || c == '\'' {
            self.string(c)?
        } else if c.is_ascii_digit()
            || (c == '.' && rest[1..].starts_with(|d: char| d.is_ascii_digit()))
        {
            self.number()
        } else if is_ident_start(c) {
            let len = rest
                .find(|ch: char| !is_ident_part(ch))
                .unwrap_or(rest.len());
            self.pos += len;
            Tok::Ident(rest[..len].to_owned())
        } else if let Some(p) = PUNCTUATION.iter().find(|p| rest.starts_with(**p)) {
            self.pos += p.len();
            Tok::Punct(p)
        } else {
            return Err("Invalid character.".into());
        };
        self.end = self.pos;
        Ok(())
    }

    fn string(&mut self, quote: char) -> Result<Tok, String> {
        let mut value = String::new();
        let mut chars = self.s[self.pos + 1..].char_indices();
        let base = self.pos + 1;
        loop {
            let Some((i, c)) = chars.next() else {
                return Err("Unterminated string literal.".into());
            };
            match c {
                c if c == quote => {
                    self.pos = base + i + 1;
                    return Ok(Tok::Str {
                        value,
                        double_quoted: quote == '"',
                    });
                }
                '\n' | '\r' => return Err("Unterminated string literal.".into()),
                '\\' => {
                    let Some((_, e)) = chars.next() else {
                        return Err("Unterminated string literal.".into());
                    };
                    match e {
                        'n' => value.push('\n'),
                        't' => value.push('\t'),
                        'r' => value.push('\r'),
                        'b' => value.push('\u{8}'),
                        'f' => value.push('\u{c}'),
                        'v' => value.push('\u{b}'),
                        '0' => value.push('\0'),
                        // A line continuation.
                        '\n' | '\u{2028}' | '\u{2029}' => {}
                        '\r' => {
                            if chars.clone().next().map(|(_, c)| c) == Some('\n') {
                                chars.next();
                            }
                        }
                        'x' | 'u' => {
                            let braced =
                                e == 'u' && chars.clone().next().map(|(_, c)| c) == Some('{');
                            let digits: String = if braced {
                                chars.next();
                                let mut d = String::new();
                                loop {
                                    match chars.next() {
                                        Some((_, '}')) => break,
                                        Some((_, h)) if h.is_ascii_hexdigit() => d.push(h),
                                        _ => return Err("Hexadecimal digit expected.".into()),
                                    }
                                }
                                d
                            } else {
                                let n = if e == 'x' { 2 } else { 4 };
                                let mut d = String::new();
                                for _ in 0..n {
                                    match chars.clone().next() {
                                        Some((_, h)) if h.is_ascii_hexdigit() => {
                                            chars.next();
                                            d.push(h);
                                        }
                                        _ => return Err("Hexadecimal digit expected.".into()),
                                    }
                                }
                                d
                            };
                            let code = u32::from_str_radix(&digits, 16).unwrap_or(u32::MAX);
                            match char::from_u32(code) {
                                Some(ch) => value.push(ch),
                                None if (0xd800..0xe000).contains(&code) => value.push('\u{fffd}'),
                                None => return Err("An extended Unicode escape value must be between 0x0 and 0x10FFFF inclusive.".into()),
                            }
                        }
                        other => value.push(other),
                    }
                }
                c => value.push(c),
            }
        }
    }

    fn number(&mut self) -> Tok {
        let rest = self.rest();
        let b = rest.as_bytes();
        if b.len() > 1 && b[0] == b'0' && matches!(b[1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
            let radix = match b[1] {
                b'x' | b'X' => 16,
                b'o' | b'O' => 8,
                _ => 2,
            };
            let len = 2 + rest[2..]
                .find(|c: char| !c.is_digit(radix))
                .unwrap_or(rest.len() - 2);
            self.pos += len;
            return Tok::Num(rest[2..len].chars().fold(0.0, |a, c| {
                a * radix as f64 + c.to_digit(radix).unwrap() as f64
            }));
        }
        let mut i = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        self.pos += i;
        let text = &rest[..i];
        let text = text.strip_suffix('.').unwrap_or(text);
        Tok::Num(text.parse().unwrap_or(f64::NAN))
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(&self.tok, Tok::Punct(q) if *q == p)
    }

    /// The token after the current one, without consuming anything.
    fn peek_next(&mut self) -> Result<Tok, String> {
        let saved = (self.tok.clone(), self.start, self.end, self.pos);
        self.next()?;
        let tok = self.tok.clone();
        (self.tok, self.start, self.end, self.pos) = saved;
        Ok(tok)
    }

    fn file(&mut self) -> Result<Option<Node>, String> {
        self.next()?;
        if self.tok == Tok::Eof {
            return Ok(None);
        }
        let root = match self.tok.clone() {
            Tok::Punct("[") => self.array()?,
            Tok::Ident(w) if matches!(w.as_str(), "true" | "false" | "null") => self.primary()?,
            Tok::Punct("-") => {
                let saved = (self.tok.clone(), self.start, self.end, self.pos);
                self.next()?;
                let numeric =
                    matches!(self.tok, Tok::Num(_)) && self.peek_next()? != Tok::Punct(":");
                (self.tok, self.start, self.end, self.pos) = saved;
                if numeric {
                    self.primary()?
                } else {
                    self.object()?
                }
            }
            Tok::Num(_) | Tok::Str { .. } if self.peek_next()? != Tok::Punct(":") => {
                self.primary()?
            }
            _ => self.object()?,
        };
        if self.tok != Tok::Eof {
            return Err("Unexpected token.".into());
        }
        Ok(Some(root))
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!(
                "Maximum call stack size exceeded: the file nests objects and arrays more than {MAX_DEPTH} levels deep."
            ));
        }
        Ok(())
    }

    fn object(&mut self) -> Result<Node, String> {
        self.enter()?;
        let node = self.object_inner();
        self.depth -= 1;
        node
    }

    fn array(&mut self) -> Result<Node, String> {
        self.enter()?;
        let node = self.array_inner();
        self.depth -= 1;
        node
    }

    fn object_inner(&mut self) -> Result<Node, String> {
        if !self.is_punct("{") {
            return Err("'{' expected.".into());
        }
        let start = self.start;
        self.next()?;
        let mut members = Vec::new();
        loop {
            if self.is_member_start() {
                members.push(self.member()?);
                if self.is_punct(",") {
                    self.next()?;
                    continue;
                }
                if self.is_punct("}") || self.tok == Tok::Eof {
                    break;
                }
                return Err("',' expected.".into());
            }
            if self.is_punct("}") || self.tok == Tok::Eof {
                break;
            }
            return Err("Property assignment expected.".into());
        }
        if !self.is_punct("}") {
            return Err("'}' expected.".into());
        }
        let end = self.end;
        self.next()?;
        Ok(Node {
            start,
            end,
            kind: Kind::Object(members),
        })
    }

    fn is_member_start(&self) -> bool {
        match &self.tok {
            Tok::Str { .. } | Tok::Num(_) | Tok::Ident(_) => true,
            Tok::Punct(p) => matches!(*p, "[" | "*" | "..." | "."),
            Tok::Eof => false,
        }
    }

    fn member(&mut self) -> Result<Member, String> {
        let start = self.start;
        let name = match self.tok.clone() {
            Tok::Str {
                value,
                double_quoted,
            } => Name::String {
                value,
                double_quoted,
            },
            Tok::Num(n) => Name::Other(crate::js::number_to_string(n)),
            Tok::Ident(word) => Name::Other(word),
            _ => {
                // A spread, a computed name or a generator method: kept as a member that is not an assignment.
                self.skip_expression()?;
                return Ok(Member {
                    start,
                    name: None,
                    value: None,
                });
            }
        };
        let is_identifier = matches!(&self.tok, Tok::Ident(w) if !is_reserved_word(w));
        self.next()?;
        if self.is_punct("?") || self.is_punct("!") {
            self.next()?;
        }
        if self.is_punct("(") || self.is_punct("<") || (is_identifier && !self.is_punct(":")) {
            // A method, an accessor or a shorthand member.
            if !(self.is_punct(",") || self.is_punct("}")) {
                self.skip_expression()?;
            }
            return Ok(Member {
                start,
                name: Some(name),
                value: None,
            });
        }
        if !self.is_punct(":") {
            return Err("':' expected.".into());
        }
        self.next()?;
        let value = self.value()?;
        Ok(Member {
            start,
            name: Some(name),
            value: Some(value),
        })
    }

    fn array_inner(&mut self) -> Result<Node, String> {
        let start = self.start;
        self.next()?;
        let mut items = Vec::new();
        loop {
            if self.is_punct(",") {
                items.push(None);
                self.next()?;
                continue;
            }
            if self.is_punct("]") || self.tok == Tok::Eof {
                break;
            }
            if !self.starts_expression() {
                return Err("Expression or comma expected.".into());
            }
            items.push(Some(self.value()?));
            if self.is_punct(",") {
                self.next()?;
                continue;
            }
            if self.is_punct("]") || self.tok == Tok::Eof {
                break;
            }
            return Err("',' expected.".into());
        }
        if !self.is_punct("]") {
            return Err("']' expected.".into());
        }
        let end = self.end;
        self.next()?;
        Ok(Node {
            start,
            end,
            kind: Kind::Array(items),
        })
    }

    fn starts_expression(&self) -> bool {
        match &self.tok {
            Tok::Eof => false,
            Tok::Punct(p) => !matches!(
                *p,
                "," | "}" | "]" | ")" | ";" | ":" | "=>" | "=" | "?" | "." | "?."
            ),
            _ => true,
        }
    }

    /// A property value or array element: TypeScript's `parseAssignmentExpressionOrHigher`.
    fn value(&mut self) -> Result<Node, String> {
        if !self.starts_expression() {
            return Err("Expression expected.".into());
        }
        let start = self.start;
        let node = match self.tok.clone() {
            Tok::Punct("{") => Some(self.object()?),
            Tok::Punct("[") => Some(self.array()?),
            Tok::Punct("-") => {
                self.next()?;
                if let Tok::Num(n) = self.tok {
                    let end = self.end;
                    self.next()?;
                    Some(Node {
                        start,
                        end,
                        kind: Kind::Negative(n),
                    })
                } else {
                    None
                }
            }
            Tok::Str { .. } | Tok::Num(_) => Some(self.primary()?),
            Tok::Ident(w) if matches!(w.as_str(), "true" | "false" | "null") => {
                Some(self.primary()?)
            }
            _ => None,
        };
        match node {
            Some(node) if !continues_expression(&self.tok) => Ok(node),
            _ => {
                self.skip_expression()?;
                Ok(Node {
                    start,
                    end: self.start,
                    kind: Kind::Other,
                })
            }
        }
    }

    fn primary(&mut self) -> Result<Node, String> {
        let (start, end) = (self.start, self.end);
        let kind = match self.tok.clone() {
            Tok::Str {
                value,
                double_quoted,
            } => Kind::String {
                value,
                double_quoted,
            },
            Tok::Num(n) => Kind::Number(n),
            Tok::Ident(w) if w == "true" => Kind::True,
            Tok::Ident(w) if w == "false" => Kind::False,
            Tok::Ident(w) if w == "null" => Kind::Null,
            Tok::Punct("-") => {
                self.next()?;
                let Tok::Num(n) = self.tok else {
                    unreachable!("checked by the caller")
                };
                let end = self.end;
                self.next()?;
                return Ok(Node {
                    start,
                    end,
                    kind: Kind::Negative(n),
                });
            }
            _ => unreachable!("not a primary"),
        };
        self.next()?;
        Ok(Node { start, end, kind })
    }

    /// Skips an expression TypeScript would parse but JSON does not take, up to the next `,`, `}`, `]` or `)` at
    /// its depth.
    fn skip_expression(&mut self) -> Result<(), String> {
        let mut depth = 0usize;
        loop {
            match &self.tok {
                Tok::Eof => return Ok(()),
                Tok::Punct(p) if matches!(*p, "{" | "[" | "(") => depth += 1,
                Tok::Punct(p) if matches!(*p, "}" | "]" | ")") => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
                Tok::Punct(",") if depth == 0 => return Ok(()),
                _ => {}
            }
            self.next()?;
        }
    }
}

fn is_reserved_word(word: &str) -> bool {
    matches!(
        word,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_comments_and_trailing_commas() {
        let v = parse_config_text("{\n\t// c\n\t\"a\": { \"b\": [1, -2, ], }, /* d */\n}").unwrap();
        assert_eq!(v.to_json(), r#"{"a":{"b":[1,-2]}}"#);
    }
}
