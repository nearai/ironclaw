mod bootstrap_env;
mod cli;
mod commands;
mod context;
mod dto;
mod file_write;
mod first_party;
mod operator_env;
mod render;
mod runtime;
mod serve_invocation;
mod webui_token;

fn main() -> anyhow::Result<()> {
    bootstrap_env::load();
    cli::run()
}
