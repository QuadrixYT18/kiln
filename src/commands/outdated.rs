use std::collections::BTreeMap;

use anyhow::Result;

use crate::check::{Checked, check_all};
use crate::cli::OutdatedArgs;
use crate::context::Ctx;
use crate::project::detect::detect;
use crate::project::scan::{conflicts, scan_project};
use crate::project::workspace::Workspace;
use crate::registry::version::{Bump, Stability};
use crate::util::term;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Current,
    Patch,
    Minor,
    Major,
    Error,
}

struct Row {
    name: String,
    current: String,
    patch: Option<String>,
    minor: Option<String>,
    major: Option<String>,
    modules: Vec<String>,
    state: State,
}

fn state_of(c: &Checked) -> State {
    if c.error.is_some() {
        State::Error
    } else if c.candidates.major.is_some() {
        State::Major
    } else if c.candidates.minor.is_some() {
        State::Minor
    } else if c.candidates.patch.is_some() {
        State::Patch
    } else {
        State::Current
    }
}

pub async fn run(ctx: &Ctx, args: OutdatedArgs) -> Result<()> {
    let project = detect(&ctx.cwd)?;
    let reg = ctx.registry()?;
    let mut ws = Workspace::new();
    let scan = scan_project(&project, &mut ws, &ctx.config.repositories)?;
    let stability = if args.pre { Stability::Any } else { Stability::Stable };

    let total = scan.deps.len();
    let checked = check_all(&reg, scan.deps.clone(), &scan.repos, stability, true).await;

    let mut rows: BTreeMap<(String, String), Row> = BTreeMap::new();
    let mut skipped = 0;
    for c in &checked {
        if !Checked::is_checkable(&c.dep) {
            skipped += 1;
            continue;
        }
        let key = (c.dep.display_name(), c.dep.current.clone().unwrap_or_default());
        let state = state_of(c);
        let row = rows.entry(key.clone()).or_insert_with(|| Row {
            name: key.0.clone(),
            current: key.1.clone(),
            patch: c.candidates.patch.clone(),
            minor: c.candidates.minor.clone(),
            major: c.candidates.major.clone(),
            modules: Vec::new(),
            state,
        });
        if !row.modules.contains(&c.dep.module) {
            row.modules.push(c.dep.module.clone());
        }
    }
    let mut rows: Vec<Row> = rows.into_values().collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    if args.only_outdated {
        rows.retain(|r| r.state != State::Current);
    }

    if rows.is_empty() {
        println!("{} everything is up to date", term::ok_mark());
    } else {
        print_table(&rows);
    }

    let outdated = rows.iter().filter(|r| matches!(r.state, State::Patch | State::Minor | State::Major)).count();
    let majors = rows.iter().filter(|r| r.state == State::Major).count();
    let errors: Vec<&Checked> = checked.iter().filter(|c| c.error.is_some()).collect();
    println!();
    println!("{} dependencies checked, {} with updates ({} major)", rows.len(), outdated, majors);
    if skipped > 0 {
        println!(
            "{}",
            term::dim(format!("{skipped} of {total} declarations skipped (version managed elsewhere or dynamic)"))
        );
    }
    for e in errors.iter().take(5) {
        eprintln!("{} {}: {}", term::warn_mark(), e.dep.display_name(), e.error.as_deref().unwrap_or(""));
    }
    if errors.len() > 5 {
        eprintln!("{} ... and {} more lookup errors", term::warn_mark(), errors.len() - 5);
    }
    let conf = conflicts(&scan.deps);
    for c in conf {
        let list = c.versions.iter().map(|(m, v)| format!("{v} ({m})")).collect::<Vec<_>>().join(", ");
        println!("{} {} is declared with different versions: {list}", term::warn_mark(), c.coord);
    }
    Ok(())
}

fn print_table(rows: &[Row]) {
    let headers = ["Dependency", "Current", "Patch", "Minor", "Major", "Status", "Where"];
    let dash = || "-".to_string();
    let cells: Vec<[String; 7]> = rows
        .iter()
        .map(|r| {
            let color = |s: String, st: State| match st {
                State::Current => term::green(s),
                State::Patch | State::Minor => term::yellow(s),
                State::Major => term::red(s),
                State::Error => term::dim(s),
            };
            let status = match r.state {
                State::Current => term::green("up to date"),
                State::Patch => term::yellow("patch"),
                State::Minor => term::yellow("minor"),
                State::Major => term::red("MAJOR"),
                State::Error => term::dim("lookup failed"),
            };
            [
                r.name.clone(),
                color(r.current.clone(), r.state),
                r.patch.clone().map(term::yellow).unwrap_or_else(|| term::dim(dash())),
                r.minor.clone().map(term::yellow).unwrap_or_else(|| term::dim(dash())),
                r.major.clone().map(term::red).unwrap_or_else(|| term::dim(dash())),
                status,
                term::dim(r.modules.join(", ")),
            ]
        })
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &cells {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(term::visible_width(c));
        }
    }
    let line = |cols: Vec<String>| {
        let mut s = String::new();
        for (i, c) in cols.iter().enumerate() {
            if i + 1 == cols.len() {
                s.push_str(c);
            } else {
                s.push_str(&term::pad(c, widths[i]));
                s.push_str("  ");
            }
        }
        s
    };
    println!("{}", line(headers.iter().map(term::bold).collect()));
    for row in cells {
        println!("{}", line(row.to_vec()));
    }
    let _ = Bump::Patch;
}
