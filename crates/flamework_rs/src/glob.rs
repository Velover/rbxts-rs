//! Glob matching as the TS transformer calls it: `glob.sync(pattern, { root, cwd: root, nocase: true })` with glob
//! 9.3 (minimatch 8, brace-expansion 2), results sorted (Q-F4) with forward slashes.
//!
//! Ported pieces, each to its library's rules:
//! - brace expansion (`{a,b}`, nesting, `{1..10}`, `{a..e}`, padding and steps, a group without a comma or a range
//!   kept literal, `\{` escapes);
//! - minimatch's pattern preprocessing at glob's optimisation level 2 (`a/../b` → `b`, `.` and empty parts dropped,
//!   `**/..` split, `**/**` merged, duplicate patterns merged);
//! - segment matching: `*`, `?`, `[...]` classes (`!` / `^` negation, ranges, POSIX classes), `\` escapes,
//!   extglobs `@(..)`, `?(..)`, `*(..)`, `+(..)` and an approximate `!(..)`, case-insensitive;
//! - the walk: dot files and directories only for a part that starts with `.`; `**` matches the directory it
//!   starts in, never enters dot directories, and treats links (symbolic links and junctions) as glob 9 does: a
//!   `**` that is the pattern's first part never enters one; any other `**` with parts after it applies the rest of
//!   the pattern inside a link it meets (one level: no `**` levels below the link); a last `**` lists a link but
//!   does not enter it, and once it has met one it no longer enters a link that an earlier part led to (glob's
//!   one-shot `followGlobstar` flag, reproduced in glob's walk order per directory); parts other than `**` follow
//!   links freely; a trailing `/` matches directories only;
//! - names as glob spells them on a case-insensitive match: the disk's, except a last literal part whose
//!   directory the walk did not list, which keeps the pattern's spelling (`SRC/MIXED/FILE.TS` →
//!   `src/Mixed/FILE.TS`).
//!
//! Absolute patterns (`/src/*`) start at `root` and return absolute paths, as glob does.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::js;

/// `glob.sync(pattern, { root, cwd: root, nocase: true })`, sorted by UTF-16 code units, with `/` separators.
pub fn glob_sync(pattern: &str, root: &Path) -> Vec<String> {
    if pattern.is_empty() || pattern.starts_with('#') {
        return Vec::new();
    }
    // `new Set(braceExpand(pattern))`: a long sequence can expand to 100,000 patterns.
    let mut seen = std::collections::HashSet::new();
    let expanded: Vec<String> = brace_expand_pattern(pattern)
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect();
    let parts = preprocess(expanded.iter().map(|p| slash_split(p)).collect());
    let mut results: Vec<String> = Vec::new();
    for parts in parts {
        let absolute = parts.len() > 1 && parts[0].is_empty();
        let compiled: Vec<Part> = parts
            .iter()
            .enumerate()
            .skip(usize::from(absolute))
            .map(|(i, p)| Part::parse(p, i))
            .collect();
        let mut walker = Walker {
            results: Vec::new(),
        };
        let prefix = if absolute {
            Some(root.to_string_lossy().replace('\\', "/"))
        } else {
            None
        };
        walker.walk(root, &mut Vec::new(), &compiled, false, false);
        for r in walker.results {
            let r = match &prefix {
                Some(prefix) if r.is_empty() => prefix.clone(),
                Some(prefix) => format!("{prefix}/{r}"),
                None => r,
            };
            results.push(r);
        }
    }
    results.sort_by(|a, b| js::utf16_cmp(a, b));
    results.dedup();
    results
}

// ------------------------------------------------------------------------------------------------ braces

/// minimatch's `braceExpand`: a pattern without a `{...}` is returned as it is.
fn brace_expand_pattern(pattern: &str) -> Vec<String> {
    let has_group = pattern.match_indices('{').any(|(i, _)| {
        let rest = &pattern[i + 1..];
        match rest.find(['{', '}']) {
            Some(j) => rest.as_bytes()[j] == b'}' && !rest[..j].chars().any(js::is_line_terminator),
            None => false,
        }
    });
    if !has_group {
        return vec![pattern.to_owned()];
    }
    brace_expand(pattern)
}

const ESC_SLASH: &str = "\0SLASH\0";
const ESC_OPEN: &str = "\0OPEN\0";
const ESC_CLOSE: &str = "\0CLOSE\0";
const ESC_COMMA: &str = "\0COMMA\0";
const ESC_PERIOD: &str = "\0PERIOD\0";
const EXPANSION_MAX: usize = 100_000;
const EXPANSION_MAX_LENGTH: usize = 4_000_000;

/// brace-expansion 2's `expand` (its top-level entry).
pub fn brace_expand(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let s = match s.strip_prefix("{}") {
        Some(rest) => format!("\\{{\\}}{rest}"),
        None => s.to_owned(),
    };
    let escaped = s
        .replace("\\\\", ESC_SLASH)
        .replace("\\{", ESC_OPEN)
        .replace("\\}", ESC_CLOSE)
        .replace("\\,", ESC_COMMA)
        .replace("\\.", ESC_PERIOD);
    expand(&escaped, true)
        .into_iter()
        .map(|e| {
            e.replace(ESC_SLASH, "\\")
                .replace(ESC_OPEN, "{")
                .replace(ESC_CLOSE, "}")
                .replace(ESC_COMMA, ",")
                .replace(ESC_PERIOD, ".")
        })
        .collect()
}

struct Balanced<'a> {
    pre: &'a str,
    body: &'a str,
    post: &'a str,
}

/// balanced-match for `{` and `}`: the first `{` and the `}` that closes it.
fn balanced(s: &str) -> Option<Balanced<'_>> {
    let find = |from: usize, c: char| s.get(from..).and_then(|t| t.find(c)).map(|i| i + from);
    let first = find(0, '{')?;
    let mut bi = Some(find(first + 1, '}')?);
    let mut ai = Some(first);
    let mut i = Some(first);
    let mut begs: Vec<usize> = Vec::new();
    let mut left = s.len();
    let mut right: Option<usize> = None;
    let mut result: Option<(usize, usize)> = None;
    while let Some(idx) = i {
        if result.is_some() {
            break;
        }
        if Some(idx) == ai {
            begs.push(idx);
            ai = find(idx + 1, '{');
        } else if begs.len() == 1 {
            result = Some((begs.pop().unwrap(), bi.unwrap()));
        } else {
            let beg = begs.pop().unwrap();
            if beg < left {
                left = beg;
                right = bi;
            }
            bi = find(idx + 1, '}');
        }
        i = match (ai, bi) {
            (Some(a), Some(b)) if a < b => Some(a),
            (_, b) => b,
        };
    }
    if result.is_none() && !begs.is_empty() {
        // Unbalanced: the widest pair seen (an unclosed group with none seen is left alone).
        result = Some((left, right?));
    }
    let (start, end) = result?;
    Some(Balanced {
        pre: &s[..start],
        body: &s[start + 1..end],
        post: &s[end + 1..],
    })
}

fn parse_comma_parts(s: &str) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    let Some(m) = balanced(s) else {
        return s.split(',').map(str::to_owned).collect();
    };
    let mut p: Vec<String> = m.pre.split(',').map(str::to_owned).collect();
    p.last_mut().unwrap().push_str(&format!("{{{}}}", m.body));
    let mut post_parts = parse_comma_parts(m.post);
    if !m.post.is_empty() {
        let first = post_parts.remove(0);
        p.last_mut().unwrap().push_str(&first);
        p.extend(post_parts);
    }
    p
}

fn combine(acc: &[String], pre: &str, values: &[String], drop_empties: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut length = 0;
    for a in acc {
        for v in values {
            if out.len() >= EXPANSION_MAX {
                return out;
            }
            let expansion = format!("{a}{pre}{v}");
            if drop_empties && expansion.is_empty() {
                continue;
            }
            if length + expansion.len() > EXPANSION_MAX_LENGTH {
                return out;
            }
            length += expansion.len();
            out.push(expansion);
        }
    }
    out
}

fn is_numeric_sequence(body: &str) -> bool {
    let parts: Vec<&str> = body.split("..").collect();
    let int = |p: &str| {
        let p = p.strip_prefix('-').unwrap_or(p);
        !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
    };
    (parts.len() == 2 || parts.len() == 3) && parts.iter().all(|p| int(p))
}

fn is_alpha_sequence(body: &str) -> bool {
    let parts: Vec<&str> = body.split("..").collect();
    let letter = |p: &str| p.len() == 1 && p.as_bytes()[0].is_ascii_alphabetic();
    let int = |p: &str| {
        let p = p.strip_prefix('-').unwrap_or(p);
        !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
    };
    (parts.len() == 2 && letter(parts[0]) && letter(parts[1]))
        || (parts.len() == 3 && letter(parts[0]) && letter(parts[1]) && int(parts[2]))
}

/// `parseInt(str, 10) == str ? parseInt(str, 10) : str.charCodeAt(0)` on a sequence bound (`-?\d+` or one letter).
/// JavaScript numbers: a bound past 2^53 rounds, as in node, instead of overflowing.
fn numeric(s: &str) -> f64 {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        s.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        s.encode_utf16().next().map_or(f64::NAN, f64::from)
    }
}

fn expand_sequence(body: &str, alpha: bool) -> Vec<String> {
    let n: Vec<&str> = body.split("..").collect();
    let x = numeric(n[0]);
    let y = numeric(n[1]);
    let width = n[0].len().max(n[1].len());
    let mut incr = if n.len() == 3 {
        numeric(n[2]).abs().max(1.0)
    } else {
        1.0
    };
    let reverse = y < x;
    if reverse {
        incr = -incr;
    }
    let pad = n.iter().any(|e| {
        let e = e.strip_prefix('-').unwrap_or(e);
        e.len() >= 2 && e.as_bytes()[0] == b'0' && e.as_bytes()[1].is_ascii_digit()
    });
    let mut out = Vec::new();
    let mut i = x;
    let mut length = 0;
    // A step too small to change a large bound repeats it, as in node, until `max` results.
    while (if reverse { i >= y } else { i <= y }) && out.len() < EXPANSION_MAX {
        let c = if alpha {
            match char::from_u32(i as u32 & 0xffff) {
                Some('\\') | None => String::new(),
                Some(c) => c.to_string(),
            }
        } else {
            let mut c = js::number_to_string(i);
            if pad {
                let need = width.saturating_sub(c.encode_utf16().count());
                if need > 0 {
                    let z = "0".repeat(need);
                    c = if i < 0.0 {
                        format!("-{z}{}", &c[1..])
                    } else {
                        format!("{z}{c}")
                    };
                }
            }
            c
        };
        if length + c.len() > EXPANSION_MAX_LENGTH {
            break;
        }
        length += c.len();
        out.push(c);
        i += incr;
    }
    out
}

fn expand(input: &str, top: bool) -> Vec<String> {
    let mut is_top = top;
    let mut acc = vec![String::new()];
    let mut drop_empties = false;
    let mut first_group = true;
    let mut s = input.to_owned();
    loop {
        let Some(m) = balanced(&s) else {
            return combine(&acc, &s, &[String::new()], drop_empties);
        };
        let (pre, body, post) = (m.pre.to_owned(), m.body.to_owned(), m.post.to_owned());
        if pre.ends_with('$') {
            acc = combine(
                &acc,
                &format!("{pre}{{{body}}}"),
                &[String::new()],
                drop_empties && post.is_empty(),
            );
            first_group = false;
            if post.is_empty() {
                break;
            }
            s = post;
            continue;
        }
        let numeric_seq = is_numeric_sequence(&body);
        let alpha_seq = is_alpha_sequence(&body);
        let sequence = numeric_seq || alpha_seq;
        let options = body.contains(',');
        if !sequence && !options {
            // `{a},b}`: `/,(?!,).*\}/` on the rest.
            let reparse = post.char_indices().any(|(i, c)| {
                c == ','
                    && !post[i + 1..].starts_with(',')
                    && post[i + 1..].find('}').is_some_and(|j| {
                        !post[i + 1..i + 1 + j].chars().any(js::is_line_terminator)
                    })
            });
            if reparse {
                s = format!("{pre}{{{body}{ESC_CLOSE}{post}");
                is_top = true;
                continue;
            }
            return combine(
                &acc,
                &format!("{pre}{{{body}}}{post}"),
                &[String::new()],
                drop_empties,
            );
        }
        if first_group {
            drop_empties = is_top && !sequence;
            first_group = false;
        }
        let values = if sequence {
            expand_sequence(&body, alpha_seq)
        } else {
            let mut n = parse_comma_parts(&body);
            if n.len() == 1 {
                n = expand(&n[0], false)
                    .into_iter()
                    .map(|e| format!("{{{e}}}"))
                    .collect();
                if n.len() == 1 {
                    acc = combine(
                        &acc,
                        &format!("{pre}{}", n[0]),
                        &[String::new()],
                        drop_empties && post.is_empty(),
                    );
                    if post.is_empty() {
                        break;
                    }
                    s = post;
                    continue;
                }
            }
            let mut drops = drop_empties && post.is_empty() && pre.is_empty();
            if drops && acc.iter().any(|a| !a.is_empty()) {
                drops = false;
            }
            let mut values = Vec::new();
            let mut length = 0;
            'outer: for part in &n {
                for v in expand(part, false) {
                    if drops && v.is_empty() {
                        continue;
                    }
                    if values.len() >= EXPANSION_MAX || length + v.len() > EXPANSION_MAX_LENGTH {
                        break 'outer;
                    }
                    length += v.len();
                    values.push(v);
                }
            }
            values
        };
        acc = combine(&acc, &pre, &values, drop_empties && post.is_empty());
        if post.is_empty() {
            break;
        }
        s = post;
    }
    acc
}

// ------------------------------------------------------------------------------------------------ preprocessing

/// minimatch's `slashSplit`: on any run of `/`.
fn slash_split(p: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_slash = false;
    for c in p.chars() {
        if c == '/' {
            if !previous_slash {
                parts.push(std::mem::take(&mut current));
            }
            previous_slash = true;
        } else {
            current.push(c);
            previous_slash = false;
        }
    }
    parts.push(current);
    parts
}

fn is_p(p: Option<&String>) -> bool {
    p.is_some_and(|p| !p.is_empty() && p != "." && p != ".." && p != "**")
}

/// minimatch's `preprocess` at optimisation level 2.
fn preprocess(mut glob_parts: Vec<Vec<String>>) -> Vec<Vec<String>> {
    loop {
        let mut did_something = false;
        let mut index = 0;
        while index < glob_parts.len() {
            let mut parts = std::mem::take(&mut glob_parts[index]);
            // <pre>/**/../<p>/<p>/<rest> -> {<pre>/../<p>/<p>/<rest>,<pre>/**/<p>/<p>/<rest>}
            let mut gs: isize = -1;
            while let Some(found) = parts.iter().skip((gs + 1) as usize).position(|p| p == "**") {
                gs = gs + 1 + found as isize;
                let g = gs as usize;
                let mut gss = g;
                while parts.get(gss + 1).is_some_and(|p| p == "**") {
                    gss += 1;
                }
                if gss > g {
                    parts.drain(g + 1..=gss);
                }
                let next = parts.get(g + 1);
                let p = parts.get(g + 2);
                let p2 = parts.get(g + 3);
                if next.is_none_or(|n| n != "..") {
                    continue;
                }
                let bad =
                    |x: Option<&String>| x.is_none_or(|x| x.is_empty() || x == "." || x == "..");
                if bad(p) || bad(p2) {
                    continue;
                }
                did_something = true;
                parts.remove(g);
                let mut other = parts.clone();
                other[g] = "**".to_owned();
                glob_parts.push(other);
                gs -= 1;
            }
            // <pre>/<e>/<rest> -> <pre>/<rest>
            let mut i = 1;
            while i + 1 < parts.len() {
                let p = &parts[i];
                if i == 1 && p.is_empty() && parts[0].is_empty() {
                    i += 1;
                    continue;
                }
                if p == "." || p.is_empty() {
                    did_something = true;
                    parts.remove(i);
                    continue;
                }
                i += 1;
            }
            if parts.len() == 2 && parts[0] == "." && (parts[1] == "." || parts[1].is_empty()) {
                did_something = true;
                parts.pop();
            }
            // <pre>/<p>/../<rest> -> <pre>/<rest>
            let mut dd: isize = 0;
            loop {
                let start = (dd + 1) as usize;
                let Some(found) = parts.iter().skip(start).position(|p| p == "..") else {
                    break;
                };
                dd = (start + found) as isize;
                let d = dd as usize;
                if is_p(parts.get(d - 1)) {
                    did_something = true;
                    let need_dot = d == 1 && parts.get(d + 1).is_some_and(|p| p == "**");
                    parts.drain(d - 1..=d);
                    if need_dot {
                        parts.insert(d - 1, ".".to_owned());
                    }
                    if parts.is_empty() {
                        parts.push(String::new());
                    }
                    dd -= 2;
                }
            }
            glob_parts[index] = parts;
            index += 1;
        }
        if !did_something {
            break;
        }
    }
    // Second phase: merge patterns that match the same paths.
    let mut i = 0;
    while i + 1 < glob_parts.len() {
        let mut j = i + 1;
        while j < glob_parts.len() {
            if let Some(matched) = parts_match(&glob_parts[i], &glob_parts[j]) {
                glob_parts[i] = matched;
                glob_parts[j] = Vec::new();
            }
            j += 1;
        }
        i += 1;
    }
    glob_parts.into_iter().filter(|g| !g.is_empty()).collect()
}

fn parts_match(a: &[String], b: &[String]) -> Option<Vec<String>> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let (mut ai, mut bi) = (0, 0);
    let mut result = Vec::new();
    let mut which = ' ';
    while ai < a.len() && bi < b.len() {
        if a[ai] == b[bi] {
            result.push(if which == 'b' {
                b[bi].clone()
            } else {
                a[ai].clone()
            });
            ai += 1;
            bi += 1;
        } else if a[ai] == "**" && a.get(ai + 1) == Some(&b[bi]) {
            result.push(a[ai].clone());
            ai += 1;
        } else if b[bi] == "**" && b.get(bi + 1) == Some(&a[ai]) {
            result.push(b[bi].clone());
            bi += 1;
        } else if a[ai] == "*" && !b[bi].is_empty() && !b[bi].starts_with('.') && b[bi] != "**" {
            if which == 'b' {
                return None;
            }
            which = 'a';
            result.push(a[ai].clone());
            ai += 1;
            bi += 1;
        } else if b[bi] == "*" && !a[ai].is_empty() && !a[ai].starts_with('.') && a[ai] != "**" {
            if which == 'a' {
                return None;
            }
            which = 'b';
            result.push(b[bi].clone());
            ai += 1;
            bi += 1;
        } else {
            return None;
        }
    }
    (a.len() == b.len()).then_some(result)
}

// ------------------------------------------------------------------------------------------------ segments

#[derive(Debug, Clone)]
enum Token {
    Char(char),
    /// `*`: any run of characters.
    Star,
    /// `?`: one character.
    One,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Ext {
        kind: char,
        alternatives: Vec<Vec<Token>>,
    },
}

#[derive(Debug, Clone)]
enum ClassItem {
    Char(char),
    Range(char, char),
    Posix(String),
}

#[derive(Debug, Clone)]
enum Part {
    /// `**`: its index among the pattern's parts (glob never follows a link for a first `**`), and whether it may
    /// still follow one.
    Globstar {
        index: usize,
        follow: Cell<bool>,
    },
    /// A part without magic, unescaped.
    Literal(String),
    Magic {
        tokens: Vec<Token>,
        dot: bool,
    },
}

impl Part {
    fn parse(p: &str, index: usize) -> Part {
        if p == "**" {
            return Part::Globstar {
                index,
                follow: Cell::new(true),
            };
        }
        let chars: Vec<char> = p.chars().collect();
        let (tokens, _) = parse_tokens(&chars, 0, None);
        if tokens.iter().all(|t| matches!(t, Token::Char(_))) {
            return Part::Literal(
                tokens
                    .iter()
                    .map(|t| {
                        if let Token::Char(c) = t {
                            *c
                        } else {
                            unreachable!()
                        }
                    })
                    .collect(),
            );
        }
        let dot = matches!(tokens.first(), Some(Token::Char('.')));
        Part::Magic { tokens, dot }
    }
}

/// Parses tokens until the end, or (inside an extglob) until `|` / `)`. Returns the tokens and where it stopped.
fn parse_tokens(chars: &[char], mut i: usize, in_ext: Option<()>) -> (Vec<Token>, usize) {
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if in_ext.is_some() && (c == '|' || c == ')') {
            return (out, i);
        }
        match c {
            '\\' if i + 1 < chars.len() => {
                out.push(Token::Char(chars[i + 1]));
                i += 2;
            }
            '*' | '?' | '+' | '@' | '!' if chars.get(i + 1) == Some(&'(') => {
                if let Some((alternatives, end)) = parse_ext(chars, i + 2) {
                    out.push(Token::Ext {
                        kind: c,
                        alternatives,
                    });
                    i = end + 1;
                } else {
                    out.push(match c {
                        '*' => Token::Star,
                        '?' => Token::One,
                        other => Token::Char(other),
                    });
                    i += 1;
                }
            }
            '*' => {
                if !matches!(out.last(), Some(Token::Star)) {
                    out.push(Token::Star);
                }
                i += 1;
            }
            '?' => {
                out.push(Token::One);
                i += 1;
            }
            '[' => match parse_class(chars, i) {
                Some((token, end)) => {
                    out.push(token);
                    i = end;
                }
                None => {
                    out.push(Token::Char('['));
                    i += 1;
                }
            },
            c => {
                out.push(Token::Char(c));
                i += 1;
            }
        }
    }
    (out, i)
}

fn parse_ext(chars: &[char], mut i: usize) -> Option<(Vec<Vec<Token>>, usize)> {
    let mut alternatives = Vec::new();
    loop {
        let (tokens, end) = parse_tokens(chars, i, Some(()));
        alternatives.push(tokens);
        match chars.get(end) {
            Some('|') => i = end + 1,
            Some(')') => return Some((alternatives, end)),
            _ => return None,
        }
    }
}

/// A `[...]` class starting at `start`: the token and the index after `]`, or `None` when it is not closed.
fn parse_class(chars: &[char], start: usize) -> Option<(Token, usize)> {
    let mut i = start + 1;
    let negated = matches!(chars.get(i), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let mut items = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            return Some((Token::Class { negated, items }, i + 1));
        }
        first = false;
        if c == '[' && chars.get(i + 1) == Some(&':') {
            let rest: String = chars[i + 2..].iter().collect();
            if let Some(end) = rest.find(":]") {
                items.push(ClassItem::Posix(rest[..end].to_owned()));
                i += 2 + rest[..end].chars().count() + 2;
                continue;
            }
        }
        let (c, next) = if c == '\\' && i + 1 < chars.len() {
            (chars[i + 1], i + 2)
        } else {
            (c, i + 1)
        };
        if chars.get(next) == Some(&'-') && chars.get(next + 1).is_some_and(|e| *e != ']') {
            let (end, after) = if chars[next + 1] == '\\' && next + 2 < chars.len() {
                (chars[next + 2], next + 3)
            } else {
                (chars[next + 1], next + 2)
            };
            items.push(ClassItem::Range(c, end));
            i = after;
        } else {
            items.push(ClassItem::Char(c));
            i = next;
        }
    }
    None
}

/// JavaScript's case-insensitive comparison of one character (`/i` without `u`, roughly: by simple case mapping).
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn class_matches(negated: bool, items: &[ClassItem], c: char) -> bool {
    if c == '/' {
        return false;
    }
    let candidates = [c, fold(c), c.to_uppercase().next().unwrap_or(c)];
    let hit = items.iter().any(|item| match item {
        ClassItem::Char(x) => fold(*x) == fold(c),
        ClassItem::Range(a, b) => candidates.iter().any(|k| (*a..=*b).contains(k)),
        ClassItem::Posix(name) => match name.as_str() {
            "alnum" => c.is_alphanumeric(),
            "alpha" => c.is_alphabetic(),
            "ascii" => c.is_ascii(),
            "blank" => c == ' ' || c == '\t',
            "cntrl" => c.is_control(),
            "digit" => c.is_ascii_digit(),
            "graph" => !c.is_whitespace() && !c.is_control(),
            "lower" | "upper" => c.is_alphabetic(), // case-insensitive: either case
            "print" => !c.is_control(),
            "punct" => c.is_ascii_punctuation(),
            "space" => c.is_whitespace(),
            "word" => c.is_alphanumeric() || c == '_',
            "xdigit" => c.is_ascii_hexdigit(),
            _ => false,
        },
    });
    hit != negated
}

/// Whether `tokens` match all of `name[at..]`.
fn match_tokens(tokens: &[Token], name: &[char], at: usize) -> bool {
    let Some((first, rest)) = tokens.split_first() else {
        return at == name.len();
    };
    match first {
        Token::Char(c) => {
            at < name.len() && fold(name[at]) == fold(*c) && match_tokens(rest, name, at + 1)
        }
        Token::One => at < name.len() && match_tokens(rest, name, at + 1),
        Token::Class { negated, items } => {
            at < name.len()
                && class_matches(*negated, items, name[at])
                && match_tokens(rest, name, at + 1)
        }
        Token::Star => (at..=name.len()).any(|k| match_tokens(rest, name, k)),
        Token::Ext { kind, alternatives } => {
            let ends_of = |from: usize| -> Vec<usize> {
                (from..=name.len())
                    .filter(|&end| {
                        alternatives
                            .iter()
                            .any(|alt| match_tokens(alt, &name[..end], from))
                    })
                    .collect()
            };
            match kind {
                '@' => ends_of(at).into_iter().any(|e| match_tokens(rest, name, e)),
                '?' => {
                    match_tokens(rest, name, at)
                        || ends_of(at).into_iter().any(|e| match_tokens(rest, name, e))
                }
                '*' | '+' => {
                    // One or more repetitions (zero more for `*`), each consuming at least one character.
                    let mut frontier = vec![at];
                    let mut seen = vec![false; name.len() + 1];
                    if *kind == '*' && match_tokens(rest, name, at) {
                        return true;
                    }
                    while let Some(from) = frontier.pop() {
                        for end in ends_of(from) {
                            if end == from || seen[end] {
                                continue;
                            }
                            seen[end] = true;
                            if match_tokens(rest, name, end) {
                                return true;
                            }
                            frontier.push(end);
                        }
                    }
                    false
                }
                // `!(a|b)`: any run that is not one of the alternatives (approximate: minimatch also looks at the
                // rest of the pattern).
                _ => (at..=name.len()).any(|end| {
                    !alternatives
                        .iter()
                        .any(|alt| match_tokens(alt, &name[..end], at))
                        && match_tokens(rest, name, end)
                }),
            }
        }
    }
}

impl Part {
    fn matches(&self, name: &str) -> bool {
        match self {
            Part::Globstar { .. } => !name.starts_with('.'),
            Part::Literal(l) => fold_str(l) == fold_str(name),
            Part::Magic { tokens, dot } => {
                if name.starts_with('.') && !dot {
                    return false;
                }
                let chars: Vec<char> = name.chars().collect();
                match_tokens(tokens, &chars, 0)
            }
        }
    }
}

fn fold_str(s: &str) -> String {
    s.to_lowercase()
}

// ------------------------------------------------------------------------------------------------ walk

struct Walker {
    results: Vec<String>,
}

struct Entry {
    name: String,
    /// A directory, or a link (symbolic link or junction) to one.
    is_dir: bool,
    /// A symbolic link or a junction (Node reports both as symbolic links).
    is_symlink: bool,
}

fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let file_type = e.file_type().ok()?;
            let is_symlink = file_type.is_symlink();
            let is_dir = if is_symlink {
                e.path().is_dir()
            } else {
                file_type.is_dir()
            };
            Some(Entry {
                name,
                is_dir,
                is_symlink,
            })
        })
        .collect()
}

/// The entry of `dir` named `name`, matched without case (the disk's spelling).
fn disk_entry(dir: &Path, name: &str) -> Option<Entry> {
    let folded = fold_str(name);
    list(dir).into_iter().find(|e| fold_str(&e.name) == folded)
}

/// A directory to walk after the current one's entries are processed, as glob queues its subwalks. glob walks a
/// directory's entries in the reverse of the order the system lists them (path-scurry puts each new child first),
/// which decides when a last `**` stops following links, so the walk takes them in that order too.
struct Task<'p> {
    name: String,
    parts: &'p [Part],
    is_symlink: bool,
}

impl Walker {
    fn push(&mut self, rel: &[String]) {
        self.results.push(rel.join("/"));
    }

    /// `parts` applied at `dir` (whose path from the cwd is `rel`). `listed` says whether `dir` was listed, so a
    /// last literal part takes the disk's spelling; `is_symlink` whether `dir` is a link, which a `**` enters only
    /// as glob 9 allows.
    fn walk(
        &mut self,
        dir: &Path,
        rel: &mut Vec<String>,
        parts: &[Part],
        listed: bool,
        is_symlink: bool,
    ) {
        let Some((part, rest)) = parts.split_first() else {
            return;
        };
        let trailing_dir = matches!(rest, [Part::Literal(l)] if l.is_empty());
        match part {
            Part::Literal(name) if name.is_empty() => {
                if rest.is_empty() && dir.is_dir() {
                    self.push(rel);
                }
            }
            Part::Literal(name) if name == "." => {
                // `.` names the directory itself and adds nothing to the path glob prints.
                if rest.is_empty() || trailing_dir {
                    self.push(rel);
                } else {
                    self.walk(dir, rel, rest, listed, is_symlink);
                }
            }
            Part::Literal(name) if name == ".." => {
                let parent = dir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| dir.to_path_buf());
                let popped = match rel.last() {
                    Some(last) if last != ".." => rel.pop(),
                    _ => {
                        rel.push("..".into());
                        None
                    }
                };
                if rest.is_empty() {
                    self.push(rel);
                } else {
                    self.walk(&parent, rel, rest, false, false);
                }
                match popped {
                    Some(last) => rel.push(last),
                    None => {
                        rel.pop();
                    }
                }
            }
            Part::Literal(name) => {
                let target: PathBuf = dir.join(name);
                if rest.is_empty() {
                    if std::fs::symlink_metadata(&target).is_ok() {
                        let spelled = if listed {
                            disk_entry(dir, name).map_or_else(|| name.clone(), |e| e.name)
                        } else {
                            name.clone()
                        };
                        rel.push(spelled);
                        self.push(rel);
                        rel.pop();
                    }
                    return;
                }
                let Some(entry) = disk_entry(dir, name) else {
                    return;
                };
                // A link counts as a directory for a trailing `/`, as glob's `canReaddir` has it.
                if !entry.is_dir && !(trailing_dir && entry.is_symlink) {
                    return;
                }
                let next = dir.join(&entry.name);
                rel.push(entry.name);
                if trailing_dir {
                    self.push(rel);
                } else {
                    self.walk(&next, rel, rest, false, entry.is_symlink);
                }
                rel.pop();
            }
            Part::Magic { .. } => {
                let mut tasks = Vec::new();
                for entry in list(dir).into_iter().rev() {
                    if !part.matches(&entry.name) {
                        continue;
                    }
                    if rest.is_empty() || (trailing_dir && (entry.is_dir || entry.is_symlink)) {
                        rel.push(entry.name.clone());
                        self.push(rel);
                        rel.pop();
                    } else if entry.is_dir && !trailing_dir {
                        tasks.push(Task {
                            name: entry.name,
                            parts: rest,
                            is_symlink: entry.is_symlink,
                        });
                    }
                }
                self.run(dir, rel, tasks);
            }
            Part::Globstar { index, follow } => {
                if !dir.is_dir() {
                    return;
                }
                // `**` matches the directory it starts in when it is last (or before a trailing `/`).
                if rest.is_empty() || trailing_dir {
                    self.push(rel);
                }
                // glob's processPatterns: a link reached by an earlier part is entered by a `**` only when that
                // `**` is not the pattern's first part and has not given up following (see below).
                if is_symlink && (*index == 0 || !follow.get()) {
                    return;
                }
                // Zero parts: the rest, tested against this directory's entries.
                if !rest.is_empty() && !trailing_dir {
                    self.walk(dir, rel, rest, true, is_symlink);
                }
                let mut tasks = Vec::new();
                for entry in list(dir).into_iter().rev() {
                    if entry.name.starts_with('.') {
                        continue;
                    }
                    if rest.is_empty() {
                        rel.push(entry.name.clone());
                        self.push(rel);
                        rel.pop();
                    }
                    if entry.is_symlink {
                        // A link (to a file too: glob's `canReaddir` is true for any link).
                        if rest.is_empty() {
                            // A last `**` marks itself as having followed a link and then, seeing the link again as
                            // its target, refuses to enter it: it enters no link, and from now on no link target
                            // either.
                            if *index > 0 {
                                follow.set(false);
                            }
                        } else if *index > 0 && follow.get() {
                            // glob 9 follows a link under a `**` that is not first: the rest of the pattern is
                            // applied inside it, with no `**` levels below.
                            if trailing_dir {
                                rel.push(entry.name.clone());
                                self.push(rel);
                                rel.pop();
                            } else if entry.is_dir {
                                tasks.push(Task {
                                    name: entry.name,
                                    parts: rest,
                                    is_symlink: true,
                                });
                            }
                        }
                    } else if entry.is_dir {
                        tasks.push(Task {
                            name: entry.name,
                            parts,
                            is_symlink: false,
                        });
                    }
                }
                self.run(dir, rel, tasks);
            }
        }
    }

    fn run(&mut self, dir: &Path, rel: &mut Vec<String>, tasks: Vec<Task<'_>>) {
        for task in tasks {
            let next = dir.join(&task.name);
            rel.push(task.name);
            self.walk(&next, rel, task.parts, false, task.is_symlink);
            rel.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces() {
        assert_eq!(brace_expand("a{b,c}d"), ["abd", "acd"]);
        assert_eq!(brace_expand("x{1..3}"), ["x1", "x2", "x3"]);
        assert_eq!(brace_expand("{a}"), ["{a}"]);
        assert_eq!(brace_expand("a{01..3}"), ["a01", "a02", "a03"]);
        assert_eq!(brace_expand("{a,{b,c}}"), ["a", "b", "c"]);
        assert_eq!(brace_expand("a{,b}"), ["a", "ab"]);
        assert_eq!(brace_expand("{c..a}"), ["c", "b", "a"]);
    }

    #[test]
    fn preprocessing() {
        let pp = |p: &str| preprocess(vec![slash_split(p)]);
        let v = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(pp("src/nope/../glob/a.ts"), [v(&["src", "glob", "a.ts"])]);
        assert_eq!(pp("src//glob/./a.ts"), [v(&["src", "glob", "a.ts"])]);
        assert_eq!(pp("src/**/../a.ts"), [v(&["src", "**", "..", "a.ts"])]);
        assert_eq!(
            pp("a/**/../b/c"),
            [v(&["b", "c"]), v(&["a", "**", "b", "c"])]
        );
    }
}
