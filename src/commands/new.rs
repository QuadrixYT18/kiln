use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::cli::NewArgs;
use crate::context::Ctx;
use crate::registry::{Registry, Repo};
use crate::templates::render::Vars;
use crate::templates::save::pascal_case;
use crate::templates::{Input, OutFile, builtin, custom, render_inputs, scan_inputs, tokens, wrapper};
use crate::util::{term, time};

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "var",
    "record",
    "yield",
    "true",
    "false",
    "null",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ci {
    None,
    Github,
    Gitlab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum License {
    None,
    Mit,
    Apache,
    Both,
}

enum Source {
    Builtin(builtin::Builtin),
    Custom(custom::Custom),
}

struct Plan {
    name: String,
    source: Source,
    kotlin: bool,
    java: u32,
    group: String,
    package: String,
    description: String,
    author: String,
    framework: String,
    paperweight: bool,
    docker: bool,
    db: Vec<String>,
    ci: Ci,
    renovate: bool,
    editorconfig: bool,
    gitignore: bool,
    gitattributes: bool,
    readme: bool,
    license: License,
    git: bool,
    extra_vars: BTreeMap<String, String>,
    output: PathBuf,
}

pub fn valid_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("the project name must not be empty");
    }
    if name.len() > 100 {
        bail!("the project name is too long");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) || name.starts_with(['.', '-']) {
        bail!("invalid project name `{name}`: use letters, digits, `-`, `_` and `.` (no spaces or path separators)");
    }
    let reserved = ["con", "prn", "aux", "nul", "com1", "lpt1"];
    if reserved.contains(&name.to_ascii_lowercase().as_str()) {
        bail!("`{name}` is a reserved name on Windows");
    }
    Ok(())
}

/// Lowercase a-z0-9 identifier segment usable in a Java package.
pub fn package_segment(name: &str) -> String {
    let mut s: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    if s.is_empty() {
        s = "app".into();
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, 'p');
    }
    if JAVA_KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

pub fn sanitize_group(group: &str) -> String {
    let parts: Vec<String> = group
        .split('.')
        .map(|p| p.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .map(|mut p| {
            if p.starts_with(|c: char| c.is_ascii_digit()) {
                p.insert(0, '_');
            }
            if JAVA_KEYWORDS.contains(&p.as_str()) {
                p.push('_');
            }
            p
        })
        .collect();
    if parts.is_empty() { "com.example".to_string() } else { parts.join(".") }
}

fn plugin_id(name: &str) -> String {
    let mut s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    s = s.trim_matches(['-', '_']).to_string();
    if !s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s.insert(0, 'p');
    }
    s.truncate(64);
    s
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$")
}

fn yaml_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn git_user_name() -> Option<String> {
    let out = std::process::Command::new("git").args(["config", "user.name"]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn default_author(ctx: &Ctx) -> String {
    ctx.config
        .new
        .author
        .clone()
        .or_else(git_user_name)
        .or_else(|| std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Your Name".to_string())
}

fn parse_license(s: &str) -> License {
    match s.to_ascii_lowercase().as_str() {
        "mit" => License::Mit,
        "apache-2.0" | "apache" => License::Apache,
        "mit-or-apache-2.0" | "mit or apache-2.0" | "dual" => License::Both,
        _ => License::None,
    }
}

fn license_name(l: License) -> &'static str {
    match l {
        License::None => "",
        License::Mit => "MIT",
        License::Apache => "Apache-2.0",
        License::Both => "MIT OR Apache-2.0",
    }
}

fn find_source(name: &str) -> Result<Source> {
    if let Some(c) = custom::find(name)? {
        return Ok(Source::Custom(c));
    }
    if let Some(b) = builtin::find(name) {
        return Ok(Source::Builtin(b));
    }
    let mut names: Vec<String> = builtin::all().iter().map(|b| b.id.to_string()).collect();
    names.extend(custom::list()?.into_iter().map(|c| c.name));
    bail!("unknown template `{name}`; available: {}", names.join(", "))
}

fn parse_var(s: &str) -> Result<(String, String)> {
    let (k, v) = s.split_once('=').ok_or_else(|| anyhow!("--var expects KEY=VALUE, got `{s}`"))?;
    if k.is_empty() {
        bail!("--var expects KEY=VALUE, got `{s}`");
    }
    Ok((k.trim().to_string(), v.to_string()))
}

fn prompt_text(msg: &str, default: Option<&str>) -> Result<String> {
    let mut p = inquire::Text::new(msg);
    if let Some(d) = default {
        p = p.with_default(d);
    }
    Ok(p.prompt()?)
}

fn gather(ctx: &Ctx, args: NewArgs) -> Result<Plan> {
    let interactive = term::interactive();
    let wizard = interactive && (args.name.is_none() || args.template.is_none());

    // Name
    let name = match args.name.clone() {
        Some(n) => n,
        None if interactive => inquire::Text::new("Project name")
            .with_validator(|s: &str| match valid_project_name(s) {
                Ok(()) => Ok(inquire::validator::Validation::Valid),
                Err(e) => Ok(inquire::validator::Validation::Invalid(e.to_string().into())),
            })
            .prompt()?,
        None => bail!("missing project name; usage: kiln new <name> --template <template>"),
    };
    valid_project_name(&name)?;

    // Template
    let mut options: Vec<(String, String)> =
        builtin::all().iter().map(|b| (b.id.to_string(), format!("{:<9} {}", b.id, b.description))).collect();
    for c in custom::list()? {
        let d = if c.manifest.description.is_empty() {
            "custom template".to_string()
        } else {
            c.manifest.description.clone()
        };
        options.retain(|(id, _)| *id != c.name);
        options.push((c.name.clone(), format!("{:<9} {d} (custom)", c.name)));
    }
    let template = match args.template.clone() {
        Some(t) => t,
        None if interactive => {
            let labels: Vec<String> = options.iter().map(|o| o.1.clone()).collect();
            let picked = inquire::Select::new("Template", labels.clone()).prompt()?;
            options[labels.iter().position(|l| *l == picked).unwrap_or(0)].0.clone()
        }
        None => bail!(
            "missing --template; available: {}",
            options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>().join(", ")
        ),
    };
    let source = find_source(&template)?;
    let (is_paper, is_backend, is_builtin, langs): (bool, bool, bool, Vec<&str>) = match &source {
        Source::Builtin(b) => (b.id == "paper", b.id == "backend", true, b.langs.to_vec()),
        Source::Custom(_) => (false, false, false, vec![]),
    };

    // Language
    let mut lang = args.lang.clone();
    if lang.is_none() && wizard && langs.len() > 1 {
        let pick = inquire::Select::new("Language", langs.iter().map(|s| s.to_string()).collect()).prompt()?;
        lang = Some(pick);
    }
    let framework = if is_backend {
        match args.framework.clone() {
            Some(f) => f,
            None if wizard => {
                inquire::Select::new("Framework", vec!["ktor".to_string(), "spring".to_string()]).prompt()?
            }
            None => "ktor".to_string(),
        }
    } else {
        String::new()
    };
    let default_lang = langs.first().copied().unwrap_or("java").to_string();
    let mut lang = lang.unwrap_or(default_lang);
    if is_backend && framework == "ktor" {
        if args.lang.as_deref() == Some("java") {
            bail!("the Ktor template is Kotlin-only");
        }
        lang = "kotlin".into();
    }
    if is_builtin && !langs.contains(&lang.as_str()) {
        bail!("template `{template}` does not support --lang {lang}");
    }
    let kotlin = lang == "kotlin";

    let mut paperweight = args.paperweight;
    if is_paper && wizard && !paperweight {
        paperweight = inquire::Confirm::new("Use paperweight-userdev (access to server internals)?")
            .with_default(false)
            .prompt()?;
    }
    if args.paperweight && !is_paper {
        bail!("--paperweight only applies to the paper template");
    }

    // Java / group / description
    let default_java = ctx.config.new.java.unwrap_or(21);
    let java = match args.java {
        Some(j) => j,
        None if wizard => {
            let choices = vec!["21", "17", "25", "11"];
            let pick = inquire::Select::new("Java version", choices).prompt()?;
            pick.parse().unwrap_or(default_java)
        }
        None => default_java,
    };
    if !(8..=99).contains(&java) {
        bail!("--java {java} is not a valid Java version");
    }
    let default_group = ctx.config.new.group.clone().unwrap_or_else(|| "com.example".to_string());
    let group = match args.group.clone() {
        Some(g) => g,
        None if wizard => prompt_text("Group id", Some(&default_group))?,
        None => default_group,
    };
    let group = sanitize_group(&group);
    let package = match args.package.clone() {
        Some(p) => sanitize_group(&p),
        None => format!("{group}.{}", package_segment(&name)),
    };
    let default_desc = match &source {
        Source::Builtin(b) => b.default_description.to_string(),
        Source::Custom(c) if !c.manifest.description.is_empty() => c.manifest.description.clone(),
        Source::Custom(_) => "A JVM project".to_string(),
    };
    let description = match args.description.clone() {
        Some(d) => d,
        None if wizard => prompt_text("Description", Some(&default_desc))?,
        None => default_desc,
    };
    let author = default_author(ctx);

    // Building blocks
    let mut docker = args.docker || !args.db.is_empty();
    let mut db = args.db.clone();
    let mut ci = match args.ci.as_deref() {
        Some("github") => Ci::Github,
        Some("gitlab") => Ci::Gitlab,
        _ => Ci::None,
    };
    let mut renovate = args.renovate;
    let (mut editorconfig, mut gitignore, mut gitattributes, mut readme) =
        (!args.bare, !args.bare, !args.bare, !args.bare);
    let mut license = args
        .license
        .as_deref()
        .map(parse_license)
        .or_else(|| ctx.config.new.license.as_deref().map(parse_license))
        .unwrap_or(License::None);
    if args.license.is_none() && !wizard {
        // Only an explicit flag (or config default) adds a license in flag mode.
        license = args
            .license
            .as_deref()
            .map(parse_license)
            .or_else(|| ctx.config.new.license.as_deref().map(parse_license))
            .unwrap_or(License::None);
    }
    let mut git = !args.no_git;

    if wizard {
        let items = [
            "Dockerfile / docker-compose",
            "GitHub Actions workflow",
            "GitLab CI pipeline",
            "Renovate config",
            ".editorconfig",
            ".gitignore",
            ".gitattributes",
            "README",
            "License file",
        ];
        let mut defaults = vec![4, 5, 6, 7];
        if docker {
            defaults.push(0);
        }
        if ci == Ci::Github {
            defaults.push(1);
        }
        if ci == Ci::Gitlab {
            defaults.push(2);
        }
        if renovate {
            defaults.push(3);
        }
        if license != License::None {
            defaults.push(8);
        }
        let chosen = inquire::MultiSelect::new(
            "Optional building blocks (space = toggle)",
            items.iter().map(|s| s.to_string()).collect(),
        )
        .with_default(&defaults)
        .prompt()?;
        let has = |label: &str| chosen.iter().any(|c| c == label);
        docker = has(items[0]);
        ci = if has(items[1]) {
            Ci::Github
        } else if has(items[2]) {
            Ci::Gitlab
        } else {
            Ci::None
        };
        renovate = has(items[3]);
        editorconfig = has(items[4]);
        gitignore = has(items[5]);
        gitattributes = has(items[6]);
        readme = has(items[7]);
        if has(items[8]) && license == License::None {
            let pick = inquire::Select::new("License", vec!["MIT", "Apache-2.0", "MIT OR Apache-2.0"]).prompt()?;
            license = parse_license(&pick.replace(' ', "-").to_ascii_lowercase());
            if pick == "MIT OR Apache-2.0" {
                license = License::Both;
            }
        } else if !has(items[8]) {
            license = License::None;
        }
        if docker && db.is_empty() && is_backend {
            let services =
                inquire::MultiSelect::new("docker-compose services", vec!["postgres".to_string(), "redis".to_string()])
                    .prompt()?;
            db = services;
        }
        git = inquire::Confirm::new("Initialize a git repository?").with_default(!args.no_git).prompt()?;
    }

    // Variables for custom templates.
    let mut extra_vars = BTreeMap::new();
    for v in &args.vars {
        let (k, val) = parse_var(v)?;
        extra_vars.insert(k, val);
    }

    let output = match &args.output {
        Some(o) => o.clone(),
        None => ctx.cwd.clone(),
    };
    Ok(Plan {
        name,
        source,
        kotlin,
        java,
        group,
        package,
        description,
        author,
        framework,
        paperweight,
        docker,
        db,
        ci,
        renovate,
        editorconfig,
        gitignore,
        gitattributes,
        readme,
        license,
        git,
        extra_vars,
        output,
    })
}

fn flag(b: bool) -> String {
    if b { "true" } else { "false" }.to_string()
}

fn base_vars(plan: &Plan) -> Vars {
    let mut v = Vars::new();
    let template_id = match &plan.source {
        Source::Builtin(b) => b.id.to_string(),
        Source::Custom(c) => c.name.clone(),
    };
    v.insert("name".into(), plan.name.clone());
    v.insert("artifact_id".into(), plan.name.to_ascii_lowercase());
    v.insert("group".into(), plan.group.clone());
    v.insert("package".into(), plan.package.clone());
    v.insert("package_path".into(), plan.package.replace('.', "/"));
    v.insert("class_name".into(), pascal_case(&plan.name));
    v.insert("plugin_id".into(), plugin_id(&plan.name));
    v.insert("version".into(), "0.1.0".into());
    v.insert("description".into(), plan.description.clone());
    v.insert("description_str".into(), escape_str(&plan.description));
    v.insert("description_yaml".into(), yaml_quote(&plan.description));
    v.insert("author".into(), plan.author.clone());
    v.insert("author_str".into(), escape_str(&plan.author));
    v.insert("author_yaml".into(), yaml_quote(&plan.author));
    v.insert("year".into(), time::year_now().to_string());
    v.insert("java_version".into(), plan.java.to_string());
    v.insert("kotlin".into(), flag(plan.kotlin));
    v.insert("lang".into(), if plan.kotlin { "kotlin" } else { "java" }.into());
    v.insert("src_dir".into(), if plan.kotlin { "kotlin" } else { "java" }.into());
    v.insert("ext".into(), if plan.kotlin { "kt" } else { "java" }.into());
    v.insert("template".into(), template_id.clone());
    v.insert("paper".into(), flag(template_id == "paper" && matches!(plan.source, Source::Builtin(_))));
    v.insert("velocity".into(), flag(template_id == "velocity" && matches!(plan.source, Source::Builtin(_))));
    v.insert("backend".into(), flag(template_id == "backend" && matches!(plan.source, Source::Builtin(_))));
    v.insert("paperweight".into(), flag(plan.paperweight));
    v.insert("ktor".into(), flag(plan.framework == "ktor"));
    v.insert("spring".into(), flag(plan.framework == "spring"));
    v.insert("docker".into(), flag(plan.docker));
    v.insert("postgres".into(), flag(plan.db.iter().any(|d| d == "postgres")));
    v.insert("redis".into(), flag(plan.db.iter().any(|d| d == "redis")));
    v.insert("db_name".into(), plan.name.to_ascii_lowercase().replace(['-', '.'], "_"));
    v.insert("has_license".into(), flag(plan.license != License::None));
    v.insert("license_name".into(), license_name(plan.license).into());
    let volumes = v["paper"] == "true" || v["velocity"] == "true" || v["postgres"] == "true";
    v.insert("volumes".into(), flag(volumes));
    v
}

fn text_file(path: &str, content: &str, vars: &Vars) -> Result<Input> {
    let rendered =
        crate::templates::render::render(content, vars, None).with_context(|| format!("while rendering {path}"))?;
    Ok(Input { path: path.to_string(), data: rendered.into_bytes() })
}

pub async fn run(ctx: &Ctx, args: NewArgs) -> Result<()> {
    let plan = gather(ctx, args)?;
    let target = plan.output.join(&plan.name);
    if target.exists() && std::fs::read_dir(&target).map(|mut d| d.next().is_some()).unwrap_or(true) {
        bail!("{} already exists and is not empty", target.display());
    }
    let reg = ctx.registry()?;
    let mut vars = base_vars(&plan);
    let mut repos: Vec<Repo> = ctx.config.repositories.iter().map(|u| Repo::new(u)).collect();

    // Inputs per template kind.
    let mut inputs: Vec<Input> = Vec::new();
    let mut derived: Vec<(String, String)> = Vec::new();
    let mut manifest_vars = BTreeMap::new();
    let mut is_gradle = true;
    match &plan.source {
        Source::Builtin(b) => {
            repos.extend(b.repos.iter().map(|u| Repo::new(&crate::registry::repo_override(u))));
            derived = b.derived.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
            inputs.push(text_file("settings.gradle.kts", builtin::SETTINGS, &vars).map(|mut i| {
                i.data = builtin::SETTINGS.as_bytes().to_vec();
                i
            })?);
            inputs
                .push(Input { path: "gradle.properties".into(), data: builtin::GRADLE_PROPERTIES.as_bytes().to_vec() });
            for f in &b.files {
                if builtin::condition_holds(f.when, &vars) {
                    inputs.push(Input { path: f.target.to_string(), data: f.content.as_bytes().to_vec() });
                }
            }
        }
        Source::Custom(c) => {
            repos.extend(c.manifest.repositories.iter().map(|u| Repo::new(u)));
            manifest_vars = c.manifest.variables.clone();
            for (path, data) in custom::files(c)? {
                inputs.push(Input { path, data });
            }
            is_gradle = inputs.iter().any(|i| i.path == "settings.gradle.kts" || i.path == "settings.gradle");
        }
    }

    // Auto-detect custom placeholders and ask for them.
    let found = scan_inputs(&inputs);
    let known: Vars = vars.clone();
    let mut missing: Vec<String> = found
        .variables
        .iter()
        .filter(|v| {
            !known.contains_key(*v) && !plan.extra_vars.contains_key(*v) && !derived.iter().any(|(k, _)| k == *v)
        })
        .cloned()
        .collect();
    missing.sort();
    for (k, v) in &plan.extra_vars {
        vars.insert(k.clone(), v.clone());
    }
    let mut unresolved = Vec::new();
    for key in missing {
        let spec = manifest_vars.get(&key);
        let default = spec.and_then(|s| s.default.clone());
        if term::interactive() {
            let prompt = spec.and_then(|s| s.prompt.clone()).unwrap_or_else(|| format!("Value for `{key}`"));
            vars.insert(key.clone(), prompt_text(&prompt, default.as_deref())?);
        } else if let Some(d) = default {
            vars.insert(key.clone(), d);
        } else {
            unresolved.push(key);
        }
    }
    if !unresolved.is_empty() {
        bail!(
            "the template needs values for: {}\n  hint: pass them with --var, e.g. --var {}=...",
            unresolved.join(", "),
            unresolved[0]
        );
    }
    for flagname in &found.flags {
        vars.entry(flagname.clone()).or_default();
    }

    // Live lookups: derived variables first (e.g. the Minecraft version).
    let spinner = term::spinner("Looking up current versions");
    if !derived.is_empty() {
        let bodies: Vec<String> = derived.iter().map(|(_, t)| t.clone()).collect();
        let map = tokens::resolve(&reg, &bodies, &repos).await.inspect_err(|_| spinner.finish_and_clear())?;
        for (k, t) in &derived {
            vars.insert(k.clone(), map[t].clone());
        }
    }
    let gradle_release =
        if is_gradle { Some(reg.gradle_current().await.inspect_err(|_| spinner.finish_and_clear())?) } else { None };
    let (mut files, _resolved) =
        render_inputs(Some(&reg), inputs, &vars, &repos, &[]).await.inspect_err(|_| spinner.finish_and_clear())?;
    spinner.finish_and_clear();

    // Wrapper and optional building blocks.
    let have = |files: &[OutFile], p: &str| files.iter().any(|f| f.path == p);
    let add_text = |files: &mut Vec<OutFile>, path: &str, content: &str| -> Result<()> {
        if have(files, path) {
            return Ok(());
        }
        let data = crate::templates::render::render(content, &vars, Some(&BTreeMap::new()))
            .with_context(|| format!("while rendering {path}"))?;
        files.push(OutFile { path: path.to_string(), data: data.into_bytes(), exec: false });
        Ok(())
    };
    if let Some(rel) = &gradle_release {
        for w in wrapper::files(rel) {
            if !have(&files, &w.path) {
                files.push(w);
            }
        }
    }
    if plan.editorconfig {
        add_text(&mut files, ".editorconfig", builtin::EDITORCONFIG)?;
    }
    if plan.gitignore {
        add_text(&mut files, ".gitignore", builtin::GITIGNORE)?;
    }
    if plan.gitattributes {
        add_text(&mut files, ".gitattributes", builtin::GITATTRIBUTES)?;
    }
    if plan.readme {
        add_text(&mut files, "README.md", builtin::README)?;
    }
    if plan.renovate {
        add_text(&mut files, "renovate.json", builtin::RENOVATE)?;
    }
    match plan.ci {
        Ci::Github => add_text(&mut files, ".github/workflows/ci.yml", builtin::GITHUB_CI)?,
        Ci::Gitlab => add_text(&mut files, ".gitlab-ci.yml", builtin::GITLAB_CI)?,
        Ci::None => {}
    }
    if plan.docker {
        match &plan.source {
            Source::Builtin(b) if matches!(b.id, "backend" | "paper" | "velocity") => {
                if b.id == "backend" {
                    add_text(&mut files, "Dockerfile", builtin::DOCKERFILE_BACKEND)?;
                }
                add_text(&mut files, "docker-compose.yml", builtin::DOCKER_COMPOSE)?;
            }
            _ => eprintln!(
                "{} Docker files are only generated for the paper, velocity and backend templates",
                term::warn_mark()
            ),
        }
    }
    match plan.license {
        License::None => {}
        License::Mit => add_text(&mut files, "LICENSE", builtin::LICENSE_MIT)?,
        License::Apache => files.push(OutFile {
            path: "LICENSE".into(),
            data: builtin::LICENSE_APACHE.as_bytes().to_vec(),
            exec: false,
        }),
        License::Both => {
            add_text(&mut files, "LICENSE-MIT", builtin::LICENSE_MIT)?;
            files.push(OutFile {
                path: "LICENSE-APACHE".into(),
                data: builtin::LICENSE_APACHE.as_bytes().to_vec(),
                exec: false,
            });
        }
    }

    write_project(&target, &files)?;
    println!(
        "{} Created {} {}",
        term::ok_mark(),
        term::bold(&plan.name),
        term::dim(format!("({} files in {})", files.len(), target.display()))
    );

    if plan.git {
        init_git(&target, files.iter().any(|f| f.path == "gradlew"));
    }

    let (build_cmd, build_cmd_win) =
        if is_gradle { ("./gradlew build", "gradlew.bat build") } else { ("mvn verify", "mvn verify") };
    println!("\nNext steps:");
    println!("  cd {}", shell_quote(&plan.name));
    if cfg!(windows) {
        println!("  {build_cmd_win}");
    } else {
        println!("  {build_cmd}   {}", term::dim(format!("(Windows: {build_cmd_win})")));
    }
    println!("  kiln add <dependency>");
    Ok(())
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        s.to_string()
    } else {
        format!("\"{s}\"")
    }
}

/// Writes all files, removing the partially created project directory on failure.
fn write_project(target: &Path, files: &[OutFile]) -> Result<()> {
    let existed = target.exists();
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(target).with_context(|| format!("could not create {}", target.display()))?;
        for f in files {
            let dest = f.path.split('/').fold(target.to_path_buf(), |acc, c| acc.join(c));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;
            }
            std::fs::write(&dest, &f.data).with_context(|| format!("could not write {}", dest.display()))?;
            #[cfg(unix)]
            if f.exec {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    })();
    if result.is_err() && !existed {
        let _ = std::fs::remove_dir_all(target);
    }
    result
}

fn git(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn init_git(dir: &Path, wrapper: bool) {
    if !git(dir, &["init", "-b", "main"])
        && !(git(dir, &["init"]) && git(dir, &["symbolic-ref", "HEAD", "refs/heads/main"]))
    {
        eprintln!("{} could not run `git init` (is git installed?)", term::warn_mark());
        return;
    }
    git(dir, &["add", "-A"]);
    if wrapper {
        // Keep the executable bit even when created on Windows.
        git(dir, &["update-index", "--chmod=+x", "gradlew"]);
    }
    if git(dir, &["commit", "-q", "-m", "chore: initial commit"]) {
        println!("{} Initialized git repository with an initial commit", term::ok_mark());
    } else {
        eprintln!(
            "{} created the repository but could not commit (set user.name and user.email); files are staged",
            term::warn_mark()
        );
    }
}

#[allow(dead_code)]
fn _registry_type(_: &Registry) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names() {
        assert!(valid_project_name("my-plugin").is_ok());
        assert!(valid_project_name("Müller_App").is_ok());
        for bad in ["", "a b", "a/b", "..", "-x", "a\\b", "CON"] {
            assert!(valid_project_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn package_names() {
        assert_eq!(package_segment("my-cool_plugin"), "mycoolplugin");
        assert_eq!(package_segment("2fast"), "p2fast");
        assert_eq!(package_segment("class"), "class_");
        assert_eq!(package_segment("---"), "app");
        assert_eq!(sanitize_group("Com.Example.Foo-Bar"), "com.example.foobar");
        assert_eq!(sanitize_group(""), "com.example");
        assert_eq!(sanitize_group("my.int.pkg"), "my.int_.pkg");
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_str("a \"b\" $c"), "a \\\"b\\\" \\$c");
        assert_eq!(yaml_quote("it's"), "'it''s'");
        assert_eq!(plugin_id("My Plugin!"), "my-plugin");
        assert_eq!(plugin_id("9lives"), "p9lives");
    }

    #[test]
    fn licenses() {
        assert_eq!(parse_license("MIT"), License::Mit);
        assert_eq!(parse_license("mit-or-apache-2.0"), License::Both);
        assert_eq!(parse_license("none"), License::None);
    }
}
