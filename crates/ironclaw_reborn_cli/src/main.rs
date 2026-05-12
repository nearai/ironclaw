mod cli;
mod commands;
mod context;

fn main() -> anyhow::Result<()> {
    cli::run()
}
