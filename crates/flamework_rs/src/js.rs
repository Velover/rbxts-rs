//! JavaScript semantics the TS transformer relies on without saying so: what `trim()` and `\s` remove, how
//! `Number(string)` parses, how a number prints, which property keys an object orders first, and the UTF-16 order
//! `Array#sort` uses.

use std::cmp::Ordering;

/// A JavaScript line terminator: what `.` in a regular expression does not match.
pub fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// JavaScript's `WhiteSpace` and `LineTerminator`: what `String#trim` removes and `\s` matches. Unlike Rust's
/// `char::is_whitespace` it includes U+FEFF and excludes U+0085.
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{b}' | '\u{c}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    ) || is_line_terminator(c)
}

/// `String#trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_whitespace)
}

/// `Number#toString()` (and `String(n)`, and what `JSON.stringify` writes for a finite number).
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n == 0.0 {
        return "0".into();
    }
    dragonbox_ecma::Buffer::new().format_finite(n).to_owned()
}

/// `Number(string)`: JavaScript's StringToNumber. Whitespace around is ignored, an empty string is 0, `Infinity`
/// may be signed, `0x` / `0o` / `0b` literals may not; anything else that is not a decimal literal is NaN.
pub fn string_to_number(s: &str) -> f64 {
    let s = trim(s);
    if s.is_empty() {
        return 0.0;
    }
    let radix = |prefix: [&str; 2], radix: u32| -> Option<f64> {
        let rest = s
            .strip_prefix(prefix[0])
            .or_else(|| s.strip_prefix(prefix[1]))?;
        if rest.is_empty() || !rest.chars().all(|c| c.is_digit(radix)) {
            return Some(f64::NAN);
        }
        // Exact for every value below 2^53; above that, accumulating in f64 rounds per digit (a rare, labelled edge).
        Some(rest.chars().fold(0.0, |acc, c| {
            acc * radix as f64 + c.to_digit(radix).unwrap() as f64
        }))
    };
    if let Some(n) = radix(["0x", "0X"], 16)
        .or_else(|| radix(["0o", "0O"], 8))
        .or_else(|| radix(["0b", "0B"], 2))
    {
        return n;
    }
    let (sign, body) = match s.as_bytes()[0] {
        b'+' => (1.0, &s[1..]),
        b'-' => (-1.0, &s[1..]),
        _ => (1.0, s),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(body) {
        return f64::NAN;
    }
    sign * body.parse::<f64>().unwrap_or(f64::NAN)
}

/// `digits [. digits] [e [+-] digits]` or `. digits [e ...]`, with at least one digit in the mantissa.
fn is_decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - frac_start;
    }
    if digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == b.len()
}

/// Whether a property key is an array index, which a JavaScript object orders before every other key, in
/// ascending numeric order: a canonical decimal integer from 0 to 2^32 - 2.
pub fn is_array_index(key: &str) -> bool {
    let b = key.as_bytes();
    if b.is_empty()
        || b.len() > 10
        || !b.iter().all(u8::is_ascii_digit)
        || (b.len() > 1 && b[0] == b'0')
    {
        return false;
    }
    key.parse::<u64>().is_ok_and(|n| n <= 4_294_967_294)
}

/// The order of `Array#sort` without a comparator on strings: by UTF-16 code units.
pub fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        for (n, s) in [
            (1.0, "1"),
            (-0.0, "0"),
            (0.5, "0.5"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (123456789012345680000.0, "123456789012345680000"),
            (0.000001, "0.000001"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
        ] {
            assert_eq!(number_to_string(n), s);
        }
    }

    #[test]
    fn strings_parse_as_number_parses_them() {
        for (s, n) in [
            (" 7 ", 7.0),
            ("0x10", 16.0),
            ("1e1", 10.0),
            ("5.", 5.0),
            (".5", 0.5),
            ("", 0.0),
            ("-3", -3.0),
        ] {
            assert_eq!(string_to_number(s), n, "{s:?}");
        }
        for s in [
            "1_0", "many", ".", "-0x1", "inf", "infinity", "NaN", "1e", "e1", "0x",
        ] {
            assert!(string_to_number(s).is_nan(), "{s:?}");
        }
        assert_eq!(string_to_number("-Infinity"), f64::NEG_INFINITY);
    }

    #[test]
    fn array_indices() {
        assert!(is_array_index("0") && is_array_index("10") && is_array_index("4294967294"));
        assert!(
            !is_array_index("01")
                && !is_array_index("4294967295")
                && !is_array_index("-1")
                && !is_array_index("")
        );
    }
}
