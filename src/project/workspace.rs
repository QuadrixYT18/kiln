//! In-memory view of the files kiln may change.
//!
//! All commands edit through a [`Workspace`]; nothing touches the disk until
//! [`Workspace::write_all`], which makes `--dry-run` and `--verify` rollbacks trivial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::util::text::{LineEnding, TextFile, unified_diff};

#[derive(Default)]
pub struct Workspace {
    files: BTreeMap<PathBuf, TextFile>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    fn load(&mut self, path: &Path) -> Result<&mut TextFile> {
        if !self.files.contains_key(path) {
            let f = TextFile::read(path)?;
            self.files.insert(path.to_path_buf(), f);
        }
        Ok(self.files.get_mut(path).expect("just inserted"))
    }

    pub fn text(&mut self, path: &Path) -> Result<String> {
        Ok(self.load(path)?.text.clone())
    }

    /// Replaces the content; line endings are normalised to the file's own style.
    pub fn set(&mut self, path: &Path, new_text: String) -> Result<()> {
        let f = self.load(path)?;
        f.text = if f.eol == LineEnding::Crlf { LineEnding::Crlf.apply(&new_text) } else { new_text };
        Ok(())
    }

    pub fn has_changes(&self) -> bool {
        self.files.values().any(TextFile::is_modified)
    }

    /// Colored unified diff of all modified files, paths shown relative to `root`.
    pub fn diff(&self, root: &Path) -> String {
        let mut out = String::new();
        for f in self.files.values().filter(|f| f.is_modified()) {
            let rel = f.path.strip_prefix(root).unwrap_or(&f.path);
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push_str(&unified_diff(&rel, &f.original, &f.text));
            out.push('\n');
        }
        out
    }

    pub fn write_all(&mut self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for f in self.files.values_mut().filter(|f| f.is_modified()) {
            f.write()?;
            written.push(f.path.clone());
        }
        Ok(written)
    }

    /// Restores the original content on disk for every file that was modified
    /// (used by `kiln update --verify` rollbacks).
    pub fn rollback_on_disk(&self) -> Result<()> {
        for f in self.files.values().filter(|f| f.is_modified()) {
            let mut restored = f.clone();
            restored.text = f.original.clone();
            restored.write()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_semantics_and_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "one\r\ntwo\r\n").unwrap();
        let mut ws = Workspace::new();
        assert!(!ws.has_changes());
        ws.set(&p, "one\ntwo\nthree\n".to_string()).unwrap();
        assert!(ws.has_changes());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\r\ntwo\r\n");
        ws.write_all().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\r\ntwo\r\nthree\r\n");
        ws.rollback_on_disk().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\r\ntwo\r\n");
    }

    #[test]
    fn bom_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.txt");
        std::fs::write(&p, "\u{feff}a\n").unwrap();
        let mut ws = Workspace::new();
        ws.set(&p, "a\nb\n".into()).unwrap();
        ws.write_all().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), "\u{feff}a\nb\n".as_bytes());
    }
}
