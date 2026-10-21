//! The environment `flamework.config.json` is substituted from (the TS transformer's `util/env.ts`): `.env`, then
//! `.env.local` over it, from the config file's directory, with the process environment over both; the
//! `${NAME}` / `${NAME:-fallback}` / `$$` substitution; and the conversion of strings to what the schema expects.

use std::path::Path;

use crate::ProjectError;
use crate::js;
use crate::json::{JsObject, JsValue};

/// The files read next to `flamework.config.json`, in order; a later file overrides an earlier one.
pub const ENV_FILES: [&str; 2] = [".env", ".env.local"];

/// An environment: variables in the order a JavaScript object holds them (first assignment's place).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    entries: Vec<(String, String)>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// `env[name] = value`. The variable names `parseEnvFile` accepts are never array indices, so insertion order
    /// is JavaScript's order.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let (name, value) = (name.into(), value.into());
        match self.entries.iter_mut().find(|(k, _)| *k == name) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((name, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The process environment, as `process.env` lists it.
    pub fn from_process() -> Self {
        let mut env = Env::new();
        for (k, v) in std::env::vars_os() {
            if let (Ok(k), Ok(v)) = (k.into_string(), v.into_string()) {
                env.set(k, v);
            }
        }
        env
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Env {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut env = Env::new();
        for (k, v) in iter {
            env.set(k, v);
        }
        env
    }
}

/// Parses a dotenv file: one `KEY=value` per line, an optional `export ` prefix, `#` comments and blank lines. A
/// value in double quotes honours `\n`, `\t`, `\r`, `\\` and `\"`; one in single quotes is taken as written; an
/// unquoted value is trimmed and ends at a ` #` comment.
///
/// As in the TS parser (a JavaScript object): a `__proto__` key is dropped, a key that appears twice keeps its first
/// place and its last value, and a line whose value holds a line terminator other than `\n` (a lone `\r`,
/// U+2028, U+2029) does not match and is skipped.
pub fn parse_env_file(text: &str) -> Env {
    let mut result = Env::new();
    for raw_line in split_lines(text) {
        let line = js::trim(raw_line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = match_line(line) else {
            continue;
        };
        if key == "__proto__" {
            continue;
        }
        result.set(key, parse_env_value(rest));
    }
    result
}

/// `text.split(/\r?\n/)`.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// `/^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/`, with JavaScript's `\s` and `.`.
fn match_line(line: &str) -> Option<(&str, &str)> {
    let with_export = line.strip_prefix("export").and_then(|rest| {
        let trimmed = rest.trim_start_matches(js::is_whitespace);
        (trimmed.len() < rest.len()).then_some(trimmed)
    });
    with_export
        .and_then(match_assignment)
        .or_else(|| match_assignment(line))
}

fn match_assignment(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return None;
    }
    let key_len = b
        .iter()
        .position(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
        .unwrap_or(b.len());
    let (key, rest) = s.split_at(key_len);
    let rest = rest
        .trim_start_matches(js::is_whitespace)
        .strip_prefix('=')?;
    let value = rest.trim_start_matches(js::is_whitespace);
    if value.chars().any(js::is_line_terminator) {
        return None;
    }
    Some((key, value))
}

fn parse_env_value(rest: &str) -> String {
    let mut chars = rest.chars();
    let quote = chars.clone().next();
    if let Some(quote @ ('"' | '\'')) = quote {
        chars.next();
        let mut value = String::new();
        while let Some(c) = chars.next() {
            if c == quote {
                return value;
            }
            if quote == '"' && c == '\\' {
                let mut lookahead = chars.clone();
                if let Some(next) = lookahead.next() {
                    chars = lookahead;
                    value.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                    continue;
                }
            }
            value.push(c);
        }
        // No closing quote: the rest of the line is the value, quote included, as dotenv does.
        return rest.to_owned();
    }
    // `rest.search(/\s#/)`.
    let mut comment = None;
    let mut previous: Option<(usize, char)> = None;
    for (i, c) in rest.char_indices() {
        if c == '#'
            && let Some((at, p)) = previous
            && js::is_whitespace(p)
        {
            comment = Some(at);
            break;
        }
        previous = Some((i, c));
    }
    js::trim(match comment {
        Some(at) => &rest[..at],
        None => rest,
    })
    .to_owned()
}

/// The variable names a config text may substitute: every `${NAME` in it, in order, without repeats. It
/// over-approximates (a name in a comment counts too), which only makes the environment read wider.
pub fn referenced_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("${") {
        rest = &rest[i + 2..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// `hashEnvValue`: what `flamework.build` records of a variable `Flamework.env` inlined, the sha1 (hex) of its
/// value, or `unset`.
pub fn hash_env_value(value: Option<&str>) -> String {
    match value {
        None => "unset".to_owned(),
        Some(value) => crate::sha::hex(&crate::sha::sha1(value.as_bytes())),
    }
}

/// [`load_env`], reading the files through `inputs` and asking it for the process value of each variable the
/// files set and each name in `names` (the process environment wins over both files, as in [`load_env`]).
/// `names` gets the files' names appended.
pub fn load_env_with(
    inputs: &mut dyn crate::inputs::Inputs,
    directory: &str,
    names: &mut Vec<String>,
) -> Env {
    let mut env = Env::new();
    for file in ENV_FILES {
        if let Some(text) = inputs.read(&crate::node_path::join(&[directory, file])) {
            for (k, v) in parse_env_file(&text).iter() {
                env.set(k, v);
                if !names.iter().any(|n| n == k) {
                    names.push(k.to_owned());
                }
            }
        }
    }
    for name in names.iter() {
        if let Some(value) = inputs.env(name) {
            env.set(name.clone(), value);
        }
    }
    env
}

/// `.env`, then `.env.local` on top, both from `directory`, with `process_env` on top of both.
pub fn load_env(directory: &Path, process_env: &Env) -> Env {
    let mut env = Env::new();
    for file in ENV_FILES {
        if let Ok(text) = std::fs::read_to_string(directory.join(file)) {
            for (k, v) in parse_env_file(&text).iter() {
                env.set(k, v);
            }
        }
    }
    for (k, v) in process_env.iter() {
        env.set(k, v);
    }
    env
}

/// Replaces `${NAME}` and `${NAME:-fallback}` in every string of a parsed config with the variable's value, and
/// `$$` with a literal dollar. Keys are left alone. A variable that is not set and has no fallback raises, naming
/// it and where it was used (`describe` turns a JSON pointer into the place).
///
/// Returns the value and the JSON pointers of every string that had a placeholder.
pub fn substitute_env(
    value: &JsValue,
    env: &Env,
    describe: &dyn Fn(&str) -> String,
) -> Result<(JsValue, Vec<String>), ProjectError> {
    let mut substituted = Vec::new();
    let value = substitute(value, env, describe, "", &mut substituted)?;
    Ok((value, substituted))
}

fn substitute(
    current: &JsValue,
    env: &Env,
    describe: &dyn Fn(&str) -> String,
    pointer: &str,
    substituted: &mut Vec<String>,
) -> Result<JsValue, ProjectError> {
    Ok(match current {
        JsValue::String(s) => {
            let (replaced, matched) = substitute_string(s, env, &|name| {
                ProjectError(format!(
                    "{} uses ${name}, which is not set in the environment, .env or .env.local and has no fallback. Set it, or write ${{{name}:-value}} to give it one.",
                    describe(pointer)
                ))
            })?;
            if matched && !substituted.iter().any(|p| p == pointer) {
                substituted.push(pointer.to_owned());
            }
            JsValue::String(replaced)
        }
        JsValue::Array(items) => JsValue::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    substitute(item, env, describe, &format!("{pointer}/{i}"), substituted)
                })
                .collect::<Result<_, _>>()?,
        ),
        JsValue::Object(object) => {
            let mut result = JsObject::new();
            for (key, item) in object.iter() {
                result.insert(
                    key.clone(),
                    substitute(
                        item,
                        env,
                        describe,
                        &format!("{pointer}/{key}"),
                        substituted,
                    )?,
                );
            }
            JsValue::Object(result)
        }
        other => other.clone(),
    })
}

/// `s.replace(/\$\$|\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}/g, ...)`: the result, and whether anything matched.
fn substitute_string(
    s: &str,
    env: &Env,
    missing: &dyn Fn(&str) -> ProjectError,
) -> Result<(String, bool), ProjectError> {
    let mut out = String::new();
    let mut matched = false;
    let mut rest = s;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("$$") {
            out.push('$');
            matched = true;
            rest = after;
            continue;
        }
        if let Some((name, fallback, len)) = match_placeholder(tail) {
            matched = true;
            match env.get(name) {
                Some(value) => out.push_str(value),
                None => match fallback {
                    Some(fallback) => out.push_str(fallback),
                    None => return Err(missing(name)),
                },
            }
            rest = &tail[len..];
            continue;
        }
        out.push('$');
        rest = &tail[1..];
    }
    out.push_str(rest);
    Ok((out, matched))
}

/// `\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}` at the start of `s`: the name, the fallback and the length.
fn match_placeholder(s: &str) -> Option<(&str, Option<&str>, usize)> {
    let body = s.strip_prefix("${")?;
    let b = body.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return None;
    }
    let name_len = b
        .iter()
        .position(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
        .unwrap_or(b.len());
    let name = &body[..name_len];
    let after = &body[name_len..];
    if let Some(stripped) = after.strip_prefix('}') {
        let _ = stripped;
        return Some((name, None, 2 + name_len + 1));
    }
    if let Some(fallback_part) = after.strip_prefix(":-") {
        let end = fallback_part.find('}')?;
        return Some((
            name,
            Some(&fallback_part[..end]),
            2 + name_len + 2 + end + 1,
        ));
    }
    None
}

/// Converts strings sitting where the schema expects a boolean, a number or a list of strings: a value from the
/// environment is always a string. Booleans accept `true` / `false` / `1` / `0` in any case; a number is what
/// JavaScript's `Number()` makes of it; a list splits on commas, trims and drops empty entries. A string that does
/// not convert raises.
pub fn coerce_by_schema(
    value: &JsValue,
    schema: &JsValue,
    describe: &dyn Fn(&str) -> String,
) -> Result<JsValue, ProjectError> {
    coerce(value, Some(schema), describe, "")
}

fn coerce(
    current: &JsValue,
    node: Option<&JsValue>,
    describe: &dyn Fn(&str) -> String,
    pointer: &str,
) -> Result<JsValue, ProjectError> {
    let Some(node) = node else {
        return Ok(current.clone());
    };
    Ok(match current {
        JsValue::String(s) => coerce_string(s, node, &|| describe(pointer))?,
        JsValue::Array(items) => JsValue::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    coerce(item, node.get("items"), describe, &format!("{pointer}/{i}"))
                })
                .collect::<Result<_, _>>()?,
        ),
        JsValue::Object(object) => match node.get("properties") {
            Some(properties) => {
                let mut result = JsObject::new();
                for (key, item) in object.iter() {
                    result.insert(
                        key.clone(),
                        coerce(
                            item,
                            properties.get(key),
                            describe,
                            &format!("{pointer}/{key}"),
                        )?,
                    );
                }
                JsValue::Object(result)
            }
            None => current.clone(),
        },
        other => other.clone(),
    })
}

fn coerce_string(
    value: &str,
    node: &JsValue,
    place: &dyn Fn() -> String,
) -> Result<JsValue, ProjectError> {
    match node.get("type").and_then(JsValue::as_str) {
        Some("boolean") => match js::trim(value).to_lowercase().as_str() {
            "true" | "1" => Ok(JsValue::Bool(true)),
            "false" | "0" => Ok(JsValue::Bool(false)),
            _ => Err(ProjectError(format!(
                "{} is \"{value}\", which is not a boolean (true, false, 1 or 0).",
                place()
            ))),
        },
        Some("number" | "integer") => {
            let parsed = js::string_to_number(value);
            if js::trim(value).is_empty() || parsed.is_nan() {
                return Err(ProjectError(format!(
                    "{} is \"{value}\", which is not a number.",
                    place()
                )));
            }
            Ok(JsValue::Number(parsed))
        }
        Some("array")
            if node
                .get("items")
                .and_then(|i| i.get("type"))
                .and_then(JsValue::as_str)
                == Some("string") =>
        {
            Ok(JsValue::Array(
                value
                    .split(',')
                    .map(js::trim)
                    .filter(|e| !e.is_empty())
                    .map(|e| JsValue::String(e.to_owned()))
                    .collect(),
            ))
        }
        _ => Ok(JsValue::String(value.to_owned())),
    }
}
