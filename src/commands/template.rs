use anyhow::{Result, bail};

use crate::cli::TemplateCommand;
use crate::context::Ctx;
use crate::templates::{builtin, custom, save};
use crate::util::{paths, term};

pub async fn run(ctx: &Ctx, cmd: TemplateCommand) -> Result<()> {
    match cmd {
        TemplateCommand::List => list(),
        TemplateCommand::Path => {
            println!("{}", paths::templates_dir()?.display());
            Ok(())
        }
        TemplateCommand::Remove { name } => {
            custom::remove(&name)?;
            println!("{} removed template {}", term::ok_mark(), term::bold(&name));
            Ok(())
        }
        TemplateCommand::Save { name, from, project_name, package, force } => {
            let from = from.unwrap_or_else(|| ctx.cwd.clone());
            if builtin::find(&name).is_some() {
                eprintln!(
                    "{} `{name}` is also a built-in template; your custom template will take precedence",
                    term::warn_mark()
                );
            }
            let report = save::save(&save::SaveOptions { name: name.clone(), from, project_name, package, force })?;
            println!(
                "{} Saved template {} {}",
                term::ok_mark(),
                term::bold(&name),
                term::dim(format!("({} files, {})", report.files, report.dir.display()))
            );
            println!("  project name  {} {} {{{{name}}}}", report.project_name, term::arrow());
            match &report.package {
                Some(p) => println!("  package       {p} {} {{{{package}}}}", term::arrow()),
                None => println!("  package       (none detected; pass --package to templatize it)"),
            }
            println!("\nUse it with: kiln new <name> --template {name}");
            Ok(())
        }
    }
}

fn list() -> Result<()> {
    let customs = custom::list()?;
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for b in builtin::all() {
        if customs.iter().any(|c| c.name == b.id) {
            continue;
        }
        rows.push((b.id.to_string(), "built-in".into(), b.description.to_string()));
    }
    for c in &customs {
        let overrides = builtin::find(&c.name).is_some();
        let desc = if c.manifest.description.is_empty() {
            "(no description)".to_string()
        } else {
            c.manifest.description.clone()
        };
        rows.push((
            c.name.clone(),
            if overrides { "custom (overrides built-in)".into() } else { "custom".into() },
            desc,
        ));
    }
    if rows.is_empty() {
        bail!("no templates found");
    }
    let w0 = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(8);
    let w1 = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).max(6);
    println!(
        "{}  {}  {}",
        term::bold(format!("{:<w0$}", "Template")),
        term::bold(format!("{:<w1$}", "Source")),
        term::bold("Description")
    );
    for (n, s, d) in rows {
        println!("{n:<w0$}  {s:<w1$}  {d}");
    }
    println!("\n{}", term::dim(format!("Custom templates live in {}", paths::templates_dir()?.display())));
    Ok(())
}
