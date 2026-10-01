use anyhow::{Result, bail};

use super::finish;
use crate::cli::RemoveArgs;
use crate::context::Ctx;
use crate::project::detect::detect;
use crate::project::edit::{Selector, remove_dependency};
use crate::project::model::module_label;
use crate::project::workspace::Workspace;
use crate::registry::aliases;
use crate::util::term;

pub async fn run(ctx: &Ctx, args: RemoveArgs) -> Result<()> {
    let project = detect(&ctx.cwd)?;
    let table = aliases::resolve_table(&ctx.config.aliases);
    let modules: Vec<&_> = match &args.module {
        Some(m) => vec![project.select_module(Some(m), &ctx.cwd)?],
        None => project.modules.iter().collect(),
    };
    let mut ws = Workspace::new();
    let mut failed = false;
    for name in &args.names {
        let sel = Selector::new(name, &table);
        match remove_dependency(&mut ws, &project, &modules, &sel) {
            Ok(out) if !out.removed_from.is_empty() => {
                println!(
                    "{} {} {} {}",
                    term::ok_mark(),
                    if args.dry_run { "Would remove" } else { "Removed" },
                    term::bold(name),
                    term::dim(format!(
                        "from {}",
                        out.removed_from.iter().map(|m| module_label(m)).collect::<Vec<_>>().join(", ")
                    ))
                );
                for n in out.notes {
                    println!("  {} {n}", term::dim("note:"));
                }
            }
            Ok(out) => {
                eprintln!("{} `{name}` was not found in any build file", term::warn_mark());
                for n in out.notes {
                    println!("  {} {n}", term::dim("note:"));
                }
                failed = true;
            }
            Err(e) => {
                eprintln!("{} {name}: {e}", term::warn_mark());
                failed = true;
            }
        }
    }
    finish(&mut ws, &project, args.dry_run)?;
    if failed && !ws.has_changes() {
        bail!("nothing was removed");
    }
    Ok(())
}
