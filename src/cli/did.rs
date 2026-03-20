//! CLI commands for instance DID inspection.

use clap::Subcommand;

/// DID-related CLI commands.
#[derive(Subcommand, Debug, Clone, Copy)]
pub enum DidCommand {
    /// Show the current instance DID.
    Show,
    /// Print the current instance DID document as JSON.
    Document,
}

/// Run the DID CLI subcommand.
pub async fn run_did_command(cmd: DidCommand) -> anyhow::Result<()> {
    let identity = crate::did::load_or_create_default().map_err(|e| anyhow::anyhow!("{e}"))?;

    match cmd {
        DidCommand::Show => {
            println!("{}", identity.did());
        }
        DidCommand::Document => {
            println!("{}", serde_json::to_string_pretty(&identity.document())?);
        }
    }

    Ok(())
}
