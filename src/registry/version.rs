//! Maven-style version parsing, ordering and stability classification.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Num(u64),
    Str(String),
}

#[derive(Debug, Clone)]
pub struct Version {
    raw: String,
    toks: Vec<Tok>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn label(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

/// How strictly unstable versions are filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stability {
    /// Only stable releases.
    #[default]
    Stable,
    /// Stable releases and `-SNAPSHOT` builds only (e.g. Paper/Velocity APIs).
    AllowSnapshot,
    /// Everything.
    Any,
}

const UNSTABLE: &[&str] = &[
    "alpha",
    "beta",
    "m",
    "milestone",
    "rc",
    "cr",
    "snapshot",
    "preview",
    "pre",
    "dev",
    "eap",
    "ea",
    "nightly",
    "canary",
    "next",
    "experimental",
    "incubating",
    "candidate",
    "pr",
    "test",
    "rcp",
];

/// Qualifiers that carry no variant information.
const NEUTRAL: &[&str] = &["final", "ga", "release", "sp", "v", "r"];

impl Version {
    pub fn new(raw: &str) -> Self {
        Self { raw: raw.to_string(), toks: tokenize(raw) }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    fn is_unstable_tok(&self, i: usize) -> bool {
        match &self.toks[i] {
            Tok::Str(s) => {
                if UNSTABLE.contains(&s.as_str()) {
                    return true;
                }
                // `1.0a1`, `2.0b2`: a single letter between a number and a number.
                (s == "a" || s == "b")
                    && i > 0
                    && matches!(self.toks[i - 1], Tok::Num(_))
                    && matches!(self.toks.get(i + 1), Some(Tok::Num(_)))
            }
            Tok::Num(_) => false,
        }
    }

    pub fn is_stable(&self) -> bool {
        !(0..self.toks.len()).any(|i| self.is_unstable_tok(i))
    }

    fn only_snapshot_unstable(&self) -> bool {
        (0..self.toks.len())
            .filter(|&i| self.is_unstable_tok(i))
            .all(|i| matches!(&self.toks[i], Tok::Str(s) if s == "snapshot"))
    }

    pub fn allowed(&self, stability: Stability) -> bool {
        match stability {
            Stability::Stable => self.is_stable(),
            Stability::AllowSnapshot => self.is_stable() || self.only_snapshot_unstable(),
            Stability::Any => true,
        }
    }

    /// Variant suffix such as `jre` / `android` (Guava) or `groovy` (Spock).
    /// Updates must stay within the same family.
    pub fn family(&self) -> String {
        let mut parts = Vec::new();
        for (i, t) in self.toks.iter().enumerate() {
            if let Tok::Str(s) = t {
                if self.is_unstable_tok(i) || NEUTRAL.contains(&s.as_str()) {
                    continue;
                }
                parts.push(s.as_str());
            }
        }
        parts.join("-")
    }

    fn leading_nums(&self) -> Vec<u64> {
        self.toks.iter().map_while(|t| if let Tok::Num(n) = t { Some(*n) } else { None }).collect()
    }

    /// Classifies the step from `self` to `newer`.
    pub fn bump_to(&self, newer: &Version) -> Bump {
        let a = self.leading_nums();
        let b = newer.leading_nums();
        let get = |v: &Vec<u64>, i: usize| v.get(i).copied().unwrap_or(0);
        if get(&a, 0) != get(&b, 0) {
            Bump::Major
        } else if get(&a, 1) != get(&b, 1) {
            Bump::Minor
        } else {
            Bump::Patch
        }
    }

    /// Dynamic selectors (`1.+`, `[1.0,2.0)`, `latest.release`) cannot be updated.
    pub fn is_dynamic(raw: &str) -> bool {
        let r = raw.trim();
        r.ends_with('+')
            || r.starts_with('[')
            || r.starts_with('(')
            || r.starts_with("latest.")
            || r.contains("${")
            || r.is_empty()
    }
}

fn tokenize(raw: &str) -> Vec<Tok> {
    let raw = raw.trim();
    let raw = raw.strip_prefix(['v', 'V']).filter(|r| r.starts_with(|c: char| c.is_ascii_digit())).unwrap_or(raw);
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut cur_digit: Option<bool> = None;
    let flush = |cur: &mut String, digit: Option<bool>, toks: &mut Vec<Tok>| {
        if cur.is_empty() {
            return;
        }
        match (digit, cur.parse::<u64>()) {
            (Some(true), Ok(n)) => toks.push(Tok::Num(n)),
            _ => toks.push(Tok::Str(cur.to_ascii_lowercase())),
        }
        cur.clear();
    };
    for c in raw.chars() {
        if matches!(c, '.' | '-' | '_' | '+') {
            flush(&mut cur, cur_digit, &mut toks);
            cur_digit = None;
            continue;
        }
        let is_digit = c.is_ascii_digit();
        if let Some(prev) = cur_digit
            && prev != is_digit
        {
            flush(&mut cur, cur_digit, &mut toks);
        }
        cur_digit = Some(is_digit);
        cur.push(c);
    }
    flush(&mut cur, cur_digit, &mut toks);
    toks
}

fn qualifier_rank(s: &str) -> (u8, &str) {
    match s {
        "alpha" | "a" => (1, ""),
        "beta" | "b" => (2, ""),
        "milestone" | "m" => (3, ""),
        "rc" | "cr" => (4, ""),
        "snapshot" => (5, ""),
        "" | "ga" | "final" | "release" => (6, ""),
        "sp" => (7, ""),
        other => (8, other),
    }
}

fn cmp_tok(a: Option<&Tok>, b: Option<&Tok>) -> Ordering {
    use Tok::*;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (Some(Num(x)), Some(Num(y))) => x.cmp(y),
        (Some(Str(x)), Some(Str(y))) => qualifier_rank(x).cmp(&qualifier_rank(y)),
        (Some(Num(_)), Some(Str(_))) => Ordering::Greater,
        (Some(Str(_)), Some(Num(_))) => Ordering::Less,
        // Missing token behaves like `0` / the release qualifier.
        (Some(Num(x)), None) => x.cmp(&0),
        (None, Some(Num(y))) => 0u64.cmp(y),
        (Some(Str(x)), None) => qualifier_rank(x).cmp(&qualifier_rank("")),
        (None, Some(Str(y))) => qualifier_rank("").cmp(&qualifier_rank(y)),
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.toks.len().max(other.toks.len());
        for i in 0..n {
            let o = cmp_tok(self.toks.get(i), other.toks.get(i));
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

/// The best newer version in each bump class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidates {
    pub patch: Option<String>,
    pub minor: Option<String>,
    pub major: Option<String>,
}

impl Candidates {
    pub fn is_empty(&self) -> bool {
        self.patch.is_none() && self.minor.is_none() && self.major.is_none()
    }

    pub fn highest(&self) -> Option<(&str, Bump)> {
        self.major
            .as_deref()
            .map(|v| (v, Bump::Major))
            .or_else(|| self.minor.as_deref().map(|v| (v, Bump::Minor)))
            .or_else(|| self.patch.as_deref().map(|v| (v, Bump::Patch)))
    }
}

/// Computes update candidates for `current` among `available`.
pub fn candidates(current: &str, available: &[String], stability: Stability) -> Candidates {
    let cur = Version::new(current);
    // A project already on a pre-release may follow pre-releases.
    let stability = if !cur.is_stable() && stability == Stability::Stable { Stability::Any } else { stability };
    let family = cur.family();
    let mut out = Candidates::default();
    let mut best: [Option<Version>; 3] = [None, None, None];
    for raw in available {
        let v = Version::new(raw);
        if v <= cur || !v.allowed(stability) || v.family() != family {
            continue;
        }
        let slot = match cur.bump_to(&v) {
            Bump::Patch => 0,
            Bump::Minor => 1,
            Bump::Major => 2,
        };
        if best[slot].as_ref().is_none_or(|b| v > *b) {
            best[slot] = Some(v);
        }
    }
    out.patch = best[0].take().map(|v| v.raw);
    out.minor = best[1].take().map(|v| v.raw);
    out.major = best[2].take().map(|v| v.raw);
    out
}

/// Highest allowed version overall (used for `kiln add` and templates).
pub fn latest(available: &[String], stability: Stability) -> Option<String> {
    available.iter().map(|s| Version::new(s)).filter(|v| v.allowed(stability)).max().map(|v| v.raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::new(s)
    }

    #[test]
    fn orders_numerically() {
        assert!(v("1.10.0") > v("1.9.9"));
        assert!(v("2.0") > v("1.99.99"));
        assert_eq!(v("1.0"), v("1.0.0"));
    }

    #[test]
    fn orders_qualifiers() {
        assert!(v("1.0.0") > v("1.0.0-rc1"));
        assert!(v("1.0.0-rc2") > v("1.0.0-rc1"));
        assert!(v("1.0.0-rc1") > v("1.0.0-beta2"));
        assert!(v("1.0.0-beta1") > v("1.0.0-alpha9"));
        assert!(v("1.0.0") > v("1.0.0-SNAPSHOT"));
        assert!(v("1.0.0.Final") == v("1.0.0"));
        assert!(v("1.0.1") > v("1.0.0.Final"));
        assert!(v("5.0.0-M1") < v("5.0.0-RC1"));
    }

    #[test]
    fn stability() {
        for s in ["1.0.0-alpha", "1.0-beta-2", "2.0.0-RC1", "3.0.0-M4", "1.0-SNAPSHOT", "1.0a1", "1.2.3-dev"] {
            assert!(!v(s).is_stable(), "{s}");
        }
        for s in ["1.0.0", "1.0.0.Final", "5.2.0.RELEASE", "33.0.0-jre", "2.3-groovy-4.0", "1.0.0-SP1"] {
            assert!(v(s).is_stable(), "{s}");
        }
    }

    #[test]
    fn families_keep_variants_apart() {
        assert_eq!(v("33.0.0-jre").family(), "jre");
        assert_eq!(v("33.0.0-android").family(), "android");
        assert_eq!(v("1.0.0").family(), "");
        let avail: Vec<String> =
            ["33.0.0-jre", "33.1.0-jre", "33.1.0-android", "34.0.0-android"].iter().map(|s| s.to_string()).collect();
        let c = candidates("33.0.0-jre", &avail, Stability::Stable);
        assert_eq!(c.minor.as_deref(), Some("33.1.0-jre"));
        assert!(c.major.is_none());
    }

    #[test]
    fn classifies_bumps() {
        let avail: Vec<String> = ["1.2.3", "1.2.4", "1.2.9-rc1", "1.3.0", "1.4.1", "2.0.0", "3.0.0-beta1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = candidates("1.2.3", &avail, Stability::Stable);
        assert_eq!(c.patch.as_deref(), Some("1.2.4"));
        assert_eq!(c.minor.as_deref(), Some("1.4.1"));
        assert_eq!(c.major.as_deref(), Some("2.0.0"));
        let c = candidates("1.2.3", &avail, Stability::Any);
        assert_eq!(c.major.as_deref(), Some("3.0.0-beta1"));
        assert_eq!(c.highest().unwrap().1, Bump::Major);
    }

    #[test]
    fn prerelease_users_follow_prereleases() {
        let avail: Vec<String> = ["1.0.0-beta1", "1.0.0-beta2", "1.0.0"].iter().map(|s| s.to_string()).collect();
        let c = candidates("1.0.0-beta1", &avail, Stability::Stable);
        assert_eq!(c.highest().unwrap().0, "1.0.0");
    }

    #[test]
    fn snapshot_mode() {
        let avail: Vec<String> =
            ["1.20.4-R0.1-SNAPSHOT", "1.21.4-R0.1-SNAPSHOT", "1.21.5-R0.1-rc1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(latest(&avail, Stability::Stable), None);
        assert_eq!(latest(&avail, Stability::AllowSnapshot).as_deref(), Some("1.21.4-R0.1-SNAPSHOT"));
        assert_eq!(latest(&avail, Stability::Any).as_deref(), Some("1.21.5-R0.1-rc1"));
    }

    #[test]
    fn dynamic_versions() {
        for s in ["1.+", "[1.0,2.0)", "latest.release", "+", "${foo}"] {
            assert!(Version::is_dynamic(s), "{s}");
        }
        assert!(!Version::is_dynamic("1.2.3"));
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use crate::util::fuzz::{Rng, mutate, rounds};

    #[test]
    fn version_logic_never_panics_and_ordering_is_total() {
        let snippets = &[
            ".",
            "-",
            "_",
            "+",
            "rc",
            "RC1",
            "beta",
            "SNAPSHOT",
            "ü",
            "99999999999999999999999",
            "0",
            "jre",
            "v",
            " ",
        ];
        let mut rng = Rng(7);
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..rounds() {
            let a = mutate(&mut rng, "1.2.3-rc1", snippets);
            let b = mutate(&mut rng, "2.0.0.Final", snippets);
            let (va, vb) = (Version::new(&a), Version::new(&b));
            // antisymmetry
            assert_eq!(va.cmp(&vb), vb.cmp(&va).reverse(), "{a:?} vs {b:?}");
            let _ = va.is_stable();
            let _ = va.family();
            let _ = va.bump_to(&vb);
            seen.push(a);
            if seen.len() > 20 {
                let _ = candidates(&seen[0], &seen, Stability::Any);
                let _ = latest(&seen, Stability::Stable);
                seen.clear();
            }
        }
    }
}
