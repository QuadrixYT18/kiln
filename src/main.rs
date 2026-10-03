mod check;
mod cli;
mod commands;
mod context;
mod project;
mod registry;
mod templates;
mod util;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::context::Ctx;
use crate::util::term;

async fn run(cli: Cli) -> anyhow::Result<()> {
    let ctx = Ctx::new(&cli)?;
    match cli.command {
        Command::New(args) => commands::new::run(&ctx, args).await,
        Command::Add(args) => commands::add::run(&ctx, args).await,
        Command::Outdated(args) => commands::outdated::run(&ctx, args).await,
        Command::Update(args) => commands::update::run(&ctx, args).await,
        Command::Remove(args) => commands::remove::run(&ctx, args).await,
        Command::Template(cmd) => commands::template::run(&ctx, cmd).await,
        Command::Completions { shell } => {
            commands::completions::run(shell);
            Ok(())
        }
        Command::Cache(cmd) => commands::cache::run(&ctx, cmd),
    }
}

fn is_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|e| {
        matches!(
            e.downcast_ref::<inquire::InquireError>(),
            Some(inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted)
        )
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    term::init(cli.color, cli.yes);
    term::set_verbose(cli.verbose);
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: could not start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if is_cancelled(&err) => {
            eprintln!("{}", term::dim("cancelled"));
            ExitCode::from(130)
        }
        Err(err) => {
            eprintln!("{} {err}", term::err_mark());
            for cause in err.chain().skip(1) {
                eprintln!("  {} {cause}", term::dim("caused by:"));
            }
            ExitCode::FAILURE
        }
    }
}
