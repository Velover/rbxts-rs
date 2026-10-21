//! Plain text to Luau string-literal text.
//!
//! The renderer takes a [`rbxts_luau::StringLiteral`] value as already-escaped text and only picks the delimiter:
//! `"…"` when the value has no `"` and no newline, else `'…'` when it has no `'`, else a long bracket `[[…]]`. A long
//! bracket does not interpret escapes, so the escaped text must never send the renderer there: newlines are
//! escaped, and when the text has both quote kinds every `"` becomes `\x22`.

use crate::LuauExpr;

/// Escapes plain text so that the renderer, whatever delimiter it picks, produces a Luau literal whose value is
/// exactly `text`:
/// - `\` → `\\`; `\n`, `\r`, `\t` by name; other control characters (and DEL) as `\xHH`, which never runs into a
///   following digit as a decimal `\ddd` would;
/// - `"` → `\x22` when `text` also contains `'` (otherwise the renderer picks the other quote);
/// - everything else, non-ASCII included, as it is.
///
/// Text that needs no escape (identifiers, most ids) comes back unchanged, so the renderer's `obj.key` shortcut for
/// keys that are valid identifiers still applies.
pub fn escape_luau_string(text: &str) -> String {
    let escape_double = text.contains('"') && text.contains('\'');
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' if escape_double => out.push_str("\\x22"),
            c if c.is_ascii_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A Luau string literal with the value `text` (plain text, escaped with [`escape_luau_string`]).
pub fn text_literal(text: &str) -> LuauExpr {
    rbxts_luau::create::string(escape_luau_string(text))
}

#[cfg(test)]
mod tests {
    use rbxts_luau::{LateStrings, create, render_expr};

    use super::*;

    fn rendered(text: &str) -> String {
        render_expr(&text_literal(text), &LateStrings::default())
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(escape_luau_string("Flamework_abc123"), "Flamework_abc123");
        assert_eq!(escape_luau_string("héllo ☃ 😀"), "héllo ☃ 😀");
        assert_eq!(rendered("plain"), "\"plain\"");
    }

    #[test]
    fn quotes_pick_a_delimiter_or_escape() {
        assert_eq!(rendered("say \"hi\""), "'say \"hi\"'");
        assert_eq!(rendered("it's"), "\"it's\"");
        assert_eq!(rendered("it's \"x\""), "\"it's \\x22x\\x22\"");
    }

    #[test]
    fn backslashes_newlines_and_controls() {
        assert_eq!(rendered("a\\b"), "\"a\\\\b\"");
        assert_eq!(rendered("a\nb\r\tc"), "\"a\\nb\\r\\tc\"");
        // \x00 then a digit: a decimal escape \0 would have read "\01"
        assert_eq!(rendered("\u{0}1\u{7f}"), "\"\\x001\\x7f\"");
        // never a long bracket, which would not interpret escapes
        assert!(!rendered("both ' and \" and \n and \\").contains("[["));
    }

    #[test]
    fn late_strings_render_escaped_values_as_keys_and_values() {
        let mut late = LateStrings::new();
        late.insert("id", escape_luau_string("ok_id"));
        late.insert("odd", escape_luau_string("x\"'y"));
        let tree = vec![create::local(
            create::any_id("t"),
            Some(create::map([
                (create::late_string("id"), create::late_string("odd")),
                (create::late_string("odd"), create::number(1.0)),
            ])),
        )];
        let out = rbxts_luau::render(&tree, &late);
        assert!(out.contains("ok_id = \"x\\x22'y\""), "{out}");
        assert!(out.contains("[\"x\\x22'y\"] = 1"), "{out}");
    }
}
