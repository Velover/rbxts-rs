//! Hashids as npm `hashids` 2.3.0 encodes (the transformer's `new Hashids(salt, 2).encode(id)` for short, tiny and
//! obfuscated ids). A port of its `encode` (no `decode`: the transformer never decodes): characters are Unicode code
//! points, as `Array.from` splits them.

const DEFAULT_ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
const DEFAULT_SEPS: &str = "cfhistuCFHISTU";
const SEPARATOR_DIV: f64 = 3.5;
const GUARD_DIV: f64 = 12.0;
const MODULO_PART: u64 = 100;

#[derive(Debug, Clone)]
pub struct Hashids {
    salt: Vec<char>,
    min_length: usize,
    alphabet: Vec<char>,
    seps: Vec<char>,
    guards: Vec<char>,
}

fn keep_unique(chars: &[char]) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in chars {
        if !out.contains(c) {
            out.push(*c);
        }
    }
    out
}

/// The consistent shuffle every Hashids implementation shares.
fn shuffle(alphabet: &[char], salt: &[char]) -> Vec<char> {
    if salt.is_empty() {
        return alphabet.to_vec();
    }
    let mut out = alphabet.to_vec();
    let mut p: u64 = 0;
    let mut v = 0usize;
    let mut i = out.len().saturating_sub(1);
    while i > 0 {
        v %= salt.len();
        let integer = salt[v] as u64;
        p += integer;
        let j = ((integer + v as u64 + p) % i as u64) as usize;
        out.swap(i, j);
        i -= 1;
        v += 1;
    }
    out
}

fn to_alphabet(mut value: u64, alphabet: &[char]) -> Vec<char> {
    let len = alphabet.len() as u64;
    let mut id = Vec::new();
    loop {
        id.insert(0, alphabet[(value % len) as usize]);
        value /= len;
        if value == 0 {
            break;
        }
    }
    id
}

impl Hashids {
    /// `new Hashids(salt, minLength)` with the default alphabet.
    pub fn new(salt: &str, min_length: usize) -> Self {
        Self::with_alphabet(salt, min_length, DEFAULT_ALPHABET, DEFAULT_SEPS)
            .expect("the default alphabet is valid")
    }

    /// `new Hashids(salt, minLength, alphabet, seps)`.
    pub fn with_alphabet(
        salt: &str,
        min_length: usize,
        alphabet: &str,
        seps: &str,
    ) -> Result<Self, String> {
        let salt: Vec<char> = salt.chars().collect();
        let alphabet_chars: Vec<char> = alphabet.chars().collect();
        let seps_chars: Vec<char> = seps.chars().collect();
        let unique = keep_unique(&alphabet_chars);
        if unique.len() < 16 {
            return Err(format!(
                "Hashids: alphabet must contain at least 16 unique characters, provided: {}",
                unique.iter().collect::<String>()
            ));
        }
        let mut alphabet: Vec<char> = unique
            .iter()
            .copied()
            .filter(|c| !seps_chars.contains(c))
            .collect();
        let filtered_seps: Vec<char> = seps_chars
            .iter()
            .copied()
            .filter(|c| unique.contains(c))
            .collect();
        let mut seps = shuffle(&filtered_seps, &salt);
        if seps.is_empty() || alphabet.len() as f64 / seps.len() as f64 > SEPARATOR_DIV {
            let seps_length = (alphabet.len() as f64 / SEPARATOR_DIV).ceil() as usize;
            if seps_length > seps.len() {
                let diff = seps_length - seps.len();
                seps.extend_from_slice(&alphabet[..diff.min(alphabet.len())]);
                alphabet = alphabet[diff.min(alphabet.len())..].to_vec();
            }
        }
        alphabet = shuffle(&alphabet, &salt);
        let guard_count = (alphabet.len() as f64 / GUARD_DIV).ceil() as usize;
        let guards;
        if alphabet.len() < 3 {
            guards = seps[..guard_count.min(seps.len())].to_vec();
            seps = seps[guard_count.min(seps.len())..].to_vec();
        } else {
            guards = alphabet[..guard_count].to_vec();
            alphabet = alphabet[guard_count..].to_vec();
        }
        Ok(Hashids {
            salt,
            min_length,
            alphabet,
            seps,
            guards,
        })
    }

    /// `encode(numbers)`. Every number must be a safe integer (below 2^53), as the transformer's counters are.
    pub fn encode(&self, numbers: &[u64]) -> String {
        if numbers.is_empty() {
            return String::new();
        }
        let mut alphabet = self.alphabet.clone();
        let numbers_id_int: u64 = numbers
            .iter()
            .enumerate()
            .map(|(i, n)| n % (i as u64 + MODULO_PART))
            .sum();
        let lottery = alphabet[(numbers_id_int % alphabet.len() as u64) as usize];
        let mut ret: Vec<char> = vec![lottery];
        for (i, &number) in numbers.iter().enumerate() {
            let mut buffer = vec![lottery];
            buffer.extend_from_slice(&self.salt);
            buffer.extend_from_slice(&alphabet);
            alphabet = shuffle(&alphabet, &buffer);
            let last = to_alphabet(number, &alphabet);
            ret.extend_from_slice(&last);
            if i + 1 < numbers.len() {
                let char_code = last[0] as u64 + i as u64;
                let extra = number % char_code;
                ret.push(self.seps[(extra % self.seps.len() as u64) as usize]);
            }
        }
        if ret.len() < self.min_length {
            let guards = &self.guards;
            let prefix = (numbers_id_int + ret[0] as u64) % guards.len() as u64;
            ret.insert(0, guards[prefix as usize]);
            if ret.len() < self.min_length {
                let suffix = (numbers_id_int + ret[2] as u64) % guards.len() as u64;
                ret.push(guards[suffix as usize]);
            }
        }
        let half_length = alphabet.len() / 2;
        while ret.len() < self.min_length {
            alphabet = shuffle(&alphabet, &alphabet.clone());
            let mut next: Vec<char> = alphabet[half_length..].to_vec();
            next.extend_from_slice(&ret);
            next.extend_from_slice(&alphabet[..half_length]);
            ret = next;
            if ret.len() > self.min_length {
                // `ret.slice(excess / 2, excess / 2 + minLength)`: a fractional start truncates.
                let start = (ret.len() - self.min_length) / 2;
                ret = ret[start..start + self.min_length].to_vec();
            }
        }
        ret.into_iter().collect()
    }

    /// `encode(n)` for one number.
    pub fn encode_one(&self, n: u64) -> String {
        self.encode(&[n])
    }
}
