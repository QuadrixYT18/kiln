pub mod add;
pub mod cache;
pub mod completions;
pub mod new;
pub mod outdated;
pub mod remove;
pub mod template;
pub mod update;

use std::path::Path;

use anyhow::Result;

use crate::project::model::{Module, Project};
use crate::project::workspace::Workspace;
use crate::util::term;

/// Chooses the module to edit, asking interactively when the project has several.
pub fn pick_module(project: &Project, arg: Option<&str>, cwd: &Path) -> Result<Module> {
    match project.select_module(arg, cwd) {
        Ok(m) => Ok(m.clone()),
        Err(e) => {
            if arg.is_none() && term::interactive() && project.modules.len() > 1 {
                let names: Vec<String> = project
                    .modules
                    .iter()
                    .map(|m| format!("{}  ({})", m.name, m.dir.strip_prefix(&project.root).unwrap_or(&m.dir).display()))
                    .collect();
                let choice = inquire::Select::new("Which module?", names.clone()).prompt()?;
                let idx = names.iter().position(|n| *n == choice).unwrap_or(0);
                Ok(project.modules[idx].clone())
            } else {
                Err(e)
            }
        }
    }
}

/// Prints the diff for `--dry-run`, or writes the files and lists them.
pub fn finish(ws: &mut Workspace, project: &Project, dry_run: bool) -> Result<()> {
    if !ws.has_changes() {
        return Ok(());
    }
    if dry_run {
        println!("{}", ws.diff(&project.root));
        println!("{} dry run: no files were written", term::warn_mark());
        return Ok(());
    }
    for p in ws.write_all()? {
        let rel = p.strip_prefix(&project.root).unwrap_or(&p);
        println!("  {} {}", term::dim("wrote"), rel.display());
    }
    Ok(())
}
