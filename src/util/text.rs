//! Text helpers: line endings, BOM handling, indentation and diffs.
//!
//! All edits in kiln are done as surgical splices on the original text so that
//! formatting and comments survive. These helpers keep CRLF files CRLF.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use similar::{ChangeTag, TextDiff};

use super::term;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn detect(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count().saturating_sub(crlf);
        if crlf > lf { Self::Crlf } else { Self::Lf }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }

    /// Converts every line break in `text` to this line ending.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Lf => text.to_string(),
            Self::Crlf => text.replace("\r\n", "\n").replace('\n', "\r\n"),
        }
    }
}

/// A text file loaded into memory with enough metadata to write it back
/// byte-for-byte compatible (BOM and line endings).
#[derive(Debug, Clone)]
pub struct TextFile {
    pub path: PathBuf,
    pub original: String,
    pub text: String,
    pub eol: LineEnding,
    pub bom: bool,
}

impl TextFile {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        let mut s = String::from_utf8(bytes).with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let bom = s.starts_with('\u{feff}');
        if bom {
            s.remove(0);
        }
        let eol = LineEnding::detect(&s);
        Ok(Self { path: path.to_path_buf(), original: s.clone(), text: s, eol, bom })
    }

    pub fn is_modified(&self) -> bool {
        self.text != self.original
    }

    pub fn write(&self) -> Result<()> {
        let mut out = String::with_capacity(self.text.len() + 3);
        if self.bom {
            out.push('\u{feff}');
        }
        out.push_str(&self.text);
        std::fs::write(&self.path, out).with_context(|| format!("could not write {}", self.path.display()))
    }
}

/// Start offset of the line containing byte offset `pos` (works for any offset,
/// even in the middle of a multi-byte character).
pub fn line_start(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    text.as_bytes()[..pos].iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0)
}

/// Returns the leading whitespace of the line that contains byte offset `pos`.
pub fn line_indent(text: &str, pos: usize) -> &str {
    let start = line_start(text, pos);
    let rest = &text[start..];
    let len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    &rest[..len]
}

/// Offset just after the line break that ends the line containing `pos`
/// (or the end of the text).
pub fn line_end_inclusive(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    match text.as_bytes()[pos..].iter().position(|b| *b == b'\n') {
        Some(i) => pos + i + 1,
        None => text.len(),
    }
}

/// A pending text edit expressed against the *original* offsets of one scan.
#[derive(Debug, Clone)]
pub struct Edit {
    pub range: std::ops::Range<usize>,
    pub replacement: String,
}

/// Applies edits (computed against the same text) from the back to the front.
pub fn apply_edits(text: &str, edits: &[Edit]) -> Result<String> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    let mut out = text.to_string();
    let mut limit = usize::MAX;
    for e in sorted {
        anyhow::ensure!(e.range.end <= limit, "internal error: overlapping edits");
        out.replace_range(e.range.clone(), &e.replacement);
        limit = e.range.start;
    }
    Ok(out)
}

/// Renders a unified diff with optional colors.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    let _ = writeln!(out, "{}", term::bold(format!("--- a/{path}")));
    let _ = writeln!(out, "{}", term::bold(format!("+++ b/{path}")));
    for group in diff.grouped_ops(3) {
        let first = &group[0];
        let last = &group[group.len() - 1];
        let old_start = first.old_range().start + 1;
        let old_len = last.old_range().end - first.old_range().start;
        let new_start = first.new_range().start + 1;
        let new_len = last.new_range().end - first.new_range().start;
        let _ = writeln!(out, "{}", term::cyan(format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@")));
        for op in &group {
            for change in diff.iter_changes(op) {
                let value = change.value().trim_end_matches(['\r', '\n']);
                let line = match change.tag() {
                    ChangeTag::Delete => term::red(format!("-{value}")),
                    ChangeTag::Insert => term::green(format!("+{value}")),
                    ChangeTag::Equal => format!(" {value}"),
                };
                let _ = writeln!(out, "{line}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_line_endings() {
        assert_eq!(LineEnding::detect("a\nb\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("a\r\nb\r\n"), LineEnding::Crlf);
        assert_eq!(LineEnding::detect("single"), LineEnding::Lf);
    }

    #[test]
    fn apply_normalises_to_crlf() {
        assert_eq!(LineEnding::Crlf.apply("a\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn edits_apply_back_to_front() {
        let edits =
            vec![Edit { range: 0..1, replacement: "XX".into() }, Edit { range: 2..3, replacement: "YY".into() }];
        assert_eq!(apply_edits("a-b", &edits).unwrap(), "XX-YY");
    }

    #[test]
    fn overlapping_edits_are_rejected() {
        let edits = vec![Edit { range: 0..2, replacement: "X".into() }, Edit { range: 1..3, replacement: "Y".into() }];
        assert!(apply_edits("abc", &edits).is_err());
    }

    #[test]
    fn indent_of_line() {
        assert_eq!(line_indent("a\n    foo\n", 7), "    ");
    }
}
