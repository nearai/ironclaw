mod cli;
mod commands;
mod context;
mod runtime;

fn main() -> anyhow::Result<()> {
    cli::run()
}
