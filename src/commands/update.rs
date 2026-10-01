use std::collections::BTreeMap;

use anyhow::{Result, bail};
use futures::future::join_all;

use super::finish;
use crate::check::{Checked, check_all, repos_for};
use crate::cli::UpdateArgs;
use crate::context::Ctx;
use crate::project::detect::detect;
use crate::project::edit::apply_updates;
use crate::project::model::{BuildKind, Project, VersionSite};
use crate::project::scan::{conflicts, scan_project};
use crate::project::workspace::Workspace;
use crate::registry::aliases;
use crate::registry::version::{Bump, Stability, Version};
use crate::registry::{Coord, Repo};
use crate::util::term;

#[derive(Clone)]
struct Proposal {
    checked: Checked,
    target: String,
    bump: Bump,
}

/// Picks the target version for one dependency under the chosen level.
fn target_for(c: &Checked, args: &UpdateArgs) -> Option<(String, Bump)> {
    let cand = &c.candidates;
    if args.patch {
        cand.patch.clone().map(|v| (v, Bump::Patch))
    } else if args.minor {
        cand.minor.clone().map(|v| (v, Bump::Minor)).or_else(|| cand.patch.clone().map(|v| (v, Bump::Patch)))
    } else {
        cand.highest().map(|(v, b)| (v.to_string(), b))
    }
}

fn matches_filter(c: &Checked, filters: &[(Option<Coord>, String)]) -> bool {
    if filters.is_empty() {
        return true;
    }
    filters.iter().any(|(coord, loose)| {
        coord.as_ref().is_some_and(|k| *k == c.dep.coord)
            || c.dep.coord.artifact.eq_ignore_ascii_case(loose)
            || c.dep.coord.group.eq_ignore_ascii_case(loose)
            || c.dep.coord.to_string().eq_ignore_ascii_case(loose)
    })
}

pub async fn run(ctx: &Ctx, args: UpdateArgs) -> Result<()> {
    let project = detect(&ctx.cwd)?;
    let reg = ctx.registry()?;
    let mut ws = Workspace::new();
    let scan = scan_project(&project, &mut ws, &ctx.config.repositories)?;
    let stability = if args.pre { Stability::Any } else { Stability::Stable };
    let table = aliases::resolve_table(&ctx.config.aliases);

    let filters: Vec<(Option<Coord>, String)> = args
        .names
        .iter()
        .map(|n| {
            let coord =
                Coord::parse(n).map(|(c, _)| c).or_else(|| table.get(&n.to_ascii_lowercase()).map(|a| a.coord.clone()));
            (coord, n.clone())
        })
        .collect();

    let checked = check_all(&reg, scan.deps.clone(), &scan.repos, stability, true).await;
    for e in checked.iter().filter(|c| c.error.is_some()).take(3) {
        eprintln!("{} {}: {}", term::warn_mark(), e.dep.display_name(), e.error.as_deref().unwrap_or(""));
    }

    // Which dependencies are candidates?
    let pool: Vec<&Checked> =
        checked.iter().filter(|c| matches_filter(c, &filters) && !c.candidates.is_empty()).collect();
    if pool.is_empty() {
        println!("{} nothing to update", term::ok_mark());
        return Ok(());
    }

    let interactive_level = !(args.patch || args.minor || args.all);
    let mut proposals: Vec<Proposal> = Vec::new();
    if interactive_level {
        if !term::interactive() {
            bail!("choose what to update with --patch, --minor or --all (or run kiln in a terminal)");
        }
        // One entry per dependency & bump class (safe update first, major separate).
        struct Entry {
            label: String,
            idx: usize,
            target: String,
            bump: Bump,
        }
        let mut entries: Vec<Entry> = Vec::new();
        let mut seen: Vec<(String, String, Bump)> = Vec::new();
        for (i, c) in pool.iter().enumerate() {
            let cur = c.dep.current.clone().unwrap_or_default();
            let safe = c
                .candidates
                .minor
                .clone()
                .map(|v| (v, Bump::Minor))
                .or_else(|| c.candidates.patch.clone().map(|v| (v, Bump::Patch)));
            let mut options = Vec::new();
            options.extend(safe);
            options.extend(c.candidates.major.clone().map(|v| (v, Bump::Major)));
            for (target, bump) in options {
                let key = (c.dep.display_name(), cur.clone(), bump);
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                let mark = if bump == Bump::Major { term::red("MAJOR") } else { term::yellow(bump.label()) };
                entries.push(Entry {
                    label: format!(
                        "{}  {} {} {}  [{}]  {}",
                        c.dep.display_name(),
                        cur,
                        term::arrow(),
                        target,
                        mark,
                        term::dim(&c.dep.module)
                    ),
                    idx: i,
                    target,
                    bump,
                });
            }
        }
        let labels: Vec<String> = entries.iter().map(|e| e.label.clone()).collect();
        let chosen =
            inquire::MultiSelect::new("Select updates to apply (space = toggle, enter = confirm)", labels.clone())
                .with_page_size(15)
                .prompt()?;
        for label in chosen {
            let e = &entries[labels.iter().position(|l| *l == label).unwrap_or(0)];
            proposals.push(Proposal { checked: pool[e.idx].clone(), target: e.target.clone(), bump: e.bump });
        }
        if proposals.is_empty() {
            println!("nothing selected");
            return Ok(());
        }
    } else {
        for c in &pool {
            if let Some((target, bump)) = target_for(c, &args) {
                proposals.push(Proposal { checked: (*c).clone(), target, bump });
            }
        }
        if proposals.is_empty() {
            println!("{} no updates at this level ({})", term::ok_mark(), if args.patch { "patch" } else { "minor" });
            return Ok(());
        }
    }

    // Declarations sharing one version site (a property / catalog version) move together.
    let mut by_site: BTreeMap<VersionSite, Vec<&Proposal>> = BTreeMap::new();
    for p in &proposals {
        by_site.entry(p.checked.dep.site.clone()).or_default().push(p);
    }
    let mut updates: Vec<(VersionSite, String)> = Vec::new();
    let mut report: Vec<(String, String, String, Bump, String)> = Vec::new(); // name, from, to, bump, module
    for (site, ps) in &by_site {
        let sharers: Vec<&Checked> = checked.iter().filter(|c| c.dep.site == *site).collect();
        let mut target =
            ps.iter().map(|p| Version::new(&p.target)).min().map(|v| v.as_str().to_string()).unwrap_or_default();
        // A sharer without a newer version caps the update.
        let blocked = sharers.iter().any(|s| s.candidates.is_empty() && s.error.is_none());
        let names: Vec<String> = sharers
            .iter()
            .map(|s| s.dep.coord.to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if names.len() > 1 {
            let cur = sharers[0].dep.current.clone().unwrap_or_default();
            if blocked {
                println!(
                    "{} {} share one version ({cur}) and not all have a newer release; skipped",
                    term::warn_mark(),
                    names.join(", ")
                );
                continue;
            }
            println!("{} {} share one version; using {target}", term::dim("note:"), names.join(", "));
        }
        if target.is_empty() {
            target = ps[0].target.clone();
        }
        updates.push((site.clone(), target.clone()));
        let p = ps[0];
        report.push((
            p.checked.dep.display_name(),
            p.checked.dep.current.clone().unwrap_or_default(),
            target,
            p.checked
                .dep
                .current
                .as_deref()
                .map(|c| Version::new(c).bump_to(&Version::new(&ps[0].target)))
                .unwrap_or(p.bump),
            p.checked.dep.module.clone(),
        ));
    }

    // Major updates: warn and summarize release notes.
    let majors: Vec<&(String, String, String, Bump, String)> = report.iter().filter(|r| r.3 == Bump::Major).collect();
    if !majors.is_empty() {
        println!("{} {} major update(s) may contain breaking changes:", term::warn_mark(), majors.len());
        let futs = proposals.iter().filter(|p| p.bump == Bump::Major).map(|p| {
            let repos: Vec<Repo> = repos_for(&reg, p.checked.dep.kind, &scan.repos, &[]);
            let reg = &reg;
            async move {
                let notes = reg.release_notes(&p.checked.dep.coord, &p.target, &repos).await;
                (p, notes)
            }
        });
        for (p, notes) in join_all(futs).await {
            println!(
                "  {} {} {} {}",
                term::bold(p.checked.dep.display_name()),
                p.checked.dep.current.clone().unwrap_or_default(),
                term::arrow(),
                term::red(&p.target)
            );
            if let Some(n) = notes {
                println!("    {} {}", term::cyan(&n.title), term::dim(&n.url));
                for line in n.summary {
                    println!("      {line}");
                }
            }
        }
        println!();
    }

    // Conflicts (same library, different versions).
    for c in conflicts(&scan.deps) {
        if proposals.iter().any(|p| p.checked.dep.coord == c.coord) {
            let list = c.versions.iter().map(|(m, v)| format!("{v} ({m})")).collect::<Vec<_>>().join(", ");
            println!("{} {} was declared with different versions: {list}", term::warn_mark(), c.coord);
        }
    }

    apply_updates(&mut ws, &updates)?;
    for (name, from, to, bump, module) in &report {
        let b = if *bump == Bump::Major { term::red(bump.label()) } else { term::yellow(bump.label()) };
        println!(
            "{} {} {} {} {} [{}] {}",
            term::ok_mark(),
            term::bold(name),
            from,
            term::arrow(),
            term::green(to),
            b,
            term::dim(module)
        );
    }
    if !ws.has_changes() {
        println!("no changes were necessary");
        return Ok(());
    }
    finish(&mut ws, &project, args.dry_run)?;
    if args.dry_run {
        return Ok(());
    }
    if args.verify {
        verify(&project, &ws).await?;
    }
    Ok(())
}

fn build_command(project: &Project) -> (String, Vec<String>) {
    match project.kind {
        BuildKind::Gradle => match project.gradle_wrapper() {
            Some(w) => (w.to_string_lossy().to_string(), vec!["build".into()]),
            None => ("gradle".into(), vec!["build".into()]),
        },
        BuildKind::Maven => match project.maven_wrapper() {
            Some(w) => (w.to_string_lossy().to_string(), vec!["-B".into(), "verify".into()]),
            None => ("mvn".into(), vec!["-B".into(), "verify".into()]),
        },
    }
}

async fn verify(project: &Project, ws: &Workspace) -> Result<()> {
    let (program, args) = build_command(project);
    println!("\n{} verifying: {} {}", term::cyan("=>"), program, args.join(" "));
    // On Windows, `gradle`/`mvn` are .bat/.cmd scripts that must go through cmd.exe.
    let needs_cmd = cfg!(windows) && !program.contains(['\\', '/']);
    let mut cmd = if needs_cmd {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&program).args(&args);
        c
    } else {
        let mut c = tokio::process::Command::new(&program);
        c.args(&args);
        c
    };
    cmd.current_dir(&project.root);
    let status = cmd.status().await;
    let ok = matches!(&status, Ok(s) if s.success());
    if ok {
        println!("{} build succeeded", term::ok_mark());
        return Ok(());
    }
    match status {
        Ok(s) => eprintln!("{} build failed ({s})", term::err_mark()),
        Err(e) => eprintln!("{} could not run `{program}`: {e}", term::err_mark()),
    }
    let revert = if term::interactive() {
        inquire::Confirm::new("Revert the dependency changes?").with_default(true).prompt().unwrap_or(true)
    } else {
        true
    };
    if revert {
        ws.rollback_on_disk()?;
        println!("{} changes reverted", term::ok_mark());
        bail!("verification failed; updates were rolled back");
    }
    bail!("verification failed; changes were kept")
}
