//! Seeded shuffles for obfuscation (`obfuscateArray`; DESIGN §10.4).
//!
//! The TS transformer shuffled with `Math.random`. Here the order is a function of a seed text (the build seed, the
//! file, the call's place and what is shuffled), so every checker gives the same output and a pinned build seed
//! reproduces it, while a new clean build (a new seed) gives a new order. The random stream is SHA-256 in counter
//! mode over the seed, which needs no new dependency.

/// A Fisher-Yates shuffle of `items`, driven by `seed`.
pub fn shuffle<T>(items: &mut [T], seed: &str) {
    let mut stream = Stream { seed: seed.as_bytes().to_vec(), counter: 0, block: [0; 32], used: 32 };
    for i in (1..items.len()).rev() {
        let j = stream.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

struct Stream {
    seed: Vec<u8>,
    counter: u64,
    block: [u8; 32],
    used: usize,
}

impl Stream {
    fn next_u64(&mut self) -> u64 {
        if self.used + 8 > self.block.len() {
            let mut input = self.seed.clone();
            input.extend_from_slice(&self.counter.to_le_bytes());
            self.block = crate::sha::sha256(&input);
            self.counter += 1;
            self.used = 0;
        }
        let bytes: [u8; 8] = self.block[self.used..self.used + 8].try_into().expect("8 bytes");
        self.used += 8;
        u64::from_le_bytes(bytes)
    }

    /// Uniform in `0..bound` (rejection sampling, so no modulo bias).
    fn below(&mut self, bound: u64) -> u64 {
        let zone = u64::MAX - u64::MAX % bound;
        loop {
            let value = self.next_u64();
            if value < zone {
                return value % bound;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shuffle;

    #[test]
    fn a_seed_gives_one_permutation() {
        let mut a: Vec<u32> = (0..20).collect();
        let mut b = a.clone();
        shuffle(&mut a, "seed|file|12|shuffle-array|1");
        shuffle(&mut b, "seed|file|12|shuffle-array|1");
        assert_eq!(a, b);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert_ne!(a, (0..20).collect::<Vec<_>>());
        let mut c: Vec<u32> = (0..20).collect();
        shuffle(&mut c, "another seed");
        assert_ne!(a, c);
    }
}
