//! Gradle build script scanning (Kotlin and Groovy DSL) and surgical edits.
//!
//! Build scripts are *not* parsed into an AST. Instead the source is "masked"
//! (comments and string contents blanked out, same byte length) so that
//! structure (braces, parentheses, statements) can be found reliably, while all
//! values are read from the original text through byte ranges. Edits splice
//! those ranges, which keeps formatting and comments untouched.

use std::collections::BTreeMap;
use std::ops::Range;

use anyhow::{Result, bail};

use crate::registry::Coord;
use crate::util::text::{LineEnding, line_end_inclusive, line_indent, line_start};

// ---------------------------------------------------------------------------
// Masking & structure
// ---------------------------------------------------------------------------

/// Blanks comments and string contents (keeping quotes and newlines) so that
/// offsets in the result line up with the source.
pub fn mask(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, i: usize| {
        if out[i] != b'\n' {
            out[i] = b' ';
        }
    };
    let mut i = 0;
    while i < n {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < n && b[i] != b'\n' {
                    blank(&mut out, i);
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                blank(&mut out, i);
                blank(&mut out, i + 1);
                i += 2;
                while i < n && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    blank(&mut out, i);
                    i += 1;
                }
                if i < n {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                }
            }
            q @ (b'"' | b'\'') => {
                let triple = b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q);
                i += if triple { 3 } else { 1 };
                while i < n {
                    if triple {
                        if b[i] == q && b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q) {
                            i += 3;
                            break;
                        }
                    } else if b[i] == b'\\' {
                        blank(&mut out, i);
                        if i + 1 < n {
                            blank(&mut out, i + 1);
                        }
                        i += 2;
                        continue;
                    } else if b[i] == q {
                        i += 1;
                        break;
                    } else if b[i] == b'\n' {
                        // Unterminated single-line string: stop masking.
                        break;
                    } else if q == b'"' && b[i] == b'$' && b.get(i + 1) == Some(&b'{') {
                        // Kotlin/Groovy template: skip to the matching brace.
                        let mut depth = 0;
                        while i < n {
                            blank(&mut out, i);
                            if b[i] == b'{' {
                                depth += 1;
                            } else if b[i] == b'}' {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            i += 1;
                        }
                        continue;
                    }
                    blank(&mut out, i);
                    i += 1;
                }
                // Restore the closing quote(s) that the loop consumed unblanked.
            }
            _ => i += 1,
        }
    }
    out
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub open: usize,
    pub close: usize,
    pub parent: Option<usize>,
}

fn matching_open_paren(m: &[u8], close: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = close as isize;
    while i >= 0 {
        match m[i as usize] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i as usize);
                }
            }
            _ => {}
        }
        i -= 1;
    }
    None
}

pub fn matching_close(m: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match m[open] {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0i32;
    for (i, &ch) in m.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn block_name(m: &[u8], open: usize) -> String {
    let mut i = open;
    while i > 0 && m[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i > 0 && m[i - 1] == b')' {
        if let Some(p) = matching_open_paren(m, i - 1) {
            i = p;
            while i > 0 && m[i - 1].is_ascii_whitespace() {
                i -= 1;
            }
        }
    }
    let end = i;
    while i > 0 && is_ident(m[i - 1]) {
        i -= 1;
    }
    String::from_utf8_lossy(&m[i..end]).into_owned()
}

pub fn blocks(m: &[u8]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (i, &c) in m.iter().enumerate() {
        match c {
            b'{' => {
                out.push(Block { name: block_name(m, i), open: i, close: m.len(), parent: stack.last().copied() });
                stack.push(out.len() - 1);
            }
            b'}' => {
                if let Some(idx) = stack.pop() {
                    out[idx].close = i;
                }
            }
            _ => {}
        }
    }
    out
}

fn has_ancestor(blocks: &[Block], mut idx: Option<usize>, name: &str) -> bool {
    while let Some(i) = idx {
        if blocks[i].name == name {
            return true;
        }
        idx = blocks[i].parent;
    }
    false
}

/// Splits the interior of a block into statements (ranges without surrounding whitespace).
pub fn statements(m: &[u8], from: usize, to: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut last_sig = 0usize;
    let mut i = from;
    while i < to {
        let c = m[i];
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if !c.is_ascii_whitespace() {
            if start.is_none() {
                start = Some(i);
            }
            last_sig = i;
        }
        let at_end = i + 1 == to;
        if (c == b'\n' && depth <= 0) || (c == b';' && depth <= 0) || at_end {
            if let Some(s) = start {
                let e = last_sig + 1;
                let last = m[last_sig];
                let continues = c == b'\n' && matches!(last, b',' | b'+' | b'=' | b'.' | b'(' | b'&' | b'|') && !at_end;
                if !continues {
                    if c != b';' || last != b';' {
                        out.push(s..e);
                    }
                    start = None;
                }
            }
        }
        i += 1;
    }
    out
}

/// String literals (content ranges) inside `range` in order of appearance.
pub fn string_literals(m: &[u8], range: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = range.start;
    while i < range.end {
        if m[i] == b'"' || m[i] == b'\'' {
            let q = m[i];
            let triple = m.get(i + 1) == Some(&q) && m.get(i + 2) == Some(&q);
            let skip = if triple { 3 } else { 1 };
            let start = i + skip;
            let mut j = start;
            while j < range.end.max(m.len()).min(m.len()) {
                if triple {
                    if m[j] == q && m.get(j + 1) == Some(&q) && m.get(j + 2) == Some(&q) {
                        break;
                    }
                } else if m[j] == q {
                    break;
                }
                j += 1;
            }
            out.push(start..j.min(m.len()));
            i = j + skip;
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Scan results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lit {
    pub value: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GSource {
    Gav {
        coord: Coord,
        version: Option<Lit>,
    },
    /// `libs.some.alias` (chain without the leading `libs.`).
    Catalog {
        chain: String,
    },
}

#[derive(Debug, Clone)]
pub struct GDep {
    pub config: String,
    pub stmt: Range<usize>,
    pub source: GSource,
    pub platform: bool,
    pub block: usize,
    pub in_buildscript: bool,
    pub quote: Option<char>,
    pub parens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GPluginSource {
    Id { id: String, version: Option<Lit> },
    Catalog { chain: String },
}

#[derive(Debug, Clone)]
pub struct GPlugin {
    pub stmt: Range<usize>,
    pub source: GPluginSource,
}

#[derive(Debug, Clone, Default)]
pub struct GradleScan {
    pub kotlin: bool,
    pub deps: Vec<GDep>,
    pub plugins: Vec<GPlugin>,
    pub repos: Vec<String>,
    /// Simple string variables (`val x = "1.0"`, `def x = '1.0'`, `ext.x = …`).
    pub vars: BTreeMap<String, Lit>,
    pub blocks: Vec<Block>,
    /// Indices of `dependencies` blocks (outside `buildscript`).
    pub dep_blocks: Vec<usize>,
    /// Indices of `repositories` blocks (outside `buildscript`/`pluginManagement`).
    pub repo_blocks: Vec<usize>,
    /// All `libs.…` accessor chains used (without the leading `libs.`).
    pub catalog_refs: Vec<String>,
    /// Every statement of every `dependencies` block with its leading identifier
    /// (even those we cannot interpret, e.g. `project(":x")`); used to place new lines.
    pub all_stmts: Vec<(usize, String, Range<usize>)>,
}

pub fn is_kotlin_script(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("kts"))
}

fn skip_ws(m: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && m[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn read_ident(m: &[u8], mut i: usize, end: usize, allow_dot: bool) -> usize {
    while i < end && (is_ident(m[i]) || (allow_dot && m[i] == b'.')) {
        i += 1;
    }
    i
}

fn gav_from_literal(src: &str, lit: Range<usize>) -> Option<(Coord, Option<Lit>)> {
    let content = &src[lit.clone()];
    if content.contains(['\n', ' ', '/']) {
        return None;
    }
    let mut parts = content.split(':');
    let g = parts.next()?;
    let a = parts.next()?;
    if g.is_empty() || a.is_empty() || g.contains('$') || a.contains('$') {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    let version = rest.first().and_then(|v| {
        let v_end = v.find('@').unwrap_or(v.len());
        let v = &v[..v_end];
        if v.is_empty() {
            return None;
        }
        let offset = lit.start + g.len() + 1 + a.len() + 1;
        Some(Lit { value: v.to_string(), range: offset..offset + v.len() })
    });
    Some((Coord::new(g, a), version))
}

fn named_arg(m: &[u8], src: &str, range: Range<usize>, key: &str) -> Option<Lit> {
    let text = &m[range.clone()];
    let k = key.as_bytes();
    let mut i = 0;
    while i + k.len() <= text.len() {
        if &text[i..i + k.len()] == k
            && (i == 0 || !is_ident(text[i - 1]))
            && text.get(i + k.len()).is_none_or(|c| !is_ident(*c))
        {
            let mut j = i + k.len();
            while j < text.len() && text[j] == b' ' {
                j += 1;
            }
            if j < text.len() && (text[j] == b'=' || text[j] == b':') {
                j += 1;
                while j < text.len() && text[j] == b' ' {
                    j += 1;
                }
                if j < text.len() && (text[j] == b'"' || text[j] == b'\'') {
                    let abs = range.start + j;
                    if let Some(l) = string_literals(m, abs..range.end).into_iter().next() {
                        return Some(Lit { value: src[l.clone()].to_string(), range: l });
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Parses the argument part of a dependency statement.
fn parse_args(src: &str, m: &[u8], args: Range<usize>, platform: bool) -> Option<(GSource, bool, Option<char>)> {
    let s = skip_ws(m, args.start, args.end);
    if s >= args.end {
        return None;
    }
    if m[s] == b'"' || m[s] == b'\'' {
        let lit = string_literals(m, s..args.end).into_iter().next()?;
        let (coord, version) = gav_from_literal(src, lit)?;
        return Some((GSource::Gav { coord, version }, platform, Some(m[s] as char)));
    }
    let id_end = read_ident(m, s, args.end, true);
    let word = &src[s..id_end];
    if id_end < args.end && m[skip_ws(m, id_end, args.end)] == b'(' && !word.is_empty() {
        let open = skip_ws(m, id_end, args.end);
        let close = matching_close(m, open)?;
        return match word {
            "platform" | "enforcedPlatform" => parse_args(src, m, open + 1..close, true),
            "testFixtures" | "variantOf" => parse_args(src, m, open + 1..close, platform),
            _ => None,
        };
    }
    if let Some(chain) = word.strip_prefix("libs.") {
        return Some((GSource::Catalog { chain: chain.to_string() }, platform, None));
    }
    if word == "group" || word == "name" || word == "version" {
        let group = named_arg(m, src, args.clone(), "group")?;
        let name = named_arg(m, src, args.clone(), "name")?;
        if group.value.contains('$') || name.value.contains('$') {
            return None;
        }
        let version = named_arg(m, src, args.clone(), "version");
        return Some((GSource::Gav { coord: Coord::new(group.value, name.value), version }, platform, Some('"')));
    }
    None
}

fn parse_dep_statement(src: &str, m: &[u8], stmt: Range<usize>, block: usize, in_buildscript: bool) -> Option<GDep> {
    let id_end = read_ident(m, stmt.start, stmt.end, false);
    if id_end == stmt.start {
        return None;
    }
    // Dotted receivers (`paperweight.paperDevBundle(...)`) are not configurations.
    if m.get(id_end) == Some(&b'.') {
        return None;
    }
    let config = src[stmt.start..id_end].to_string();
    let j = skip_ws(m, id_end, stmt.end);
    let (args, parens) = if j < stmt.end && m[j] == b'(' {
        let close = matching_close(m, j)?;
        (j + 1..close, true)
    } else {
        (j..stmt.end, false)
    };
    let (source, platform, quote) = parse_args(src, m, args, false)?;
    Some(GDep { config, stmt, source, platform, block, in_buildscript, quote, parens })
}

fn version_after(src: &str, m: &[u8], from: usize, end: usize) -> Option<Lit> {
    // Find `version` keyword followed by a string literal.
    let mut i = from;
    while i + 7 <= end {
        if &m[i..i + 7] == b"version" && (i == 0 || !is_ident(m[i - 1])) && m.get(i + 7).is_none_or(|c| !is_ident(*c)) {
            let j = skip_ws(m, i + 7, end);
            if j < end && (m[j] == b'"' || m[j] == b'\'') {
                let l = string_literals(m, j..end).into_iter().next()?;
                return Some(Lit { value: src[l.clone()].to_string(), range: l });
            }
        }
        i += 1;
    }
    None
}

fn parse_plugin_statement(src: &str, m: &[u8], stmt: Range<usize>) -> Option<GPlugin> {
    let id_end = read_ident(m, stmt.start, stmt.end, false);
    let word = &src[stmt.start..id_end];
    let j = skip_ws(m, id_end, stmt.end);
    match word {
        "id" | "kotlin" => {
            let (arg_start, after) = if j < stmt.end && m[j] == b'(' {
                let close = matching_close(m, j)?;
                (j + 1, close + 1)
            } else {
                (j, stmt.end)
            };
            let lit = string_literals(m, arg_start..stmt.end).into_iter().next()?;
            let raw = src[lit.clone()].to_string();
            let id = if word == "kotlin" { format!("org.jetbrains.kotlin.{raw}") } else { raw };
            let after = if j < stmt.end && m[j] == b'(' { after } else { lit.end + 1 };
            let version = version_after(src, m, after.min(stmt.end), stmt.end);
            Some(GPlugin { stmt, source: GPluginSource::Id { id, version } })
        }
        "alias" => {
            let open = j;
            if open < stmt.end && m[open] == b'(' {
                let close = matching_close(m, open)?;
                let inner = src[open + 1..close].trim();
                let chain = inner.strip_prefix("libs.plugins.")?;
                return Some(GPlugin { stmt, source: GPluginSource::Catalog { chain: chain.to_string() } });
            }
            None
        }
        _ => None,
    }
}

fn repo_urls(src: &str, m: &[u8], stmt: Range<usize>) -> Vec<String> {
    let id_end = read_ident(m, stmt.start, stmt.end, false);
    match &src[stmt.start..id_end] {
        "mavenCentral" => vec![crate::registry::DEFAULT_CENTRAL.to_string()],
        "google" => vec!["https://maven.google.com".to_string()],
        "gradlePluginPortal" => vec![crate::registry::DEFAULT_PLUGIN_PORTAL.to_string()],
        "maven" => string_literals(m, stmt.clone())
            .into_iter()
            .map(|l| src[l].to_string())
            .find(|s| s.starts_with("http"))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn collect_vars(src: &str, m: &[u8], blocks: &[Block], out: &mut BTreeMap<String, Lit>) {
    // Scan every statement of the file (top level and nested `ext {}`) line by line.
    let all = statements(m, 0, m.len());
    let mut stmts = all;
    // Statements inside `ext { … }` blocks.
    for b in blocks.iter().filter(|b| b.name == "ext" || b.name == "extra") {
        stmts.extend(statements(m, b.open + 1, b.close));
    }
    for st in stmts {
        let text = &src[st.clone()];
        let mt = &m[st.clone()];
        let mut t = text;
        let mut off = 0usize;
        for prefix in ["const val ", "val ", "var ", "def ", "final String ", "String ", "ext."] {
            if let Some(rest) = t.strip_prefix(prefix) {
                off += prefix.len();
                t = rest;
                break;
            }
        }
        let name_end = t.bytes().take_while(|c| is_ident(*c)).count();
        if name_end == 0 {
            continue;
        }
        let name = &t[..name_end];
        let mut k = off + name_end;
        // optional `: String`
        let after = skip_ws(mt, k, mt.len());
        k = after;
        if k < mt.len() && mt[k] == b':' {
            while k < mt.len() && mt[k] != b'=' {
                k += 1;
            }
        }
        if k >= mt.len() || mt[k] != b'=' {
            continue;
        }
        k = skip_ws(mt, k + 1, mt.len());
        if k < mt.len() && (mt[k] == b'"' || mt[k] == b'\'') {
            if let Some(l) = string_literals(m, st.start + k..st.end).into_iter().next() {
                if l.end + 1 == st.end || m[l.end + 1..st.end].iter().all(|c| c.is_ascii_whitespace()) {
                    out.insert(name.to_string(), Lit { value: src[l.clone()].to_string(), range: l });
                }
            }
        }
    }
}

fn collect_catalog_refs(src: &str, m: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 < m.len() {
        if &m[i..i + 5] == b"libs." && (i == 0 || !(is_ident(m[i - 1]) || m[i - 1] == b'.')) {
            let end = read_ident(m, i + 5, m.len(), true);
            out.push(src[i + 5..end].trim_end_matches('.').to_string());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

pub fn scan(src: &str, kotlin: bool) -> GradleScan {
    let m = mask(src);
    let blocks = blocks(&m);
    let mut scan = GradleScan { kotlin, blocks: blocks.clone(), ..Default::default() };
    for (idx, b) in blocks.iter().enumerate() {
        let in_buildscript = has_ancestor(&blocks, b.parent, "buildscript") || b.name == "buildscript";
        match b.name.as_str() {
            "dependencies" => {
                // Only direct `dependencies {}`; ignore nested constraints etc.
                if !in_buildscript {
                    scan.dep_blocks.push(idx);
                }
                // Skip `dependencies` blocks that belong to `pluginManagement` (none expected).
                for st in statements(&m, b.open + 1, b.close) {
                    let id_end = read_ident(&m, st.start, st.end, false);
                    if id_end > st.start && m.get(id_end) != Some(&b'.') {
                        scan.all_stmts.push((idx, src[st.start..id_end].to_string(), st.clone()));
                    }
                    if let Some(d) = parse_dep_statement(src, &m, st, idx, in_buildscript) {
                        scan.deps.push(d);
                    }
                }
            }
            "plugins" => {
                for st in statements(&m, b.open + 1, b.close) {
                    if let Some(p) = parse_plugin_statement(src, &m, st) {
                        scan.plugins.push(p);
                    }
                }
            }
            "repositories" => {
                if !in_buildscript && !has_ancestor(&blocks, b.parent, "pluginManagement") {
                    scan.repo_blocks.push(idx);
                }
                for st in statements(&m, b.open + 1, b.close) {
                    scan.repos.extend(repo_urls(src, &m, st));
                }
            }
            _ => {}
        }
    }
    collect_vars(src, &m, &blocks, &mut scan.vars);
    scan.catalog_refs = collect_catalog_refs(src, &m);
    scan
}

/// Resolves `$name` / `${name}` inside a version literal.
pub fn var_reference(value: &str) -> Option<&str> {
    let v = value.trim();
    if let Some(r) = v.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        return Some(r.trim());
    }
    let r = v.strip_prefix('$')?;
    r.bytes().all(is_ident).then_some(r)
}

/// Parses `gradle.properties`: key -> (value, value range).
pub fn parse_properties(src: &str) -> BTreeMap<String, Lit> {
    let mut out = BTreeMap::new();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('!') {
            if let Some(eq) = line.find(['=', ':']) {
                let key = line[..eq].trim();
                let raw = &line[eq + 1..];
                let lead = raw.len() - raw.trim_start().len();
                let val = raw.trim();
                if !key.is_empty() && !val.is_empty() && !line.trim_end().ends_with('\\') {
                    let start = offset + eq + 1 + lead;
                    out.insert(key.to_string(), Lit { value: val.to_string(), range: start..start + val.len() });
                }
            }
        }
        offset += line.len();
    }
    out
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

fn unit_indent(src: &str) -> String {
    for line in src.lines() {
        if line.starts_with('\t') {
            return "\t".into();
        }
        let n = line.len() - line.trim_start_matches(' ').len();
        if n > 0 && !line.trim().is_empty() {
            return " ".repeat(n.min(4).max(2));
        }
    }
    "    ".into()
}

/// Chooses the quote style for new Groovy strings from existing dependency literals.
fn dominant_quote(scan: &GradleScan) -> char {
    let (mut single, mut double) = (0, 0);
    for d in &scan.deps {
        match d.quote {
            Some('\'') => single += 1,
            Some('"') => double += 1,
            _ => {}
        }
    }
    if scan.kotlin || double >= single && double > 0 { '"' } else { '\'' }
}

/// Builds the statement text for a dependency.
pub fn render_statement(scan: &GradleScan, config: &str, expr_catalog: Option<&str>, gav: Option<&str>) -> String {
    let arg = match (expr_catalog, gav) {
        (Some(c), _) => format!("libs.{c}"),
        (None, Some(g)) => {
            let q = dominant_quote(scan);
            format!("{q}{g}{q}")
        }
        _ => String::new(),
    };
    let parens = scan.kotlin || scan.deps.iter().filter(|d| !d.in_buildscript).any(|d| d.parens);
    if parens { format!("{config}({arg})") } else { format!("{config} {arg}") }
}

fn is_test_config(c: &str) -> bool {
    c.starts_with("test") || c.starts_with("integrationTest")
}

/// Inserts a dependency statement into the project's `dependencies {}` block,
/// creating the block when necessary.
pub fn add_statement(
    src: &str,
    kotlin: bool,
    config: &str,
    expr_catalog: Option<&str>,
    gav: Option<&str>,
) -> Result<String> {
    let scan = scan(src, kotlin);
    let m = mask(src);
    let eol = LineEnding::detect(src);
    let nl = eol.as_str();
    let unit = unit_indent(src);
    let stmt_text = render_statement(&scan, config, expr_catalog, gav);

    // Prefer a top-level `dependencies` block (parent None), else the first one.
    let block_idx = scan
        .dep_blocks
        .iter()
        .copied()
        .find(|&i| scan.blocks[i].parent.is_none())
        .or_else(|| scan.dep_blocks.first().copied());

    let mut out = src.to_string();
    let Some(bi) = block_idx else {
        if !out.ends_with('\n') && !out.is_empty() {
            out.push_str(nl);
        }
        if !out.is_empty() {
            out.push_str(nl);
        }
        out.push_str(&format!("dependencies {{{nl}{unit}{stmt_text}{nl}}}{nl}"));
        return Ok(out);
    };
    let block = &scan.blocks[bi];
    let in_block: Vec<&(usize, String, Range<usize>)> = scan.all_stmts.iter().filter(|(b, _, _)| *b == bi).collect();

    // Same configuration → after its last statement; same family → after last of family.
    let anchor = in_block
        .iter()
        .rev()
        .find(|(_, c, _)| c == config)
        .or_else(|| in_block.iter().rev().find(|(_, c, _)| is_test_config(c) == is_test_config(config)))
        .or_else(|| in_block.last());

    if let Some((_, _, range)) = anchor {
        let indent = line_indent(src, range.start).to_string();
        let at = line_end_inclusive(src, range.end.saturating_sub(1));
        // The statement may be followed by `}` on the same line (one-liner blocks).
        if m[range.end..at].iter().any(|c| !c.is_ascii_whitespace()) {
            bail_single_line()?;
        }
        let text = if src[..at].ends_with('\n') {
            format!("{indent}{stmt_text}{nl}")
        } else {
            format!("{nl}{indent}{stmt_text}")
        };
        out.insert_str(at, &text);
        return Ok(out);
    }

    // Empty block.
    let inner = &src[block.open + 1..block.close];
    let base_indent = line_indent(src, block.open).to_string();
    if inner.contains('\n') {
        let at = line_start(src, block.close);
        out.insert_str(at, &format!("{base_indent}{unit}{stmt_text}{nl}"));
    } else {
        out.replace_range(block.open + 1..block.close, &format!("{nl}{base_indent}{unit}{stmt_text}{nl}{base_indent}"));
    }
    Ok(out)
}

fn bail_single_line() -> Result<()> {
    bail!(
        "cannot edit a one-line `dependencies {{ … }}` block with existing entries; please put each dependency on its own line"
    )
}

/// Removes whole statements (and their lines when they stand alone).
pub fn remove_statements(src: &str, ranges: &[Range<usize>]) -> String {
    let mut sorted: Vec<Range<usize>> = ranges.to_vec();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut out = src.to_string();
    for r in sorted {
        let ls = line_start(&out, r.start);
        let le = line_end_inclusive(&out, r.end.saturating_sub(1).max(r.start));
        let before = out[ls..r.start].trim().is_empty();
        let after = out[r.end..le].trim().is_empty();
        if before && after {
            out.replace_range(ls..le, "");
        } else {
            out.replace_range(r.clone(), "");
        }
    }
    out
}

/// Adds `maven("url")` to the first project-level `repositories {}` block,
/// creating the block (above `dependencies`) when needed. Returns `None` when
/// the repository is already declared.
pub fn ensure_repository(src: &str, kotlin: bool, url: &str) -> Result<Option<String>> {
    let scan = scan(src, kotlin);
    let norm = |s: &str| s.trim_end_matches('/').to_ascii_lowercase();
    if scan.repos.iter().any(|r| norm(r) == norm(url)) {
        return Ok(None);
    }
    let nl = LineEnding::detect(src).as_str();
    let unit = unit_indent(src);
    let line = if kotlin { format!("maven(\"{url}\")") } else { format!("maven {{ url '{url}' }}") };
    let mut out = src.to_string();
    let block = scan
        .repo_blocks
        .iter()
        .copied()
        .find(|&i| scan.blocks[i].parent.is_none())
        .or_else(|| scan.repo_blocks.first().copied());
    if let Some(bi) = block {
        let b = &scan.blocks[bi];
        let base = line_indent(src, b.open).to_string();
        let inner = &src[b.open + 1..b.close];
        if inner.contains('\n') {
            let at = line_start(src, b.close);
            out.insert_str(at, &format!("{base}{unit}{line}{nl}"));
        } else {
            out.replace_range(
                b.open + 1..b.close,
                &format!("{nl}{base}{unit}{inner_trim}{nl}{base}{unit}{line}{nl}{base}", inner_trim = inner.trim()),
            );
        }
        return Ok(Some(out));
    }
    // No block: create one before `dependencies {` (or at the end).
    let at = scan
        .dep_blocks
        .iter()
        .copied()
        .find(|&i| scan.blocks[i].parent.is_none())
        .map(|i| line_start(src, scan.blocks[i].open))
        .unwrap_or(src.len());
    let mut section = format!("repositories {{{nl}{unit}mavenCentral(){nl}{unit}{line}{nl}}}{nl}{nl}");
    if at == src.len() && !src.is_empty() && !src.ends_with('\n') {
        section = format!("{nl}{nl}{section}");
    }
    out.insert_str(at, &section);
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KTS: &str = r#"plugins {
    java
    kotlin("jvm") version "2.0.0"
    id("io.papermc.paperweight.userdev") version "1.7.1" // paper
    alias(libs.plugins.shadow)
}

val hikariVersion = "5.1.0"

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
    maven { url = uri("https://jitpack.io") }
}

dependencies {
    // database
    implementation("com.zaxxer:HikariCP:$hikariVersion")
    implementation("org.postgresql:postgresql:42.7.3") {
        exclude(group = "x", module = "y")
    }
    implementation(libs.guava)
    compileOnly(group = "org.slf4j", name = "slf4j-api", version = "2.0.9")
    implementation(platform("org.springframework.boot:spring-boot-dependencies:3.3.0"))
    testImplementation(kotlin("test"))
    testImplementation(project(":core"))
    paperweight.paperDevBundle("1.21-R0.1-SNAPSHOT")
}
"#;

    const GROOVY: &str = r#"plugins {
    id 'java'
    id 'org.springframework.boot' version '3.2.0'
}

ext {
    gsonVersion = '2.10.1'
}

repositories {
    mavenCentral()
}

dependencies {
    implementation 'com.google.code.gson:gson:2.10.1'
    implementation "org.apache.commons:commons-lang3:3.14.0"
    testImplementation group: 'junit', name: 'junit', version: '4.13.2'
    runtimeOnly libs.postgres
}
"#;

    #[test]
    fn masks_comments_and_strings() {
        let src = "a // \"x\"\nb(\"c:d\") /* } */ e";
        let m = mask(src);
        assert_eq!(m.len(), src.len());
        let s = String::from_utf8(m).unwrap();
        assert!(!s.contains("x") && !s.contains("c:d") && !s.contains('}'));
        assert!(s.contains("b(\"   \")"), "{s}");
    }

    #[test]
    fn scans_kotlin_dependencies() {
        let s = scan(KTS, true);
        let gavs: Vec<_> = s
            .deps
            .iter()
            .map(|d| match &d.source {
                GSource::Gav { coord, version } => {
                    format!("{} {}@{}", d.config, coord, version.as_ref().map(|v| v.value.as_str()).unwrap_or("-"))
                }
                GSource::Catalog { chain } => format!("{} libs.{chain}", d.config),
            })
            .collect();
        assert_eq!(
            gavs,
            vec![
                "implementation com.zaxxer:HikariCP@$hikariVersion",
                "implementation org.postgresql:postgresql@42.7.3",
                "implementation libs.guava",
                "compileOnly org.slf4j:slf4j-api@2.0.9",
                "implementation org.springframework.boot:spring-boot-dependencies@3.3.0",
            ]
        );
        assert!(s.deps[4].platform);
        assert_eq!(s.vars["hikariVersion"].value, "5.1.0");
        assert_eq!(&KTS[s.vars["hikariVersion"].range.clone()], "5.1.0");
    }

    #[test]
    fn scans_plugins_and_repos() {
        let s = scan(KTS, true);
        let ids: Vec<_> = s
            .plugins
            .iter()
            .map(|p| match &p.source {
                GPluginSource::Id { id, version } => {
                    format!("{id}@{}", version.as_ref().map(|v| v.value.clone()).unwrap_or_default())
                }
                GPluginSource::Catalog { chain } => format!("libs.plugins.{chain}"),
            })
            .collect();
        assert_eq!(
            ids,
            vec!["org.jetbrains.kotlin.jvm@2.0.0", "io.papermc.paperweight.userdev@1.7.1", "libs.plugins.shadow"]
        );
        assert_eq!(s.repos.len(), 3);
        assert!(s.repos.contains(&"https://jitpack.io".to_string()));
        assert!(s.catalog_refs.contains(&"guava".to_string()));
        assert!(s.catalog_refs.contains(&"plugins.shadow".to_string()));
    }

    #[test]
    fn scans_groovy() {
        let s = scan(GROOVY, false);
        assert_eq!(s.deps.len(), 4);
        match &s.deps[2].source {
            GSource::Gav { coord, version } => {
                assert_eq!(coord, &Coord::new("junit", "junit"));
                assert_eq!(version.as_ref().unwrap().value, "4.13.2");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(s.deps[3].source, GSource::Catalog { .. }));
        assert_eq!(s.vars["gsonVersion"].value, "2.10.1");
        let p = match &s.plugins[1].source {
            GPluginSource::Id { id, version } => (id.clone(), version.clone().unwrap().value),
            _ => panic!(),
        };
        assert_eq!(p, ("org.springframework.boot".into(), "3.2.0".into()));
    }

    #[test]
    fn version_ranges_point_at_the_version_only() {
        let s = scan(KTS, true);
        if let GSource::Gav { version: Some(v), .. } = &s.deps[1].source {
            assert_eq!(&KTS[v.range.clone()], "42.7.3");
        } else {
            panic!()
        }
    }

    #[test]
    fn adds_after_same_config() {
        let out = add_statement(KTS, true, "implementation", None, Some("redis.clients:jedis:5.1.0")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        let idx = lines.iter().position(|l| l.contains("jedis")).unwrap();
        assert_eq!(lines[idx], "    implementation(\"redis.clients:jedis:5.1.0\")");
        assert!(lines[idx - 1].contains("platform"), "{}", lines[idx - 1]);
    }

    #[test]
    fn adds_test_scope_near_tests() {
        let out = add_statement(KTS, true, "testImplementation", Some("mockk"), None).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        let idx = lines.iter().position(|l| l.contains("libs.mockk")).unwrap();
        assert_eq!(lines[idx], "    testImplementation(libs.mockk)");
        assert!(
            lines[idx - 1].contains("project(\":core\")")
                || lines[idx - 1].contains("kotlin(\"test\")")
                || lines[idx - 1].contains("testImplementation")
        );
    }

    #[test]
    fn adds_groovy_in_existing_style() {
        let out = add_statement(GROOVY, false, "implementation", None, Some("redis.clients:jedis:5.1.0")).unwrap();
        assert!(out.contains("    implementation \"redis.clients:jedis:5.1.0\"\n"), "{out}");
        let out = add_statement(GROOVY, false, "compileOnly", None, Some("a:b:1")).unwrap();
        assert!(out.contains("    compileOnly \"a:b:1\"\n"), "{out}");
    }

    #[test]
    fn creates_dependencies_block() {
        let src = "plugins {\n    java\n}\n";
        let out = add_statement(src, true, "implementation", None, Some("a:b:1")).unwrap();
        assert_eq!(out, "plugins {\n    java\n}\n\ndependencies {\n    implementation(\"a:b:1\")\n}\n");
    }

    #[test]
    fn fills_empty_block() {
        let out = add_statement("dependencies {\n}\n", true, "implementation", None, Some("a:b:1")).unwrap();
        assert_eq!(out, "dependencies {\n    implementation(\"a:b:1\")\n}\n");
        let out = add_statement("dependencies {}\n", true, "implementation", None, Some("a:b:1")).unwrap();
        assert_eq!(out, "dependencies {\n    implementation(\"a:b:1\")\n}\n");
    }

    #[test]
    fn crlf_is_kept() {
        let src = KTS.replace('\n', "\r\n");
        let out = add_statement(&src, true, "implementation", None, Some("a:b:1")).unwrap();
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
        assert!(out.contains("implementation(\"a:b:1\")\r\n"));
    }

    #[test]
    fn removes_statements_with_blocks() {
        let s = scan(KTS, true);
        let pg = s.deps[1].stmt.clone();
        let out = remove_statements(KTS, &[pg]);
        assert!(!out.contains("postgresql"));
        assert!(!out.contains("exclude("));
        assert!(out.contains("implementation(libs.guava)"));
    }

    #[test]
    fn repository_handling() {
        assert!(ensure_repository(KTS, true, "https://repo.papermc.io/repository/maven-public").unwrap().is_none());
        let out = ensure_repository(KTS, true, "https://example.org/m").unwrap().unwrap();
        assert!(out.contains("    maven(\"https://example.org/m\")\n}"), "{out}");
        let out = ensure_repository("dependencies {\n    api(\"a:b:1\")\n}\n", true, "https://x.y/z").unwrap().unwrap();
        assert!(
            out.starts_with("repositories {\n    mavenCentral()\n    maven(\"https://x.y/z\")\n}\n\ndependencies"),
            "{out}"
        );
    }

    #[test]
    fn properties_file() {
        let p = parse_properties("# c\norg.gradle.jvmargs=-Xmx2g\nkotlinVersion = 2.0.0\n");
        assert_eq!(p["kotlinVersion"].value, "2.0.0");
        let src = "# c\norg.gradle.jvmargs=-Xmx2g\nkotlinVersion = 2.0.0\n";
        assert_eq!(&src[p["kotlinVersion"].range.clone()], "2.0.0");
    }

    #[test]
    fn var_references() {
        assert_eq!(var_reference("$foo"), Some("foo"));
        assert_eq!(var_reference("${foo}"), Some("foo"));
        assert_eq!(var_reference("1.0"), None);
        assert_eq!(var_reference("$foo.bar"), None);
    }
}
