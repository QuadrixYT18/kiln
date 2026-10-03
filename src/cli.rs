//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

use crate::util::term::ColorChoice;

const ABOUT: &str = "Set up JVM projects in seconds and manage Gradle/Maven dependencies";

const LONG_ABOUT: &str = "kiln creates new JVM projects from templates and adds, checks and updates \
dependencies in Gradle (Kotlin/Groovy DSL, version catalogs) and Maven projects, \
without reformatting your build files.";

const EXAMPLES: &str = "\
EXAMPLES:
    kiln new my-plugin --template paper --java 21
    kiln add hikari postgres jedis
    kiln outdated
    kiln update --minor --verify
    kiln remove jedis

Run `kiln <command> --help` for details and examples of each command.";

#[derive(Parser, Debug)]
#[command(name = "kiln", version = env!("KILN_LONG_VERSION"), about = ABOUT, long_about = LONG_ABOUT, after_help = EXAMPLES, propagate_version = true)]
pub struct Cli {
    /// Run as if kiln was started in this directory
    #[arg(short = 'C', long = "path", global = true, value_name = "DIR", env = "KILN_PATH")]
    pub path: Option<PathBuf>,

    /// Never touch the network; use cached data only
    #[arg(long, global = true, env = "KILN_OFFLINE")]
    pub offline: bool,

    /// Ignore and do not write the response cache
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Show full diagnostics (complete URLs and error details)
    #[arg(short = 'v', long, global = true, env = "KILN_VERBOSE")]
    pub verbose: bool,

    /// Answer prompts with their defaults (non-interactive)
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// When to use colored output
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto, value_name = "WHEN")]
    pub color: ColorChoice,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new JVM project from a template
    #[command(after_long_help = "\
EXAMPLES:
    kiln new                                   # interactive wizard
    kiln new my-plugin --template paper --java 21
    kiln new api --template backend --framework ktor --docker --db postgres
    kiln new lib --template library --lang kotlin --ci github --license mit
    kiln new tool --template my-saved-template --var author=Jane")]
    New(NewArgs),

    /// Add dependencies (aliases, `group:artifact`, or search terms)
    #[command(after_long_help = "\
EXAMPLES:
    kiln add hikari postgres jedis             # built-in aliases
    kiln add com.google.guava:guava            # explicit coordinates
    kiln add junit mockk --test                # test scope
    kiln add lombok --compile-only
    kiln add gson@2.10.1                       # pin a version
    kiln add okhttp --module :app --dry-run    # show a diff, write nothing")]
    Add(AddArgs),

    /// Show available updates for dependencies and plugins
    #[command(after_long_help = "\
EXAMPLES:
    kiln outdated
    kiln outdated --only-outdated --pre
    kiln outdated --offline                    # use cached version data")]
    Outdated(OutdatedArgs),

    /// Update dependencies and plugins
    #[command(after_long_help = "\
EXAMPLES:
    kiln update                                # pick interactively
    kiln update --patch                        # only patch releases
    kiln update --minor --verify               # build afterwards, offer rollback
    kiln update --all --dry-run                # preview every update, even major ones
    kiln update guava jackson --minor          # only matching dependencies")]
    Update(UpdateArgs),

    /// Remove dependencies from build files and the version catalog
    #[command(after_long_help = "\
EXAMPLES:
    kiln remove jedis
    kiln remove com.google.guava:guava --module :app
    kiln remove hikari --dry-run")]
    Remove(RemoveArgs),

    /// Manage your own project templates
    #[command(subcommand)]
    Template(TemplateCommand),

    /// Generate shell completions
    #[command(after_long_help = "\
EXAMPLES:
    kiln completions bash > ~/.local/share/bash-completion/completions/kiln
    kiln completions zsh > ~/.zfunc/_kiln
    kiln completions fish > ~/.config/fish/completions/kiln.fish
    kiln completions powershell >> $PROFILE")]
    Completions {
        /// Target shell
        shell: Shell,
    },

    /// Inspect or clear the response cache
    #[command(subcommand)]
    Cache(CacheCommand),
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Project name (also the directory name)
    pub name: Option<String>,

    /// Template: paper, velocity, library, backend, empty, or a custom template name
    #[arg(short, long)]
    pub template: Option<String>,

    /// Language for templates that support both
    #[arg(long, value_parser = ["java", "kotlin"])]
    pub lang: Option<String>,

    /// Java version (toolchain)
    #[arg(long, value_name = "N")]
    pub java: Option<u32>,

    /// Group id / base package prefix (default from config, else `com.example`)
    #[arg(long)]
    pub group: Option<String>,

    /// Java package (default: group + sanitized name)
    #[arg(long)]
    pub package: Option<String>,

    /// One-line project description
    #[arg(long)]
    pub description: Option<String>,

    /// Backend framework (backend template)
    #[arg(long, value_parser = ["ktor", "spring"])]
    pub framework: Option<String>,

    /// Use paperweight-userdev (paper template)
    #[arg(long)]
    pub paperweight: bool,

    /// Add Dockerfile and docker-compose.yml
    #[arg(long)]
    pub docker: bool,

    /// Services in docker-compose.yml (postgres, redis); implies --docker
    #[arg(long, value_delimiter = ',', value_parser = ["postgres", "redis"])]
    pub db: Vec<String>,

    /// CI pipeline
    #[arg(long, value_parser = ["github", "gitlab", "none"])]
    pub ci: Option<String>,

    /// Add a Renovate configuration
    #[arg(long)]
    pub renovate: bool,

    /// License file
    #[arg(long, value_parser = ["mit", "apache-2.0", "mit-or-apache-2.0", "none"])]
    pub license: Option<String>,

    /// Skip .editorconfig / .gitignore / .gitattributes / README
    #[arg(long)]
    pub bare: bool,

    /// Do not run `git init` and create the first commit
    #[arg(long)]
    pub no_git: bool,

    /// Set a custom-template placeholder (repeatable): --var key=value
    #[arg(long = "var", value_name = "KEY=VALUE")]
    pub vars: Vec<String>,

    /// Directory to create the project in (default: current directory)
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
#[command(group = clap::ArgGroup::new("scope").multiple(false))]
pub struct AddArgs {
    /// Aliases, `group:artifact[:version]`, `name@version`, or search terms
    #[arg(required = true, value_name = "NAME")]
    pub names: Vec<String>,

    /// Compile/implementation scope (default)
    #[arg(long, group = "scope")]
    pub compile: bool,
    /// Test scope
    #[arg(long, group = "scope")]
    pub test: bool,
    /// Runtime scope
    #[arg(long, group = "scope")]
    pub runtime: bool,
    /// Compile-only (`compileOnly` / Maven `provided`)
    #[arg(long, group = "scope")]
    pub compile_only: bool,
    /// Annotation processor
    #[arg(long, group = "scope")]
    pub annotation_processor: bool,

    /// Allow alpha/beta/RC/milestone/SNAPSHOT versions
    #[arg(long)]
    pub pre: bool,

    /// Module to edit (multi-module projects)
    #[arg(short, long)]
    pub module: Option<String>,

    /// Show a diff without writing anything
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct OutdatedArgs {
    /// Consider pre-release versions
    #[arg(long)]
    pub pre: bool,
    /// Hide dependencies that are up to date
    #[arg(long)]
    pub only_outdated: bool,
}

#[derive(Args, Debug)]
#[command(group = clap::ArgGroup::new("level").multiple(false))]
pub struct UpdateArgs {
    /// Only update these dependencies (name, alias, or group:artifact)
    pub names: Vec<String>,
    /// Apply patch updates only
    #[arg(long, group = "level")]
    pub patch: bool,
    /// Apply patch and minor updates
    #[arg(long, group = "level")]
    pub minor: bool,
    /// Apply every update, including major ones
    #[arg(long, group = "level")]
    pub all: bool,
    /// Consider pre-release versions
    #[arg(long)]
    pub pre: bool,
    /// Show a diff without writing anything
    #[arg(long)]
    pub dry_run: bool,
    /// Run the build afterwards and offer to roll back on failure
    #[arg(long)]
    pub verify: bool,
}

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Dependencies to remove (alias, artifactId, catalog alias, or group:artifact)
    #[arg(required = true, value_name = "NAME")]
    pub names: Vec<String>,
    /// Limit removal to one module
    #[arg(short, long)]
    pub module: Option<String>,
    /// Show a diff without writing anything
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Subcommand, Debug)]
pub enum TemplateCommand {
    /// List built-in and custom templates
    List,
    /// Turn an existing project into a reusable template
    #[command(after_long_help = "\
EXAMPLES:
    kiln template save my-service                # from the current directory
    kiln template save my-service --from ../demo --package com.acme.demo")]
    Save {
        /// Template name
        name: String,
        /// Project directory (default: current directory)
        #[arg(long, value_name = "DIR")]
        from: Option<PathBuf>,
        /// Project name inside the sources (default: directory name)
        #[arg(long)]
        project_name: Option<String>,
        /// Base package to replace with `{{package}}` (default: auto-detected)
        #[arg(long)]
        package: Option<String>,
        /// Overwrite an existing template
        #[arg(long)]
        force: bool,
    },
    /// Delete a custom template
    Remove { name: String },
    /// Print the templates directory
    Path,
}

#[derive(Subcommand, Debug)]
pub enum CacheCommand {
    /// Print the cache directory
    Path,
    /// Delete all cached responses
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
