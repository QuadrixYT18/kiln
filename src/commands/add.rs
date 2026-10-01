use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use futures::future::join_all;

use super::{finish, pick_module};
use crate::cli::AddArgs;
use crate::context::Ctx;
use crate::project::detect::detect;
use crate::project::edit::{AddOutcome, AddRequest, add_dependency};
use crate::project::model::Scope;
use crate::project::scan::scan_project;
use crate::project::workspace::Workspace;
use crate::registry::aliases::{self, Alias};
use crate::registry::version::{Stability, Version};
use crate::registry::{Coord, Registry, Repo, SearchHit};
use crate::util::term;

enum Resolution {
    Exact { coord: Coord, version: Option<String>, repo: Option<Repo> },
    Choices { query: String, version: Option<String>, hits: Vec<SearchHit> },
}

async fn resolve(reg: &Registry, table: &BTreeMap<String, Alias>, input: &str) -> Result<Resolution> {
    let (name, at_version) = match input.rsplit_once('@') {
        Some((n, v)) if !n.is_empty() && !v.is_empty() => (n, Some(v.to_string())),
        _ => (input, None),
    };
    if let Some((coord, v)) = Coord::parse(name) {
        return Ok(Resolution::Exact { coord, version: at_version.or(v), repo: None });
    }
    if let Some(a) = table.get(&name.to_ascii_lowercase()) {
        return Ok(Resolution::Exact { coord: a.coord.clone(), version: at_version, repo: a.repo.clone() });
    }
    let hits = reg.search(name).await.map_err(|e| {
        anyhow!("could not search for `{name}`: {e}\n  hint: use explicit coordinates (group:artifact) or an alias")
    })?;
    if hits.is_empty() {
        bail!("no artifact found for `{name}`\n  hint: use explicit coordinates (group:artifact)");
    }
    Ok(Resolution::Choices { query: name.to_string(), version: at_version, hits })
}

fn describe(hit: &SearchHit) -> String {
    let when =
        hit.timestamp_ms.map(|ms| format!(", updated {}", crate::util::time::year_month(ms))).unwrap_or_default();
    format!("{}  (latest {}{when})", hit.coord, hit.latest.as_deref().unwrap_or("?"))
}

pub async fn run(ctx: &Ctx, args: AddArgs) -> Result<()> {
    let project = detect(&ctx.cwd)?;
    let module = pick_module(&project, args.module.as_deref(), &ctx.cwd)?;
    let reg = ctx.registry()?;
    let table = aliases::resolve_table(&ctx.config.aliases);
    let scope = if args.test {
        Scope::Test
    } else if args.runtime {
        Scope::Runtime
    } else if args.compile_only {
        Scope::CompileOnly
    } else if args.annotation_processor {
        Scope::AnnotationProcessor
    } else {
        Scope::Compile
    };
    let stability = if args.pre { Stability::Any } else { Stability::Stable };

    let mut ws = Workspace::new();
    let scan = scan_project(&project, &mut ws, &ctx.config.repositories)?;

    // 1. Resolve all names concurrently.
    let spinner = term::spinner("Resolving dependencies");
    let resolved = join_all(args.names.iter().map(|n| resolve(&reg, &table, n))).await;
    spinner.finish_and_clear();

    // 2. Disambiguate sequentially.
    let mut targets: Vec<(Coord, Option<String>, Option<Repo>)> = Vec::new();
    let mut failed = false;
    for (input, res) in args.names.iter().zip(resolved) {
        match res {
            Err(e) => {
                eprintln!("{} {e}", term::err_mark());
                failed = true;
            }
            Ok(Resolution::Exact { coord, version, repo }) => targets.push((coord, version, repo)),
            Ok(Resolution::Choices { query, version, hits }) => {
                let top: Vec<SearchHit> = hits.into_iter().take(10).collect();
                if top.len() == 1 || !term::interactive() {
                    if top.len() > 1 {
                        eprintln!(
                            "{} `{query}` is ambiguous; using {} (be explicit with group:artifact)",
                            term::warn_mark(),
                            top[0].coord
                        );
                    }
                    targets.push((top[0].coord.clone(), version, None));
                } else {
                    let labels: Vec<String> = top.iter().map(describe).collect();
                    let picked =
                        inquire::Select::new(&format!("Multiple matches for `{input}`:"), labels.clone()).prompt()?;
                    let idx = labels.iter().position(|l| *l == picked).unwrap_or(0);
                    targets.push((top[idx].coord.clone(), version, None));
                }
            }
        }
    }

    // 3. Look up versions concurrently.
    let lookups = join_all(targets.iter().map(|(coord, version, repo)| {
        let reg = &reg;
        let mut repos = vec![reg.central()];
        repos.extend(scan.repos.iter().cloned());
        repos.extend(repo.iter().cloned());
        repos.extend(ctx.config.repositories.iter().map(|u| Repo::new(u)));
        let coord = coord.clone();
        let version = version.clone();
        async move {
            let all = reg.versions(&coord, &repos).await?;
            match version {
                Some(v) => {
                    if !all.iter().any(|a| Version::new(a) == Version::new(&v)) {
                        eprintln!(
                            "{} version {v} of {coord} was not found in the repositories; using it anyway",
                            term::warn_mark()
                        );
                    }
                    Ok::<String, anyhow::Error>(v)
                }
                None => crate::registry::version::latest(&all, stability)
                    .ok_or_else(|| anyhow!("{coord} has no stable release (use --pre to allow pre-releases)")),
            }
        }
    }))
    .await;

    // 4. Apply edits.
    let known: Vec<Repo> = scan.repos.clone();
    let mut added = 0;
    for ((coord, _, repo), version) in targets.iter().zip(lookups) {
        let version = match version {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{} {coord}: {e}", term::err_mark());
                failed = true;
                continue;
            }
        };
        let req = AddRequest {
            coord,
            version: Some(&version),
            scope,
            module: &module,
            repo: repo.as_ref(),
            known_repos: &known,
        };
        match add_dependency(&mut ws, &project, &req)? {
            AddOutcome::Added { notes } => {
                added += 1;
                println!(
                    "{} {} {} {} {} {}",
                    term::ok_mark(),
                    if args.dry_run { "Would add" } else { "Added" },
                    term::bold(coord),
                    term::cyan(&version),
                    term::dim(format!("({})", scope_label(&project, scope))),
                    term::dim(format!("to {}", module.name)),
                );
                for n in notes {
                    println!("  {} {n}", term::dim("note:"));
                }
            }
            AddOutcome::AlreadyPresent { version } => {
                println!(
                    "{} {} is already declared in {}{} {}",
                    term::warn_mark(),
                    coord,
                    module.name,
                    version.map(|v| format!(" ({v})")).unwrap_or_default(),
                    term::dim("- use `kiln update` to upgrade")
                );
            }
        }
    }

    if added > 0 {
        finish(&mut ws, &project, args.dry_run)?;
    }
    if failed {
        bail!("some dependencies could not be added");
    }
    Ok(())
}

fn scope_label(project: &crate::project::model::Project, scope: Scope) -> &'static str {
    match project.kind {
        crate::project::model::BuildKind::Gradle => scope.gradle_config(),
        crate::project::model::BuildKind::Maven => scope.maven_scope().unwrap_or("compile"),
    }
}
