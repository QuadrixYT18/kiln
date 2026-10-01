//! A tiny template language shared by built-in and custom templates.
//!
//! * `{{name}}` – substitutes a variable (identifier characters only, so GitHub
//!   Actions' `${{ matrix.os }}` is left untouched).
//! * `{{latest:group:artifact?pre|before:-R}}`, `{{plugin:id}}` – live version
//!   lookups, resolved by [`super::tokens`].
//! * `{{#if name}}…{{#else}}…{{/if}}` and `{{#unless name}}…{{/unless}}` on their
//!   own lines.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};

pub type Vars = BTreeMap<String, String>;

pub fn truthy(v: Option<&String>) -> bool {
    matches!(v, Some(s) if !s.is_empty() && s != "false" && s != "0")
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_token(s: &str) -> bool {
    (s.starts_with("latest:") || s.starts_with("plugin:"))
        && s.len() > 7
        && !s.contains(|c: char| c.is_whitespace() || c == '{' || c == '}')
}

/// Placeholder kinds found in a template.
#[derive(Debug, Default, Clone)]
pub struct Found {
    pub variables: BTreeSet<String>,
    pub flags: BTreeSet<String>,
    pub tokens: BTreeSet<String>,
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
    Token(&'a str),
}

fn split(line: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start + 2..].find("}}") else { break };
        let inner = &rest[start + 2..start + 2 + end];
        if is_ident(inner) {
            if start > 0 {
                out.push(Piece::Text(&rest[..start]));
            }
            out.push(Piece::Var(inner));
            rest = &rest[start + 2 + end + 2..];
        } else if is_token(inner) {
            if start > 0 {
                out.push(Piece::Text(&rest[..start]));
            }
            out.push(Piece::Token(inner));
            rest = &rest[start + 2 + end + 2..];
        } else {
            out.push(Piece::Text(&rest[..start + 2]));
            rest = &rest[start + 2..];
        }
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    out
}

enum Directive<'a> {
    If(&'a str),
    Unless(&'a str),
    Else,
    End,
}

fn directive(line: &str) -> Option<Directive<'_>> {
    let t = line.trim();
    let inner = t.strip_prefix("{{")?.strip_suffix("}}")?;
    if let Some(n) = inner.strip_prefix("#if ") {
        return is_ident(n.trim()).then(|| Directive::If(n.trim()));
    }
    if let Some(n) = inner.strip_prefix("#unless ") {
        return is_ident(n.trim()).then(|| Directive::Unless(n.trim()));
    }
    match inner.trim() {
        "#else" => Some(Directive::Else),
        "/if" | "/unless" => Some(Directive::End),
        _ => None,
    }
}

/// Scans text for placeholders without rendering it.
pub fn scan(text: &str) -> Found {
    let mut f = Found::default();
    for line in text.lines() {
        match directive(line) {
            Some(Directive::If(n)) | Some(Directive::Unless(n)) => {
                f.flags.insert(n.to_string());
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        for p in split(line) {
            match p {
                Piece::Var(v) => {
                    f.variables.insert(v.to_string());
                }
                Piece::Token(t) => {
                    f.tokens.insert(t.to_string());
                }
                Piece::Text(_) => {}
            }
        }
    }
    f
}

/// Renders `text`. `tokens` maps token bodies to their resolved value; when it is
/// `None`, tokens are kept verbatim (first pass used to collect them).
pub fn render(text: &str, vars: &Vars, tokens: Option<&BTreeMap<String, String>>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    // (active, parent_active, seen_else)
    let mut stack: Vec<(bool, bool, bool)> = Vec::new();
    for line in text.split_inclusive('\n') {
        let active_now = stack.last().map(|s| s.0).unwrap_or(true);
        match directive(line) {
            Some(Directive::If(n)) => {
                let cond = truthy(vars.get(n));
                stack.push((active_now && cond, active_now, false));
                continue;
            }
            Some(Directive::Unless(n)) => {
                let cond = !truthy(vars.get(n));
                stack.push((active_now && cond, active_now, false));
                continue;
            }
            Some(Directive::Else) => {
                let Some(top) = stack.last_mut() else { bail!("`{{{{#else}}}}` without `{{{{#if}}}}`") };
                // Recompute: else branch is active iff parent active and the `if` branch was not.
                let was_true = top.0 && !top.2;
                top.0 = top.1 && !was_true;
                top.2 = true;
                continue;
            }
            Some(Directive::End) => {
                if stack.pop().is_none() {
                    bail!("unmatched `{{{{/if}}}}`");
                }
                continue;
            }
            None => {}
        }
        if !active_now {
            continue;
        }
        for p in split(line) {
            match p {
                Piece::Text(t) => out.push_str(t),
                Piece::Var(v) => match vars.get(v) {
                    Some(val) => out.push_str(val),
                    None => bail!("unknown placeholder `{{{{{v}}}}}`"),
                },
                Piece::Token(t) => match tokens {
                    Some(map) => match map.get(t) {
                        Some(val) => out.push_str(val),
                        None => bail!("unresolved version lookup `{{{{{t}}}}}`"),
                    },
                    None => {
                        out.push_str("{{");
                        out.push_str(t);
                        out.push_str("}}");
                    }
                },
            }
        }
    }
    if !stack.is_empty() {
        bail!("`{{{{#if}}}}` without matching `{{{{/if}}}}`");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn substitutes_variables() {
        let v = vars(&[("name", "demo"), ("n", "1")]);
        assert_eq!(render("hi {{name}}-{{n}}\n", &v, None).unwrap(), "hi demo-1\n");
    }

    #[test]
    fn leaves_github_actions_expressions_alone() {
        let v = Vars::new();
        let s = "os: ${{ matrix.os }}\nx: ${{secrets.TOKEN}}\n";
        // `{{secrets.TOKEN}}` has a dot and is therefore not a placeholder either.
        assert_eq!(render(s, &v, None).unwrap(), s);
    }

    #[test]
    fn conditionals() {
        let t = "a\n{{#if x}}\nb\n{{#else}}\nc\n{{/if}}\nd\n";
        assert_eq!(render(t, &vars(&[("x", "true")]), None).unwrap(), "a\nb\nd\n");
        assert_eq!(render(t, &vars(&[("x", "false")]), None).unwrap(), "a\nc\nd\n");
        assert_eq!(render(t, &Vars::new(), None).unwrap(), "a\nc\nd\n");
        let u = "{{#unless x}}\nonly\n{{/unless}}\n";
        assert_eq!(render(u, &Vars::new(), None).unwrap(), "only\n");
    }

    #[test]
    fn nested_conditionals() {
        let t = "{{#if a}}\n1\n{{#if b}}\n2\n{{#else}}\n3\n{{/if}}\n4\n{{#else}}\n5\n{{/if}}\n";
        assert_eq!(render(t, &vars(&[("a", "true"), ("b", "true")]), None).unwrap(), "1\n2\n4\n");
        assert_eq!(render(t, &vars(&[("a", "true")]), None).unwrap(), "1\n3\n4\n");
        assert_eq!(render(t, &vars(&[("b", "true")]), None).unwrap(), "5\n");
    }

    #[test]
    fn unbalanced_is_an_error() {
        assert!(render("{{#if x}}\na\n", &Vars::new(), None).is_err());
        assert!(render("{{/if}}\n", &Vars::new(), None).is_err());
    }

    #[test]
    fn unknown_variable_is_an_error() {
        assert!(render("{{nope}}", &Vars::new(), None).is_err());
    }

    #[test]
    fn tokens_pass_through_then_resolve() {
        let t = "v = \"{{latest:a:b?pre|before:-R}}\"\n";
        let first = render(t, &Vars::new(), None).unwrap();
        assert_eq!(first, t);
        let found = scan(&first);
        assert!(found.tokens.contains("latest:a:b?pre|before:-R"));
        let mut map = BTreeMap::new();
        map.insert("latest:a:b?pre|before:-R".to_string(), "1.2.3".to_string());
        assert_eq!(render(t, &Vars::new(), Some(&map)).unwrap(), "v = \"1.2.3\"\n");
    }

    #[test]
    fn scan_collects_names() {
        let f = scan("{{#if flag}}\n{{name}} {{other}}\n{{/if}}\n");
        assert!(f.variables.contains("name") && f.variables.contains("other"));
        assert!(f.flags.contains("flag"));
        assert!(!f.variables.contains("flag"));
    }

    #[test]
    fn inactive_regions_do_not_render_variables() {
        let t = "{{#if x}}\n{{missing}}\n{{/if}}\nok\n";
        assert_eq!(render(t, &Vars::new(), None).unwrap(), "ok\n");
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use crate::util::fuzz::{Rng, mutate, rounds};

    #[test]
    fn rendering_never_panics_on_broken_templates() {
        let seed = "{{#if kotlin}}\nplugins { {{name}} }\n{{#else}}\nx {{latest:g:a?pre|before:-R}}\n{{/if}}\n{{#unless a}}\n{{b}}\n{{/unless}}\n";
        let snippets = &["{{", "}}", "{{#if x}}", "{{/if}}", "{{#else}}", "\n", "ü", "{", "}", "{{latest:", "|", "?"];
        let mut rng = Rng(42);
        let mut vars = Vars::new();
        vars.insert("name".into(), "n".into());
        for _ in 0..rounds() {
            let text = mutate(&mut rng, seed, snippets);
            let _ = scan(&text);
            let _ = render(&text, &vars, None);
            let _ = render(&text, &vars, Some(&BTreeMap::new()));
        }
    }
}
