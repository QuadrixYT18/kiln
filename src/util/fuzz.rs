//! Test-only helpers for robustness ("never panic on broken input") tests.
//! A deterministic xorshift PRNG keeps the tests reproducible without extra crates.

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Number of fuzz rounds; raise with `KILN_FUZZ_ROUNDS=200000 cargo test fuzz`.
pub fn rounds() -> usize {
    std::env::var("KILN_FUZZ_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000)
}

/// Randomly deletes, inserts (from `snippets`) and truncates.
pub fn mutate(rng: &mut Rng, text: &str, snippets: &[&str]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for _ in 0..1 + rng.below(6) {
        if chars.is_empty() {
            break;
        }
        let i = rng.below(chars.len());
        match rng.below(3) {
            0 => {
                chars.remove(i);
            }
            1 => {
                let s = snippets[rng.below(snippets.len())];
                for (k, c) in s.chars().enumerate() {
                    chars.insert((i + k).min(chars.len()), c);
                }
            }
            _ => chars.truncate(i),
        }
    }
    chars.into_iter().collect()
}
