//! Project templates: built-in and user-defined, rendered with a tiny
//! placeholder language and live version lookups.

pub mod builtin;
pub mod custom;
pub mod render;
pub mod save;
pub mod tokens;
pub mod wrapper;

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::registry::{Registry, Repo};
use render::Vars;

/// A file to be written into the new project.
#[derive(Debug, Clone)]
pub struct OutFile {
    /// Relative path with `/` separators.
    pub path: String,
    pub data: Vec<u8>,
    pub exec: bool,
}

/// A template input file: path template + raw bytes.
pub struct Input {
    pub path: String,
    pub data: Vec<u8>,
}

fn as_text(data: &[u8]) -> Option<&str> {
    if data.contains(&0) {
        return None;
    }
    std::str::from_utf8(data).ok()
}

/// Placeholders a set of inputs needs, split by kind.
pub fn scan_inputs(inputs: &[Input]) -> render::Found {
    let mut found = render::Found::default();
    for i in inputs {
        let mut merge = |f: render::Found| {
            found.variables.extend(f.variables);
            found.flags.extend(f.flags);
            found.tokens.extend(f.tokens);
        };
        merge(render::scan(&i.path));
        if let Some(t) = as_text(&i.data) {
            merge(render::scan(t));
        }
    }
    found
}

/// Renders inputs: pass 1 collects version tokens, they are resolved (in
/// parallel), pass 2 produces the final files.
pub async fn render_inputs(
    reg: Option<&Registry>,
    inputs: Vec<Input>,
    vars: &Vars,
    repos: &[Repo],
    extra_tokens: &[String],
) -> Result<(Vec<OutFile>, BTreeMap<String, String>)> {
    let mut texts: Vec<Option<String>> = Vec::new();
    let mut all_tokens: Vec<String> = extra_tokens.to_vec();
    for i in &inputs {
        match as_text(&i.data) {
            Some(t) => {
                let pass1 = render::render(t, vars, None).with_context(|| format!("while rendering {}", i.path))?;
                for tok in render::scan(&pass1).tokens {
                    if !all_tokens.contains(&tok) {
                        all_tokens.push(tok);
                    }
                }
                texts.push(Some(pass1));
            }
            None => texts.push(None),
        }
    }
    let resolved = if all_tokens.is_empty() {
        BTreeMap::new()
    } else {
        let reg = reg.context("version lookups require network access")?;
        tokens::resolve(reg, &all_tokens, repos).await?
    };
    let mut out = Vec::new();
    for (input, text) in inputs.into_iter().zip(texts) {
        let path = render::render(&input.path, vars, Some(&resolved))
            .with_context(|| format!("while rendering the path {}", input.path))?;
        let data = match text {
            Some(t) => render::render(&t, vars, Some(&resolved))?.into_bytes(),
            None => input.data,
        };
        out.push(OutFile { path, data, exec: false });
    }
    Ok((out, resolved))
}
