//! uuid v5 as npm `uuid` 8.3.2 makes it (`v5(name, namespace)`): SHA-1 of the namespace's 16 bytes and the name's
//! UTF-8, version and variant bits set, lowercase. Plus the v4-shaped random uuid the build seed is.
//!
//! The TS transformer's other v4 use, a random uuid per obfuscated string (`BuildInfo.hashString`), is replaced by a
//! v5 derived from the build seed (DESIGN §10.4): see [`crate::build_info::BuildInfo::hash_string`].

use crate::sha;

/// The namespace of the callsite uuids (`CALLSITE_UUID_NAMESPACE` in `transformUserMacro.ts`).
pub const CALLSITE_NAMESPACE: &str = "6f4c1d2e-8b3a-4e5f-9c7d-2a1b0e9f8d7c";

/// `uuid.validate`: the RFC layout with version 1-5 and variant 8-b, or the nil uuid, in any case.
pub fn validate(uuid: &str) -> bool {
    let b = uuid.as_bytes();
    if b.len() != 36 {
        return false;
    }
    if uuid == "00000000-0000-0000-0000-000000000000" {
        return true;
    }
    for (i, c) in b.iter().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => *c == b'-',
            14 => (b'1'..=b'5').contains(c),
            19 => matches!(c.to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_hexdigit(),
        };
        if !ok {
            return false;
        }
    }
    true
}

/// `uuid.parse`: the 16 bytes, or npm's `TypeError` message.
pub fn parse(uuid: &str) -> Result<[u8; 16], String> {
    if !validate(uuid) {
        return Err("Invalid UUID".into());
    }
    let digits: Vec<u8> = uuid
        .bytes()
        .filter(|c| *c != b'-')
        .map(|c| (c as char).to_digit(16).unwrap() as u8)
        .collect();
    let mut out = [0u8; 16];
    for (i, pair) in digits.as_chunks::<2>().0.iter().enumerate() {
        out[i] = pair[0] << 4 | pair[1];
    }
    Ok(out)
}

/// Formats 16 bytes as a lowercase uuid.
pub fn stringify(bytes: &[u8; 16]) -> String {
    let h = sha::hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// A namespace that has been checked to be a uuid: v5 uuids in it cannot fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    text: String,
    bytes: [u8; 16],
}

impl Namespace {
    /// Checks a uuid, failing with npm's `TypeError` message.
    pub fn parse(text: &str) -> Result<Self, String> {
        Ok(Namespace {
            text: text.to_owned(),
            bytes: parse(text)?,
        })
    }

    /// The callsite uuids' constant namespace.
    pub fn callsite() -> Self {
        Namespace::parse(CALLSITE_NAMESPACE).unwrap_or_else(|_| unreachable!("a valid uuid"))
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// `uuid.v5(name, namespace)`.
    pub fn v5(&self, name: &str) -> String {
        v5_bytes(name, &self.bytes)
    }
}

/// `uuid.v5(name, namespace)`. Fails with npm's message when the namespace is not a valid uuid.
pub fn v5(name: &str, namespace: &str) -> Result<String, String> {
    Ok(v5_bytes(name, &parse(namespace)?))
}

fn v5_bytes(name: &str, namespace: &[u8; 16]) -> String {
    let mut data = namespace.to_vec();
    data.extend_from_slice(name.as_bytes());
    let hash = sha::sha1(&data);
    let mut bytes: [u8; 16] = hash[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    stringify(&bytes)
}

/// A random (v4-shaped) uuid from the operating system's generator: the build seed.
pub fn random_v4() -> String {
    let mut bytes = [0u8; 16];
    crate::random::fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    stringify(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_v4_is_a_valid_uuid() {
        let a = random_v4();
        assert!(validate(&a) && a.as_bytes()[14] == b'4');
        assert_ne!(a, random_v4());
    }
}
