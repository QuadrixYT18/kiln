use anyhow::Result;

use crate::cli::CacheCommand;
use crate::context::Ctx;
use crate::util::{paths, term};

pub fn run(ctx: &Ctx, cmd: CacheCommand) -> Result<()> {
    match cmd {
        CacheCommand::Path => println!("{}", paths::cache_dir()?.display()),
        CacheCommand::Clear => {
            let n = ctx.http()?.cache().clear()?;
            println!("{} removed {n} cached response(s)", term::ok_mark());
        }
    }
    Ok(())
}
