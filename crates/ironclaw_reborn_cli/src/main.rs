mod cli;
mod commands;
mod context;
mod runtime;

fn main() -> anyhow::Result<()> {
    // Mirror the v1 binary's behavior so dev workflows can keep LLM
    // keys / base URLs in `.env`. Silent on missing file — production
    // hosts use shell-exported env or systemd unit env, not `.env`.
    let _ = dotenvy::dotenv();
    cli::run()
}
