use clap::{Args, Subcommand};
use ironclaw_reborn::ModelSlot;

#[derive(Debug, Args)]
pub(crate) struct ModelsCommand {
    #[command(subcommand)]
    command: ModelsSubcommand,
}

#[derive(Debug, Subcommand)]
enum ModelsSubcommand {
    /// List Reborn model purpose slots.
    List(ModelsListCommand),
    /// Show Reborn model route status.
    Status(ModelsStatusCommand),
}

#[derive(Debug, Args)]
struct ModelsListCommand {
    /// Output model slots as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ModelsStatusCommand {
    /// Output model status as JSON.
    #[arg(long)]
    json: bool,
}

impl ModelsCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            ModelsSubcommand::List(command) => command.execute(),
            ModelsSubcommand::Status(command) => command.execute(),
        }
    }
}

impl ModelsListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let slots = ModelSlot::all();

        if self.json {
            let slots = slots
                .iter()
                .map(|slot| serde_json::json!({ "slot": slot.as_str() }))
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::json!({
                    "slots": slots,
                    "routes": "not-configured",
                    "v1_state": "not-used",
                })
            );
            return Ok(());
        }

        println!("IronClaw Reborn model slots");
        for slot in slots {
            println!("- {}", slot);
        }
        println!("routes: not-configured");
        println!("v1_state: not-used");
        Ok(())
    }
}

impl ModelsStatusCommand {
    fn execute(self) -> anyhow::Result<()> {
        let slots = ModelSlot::all();

        if self.json {
            let mut slot_status = serde_json::Map::new();
            for slot in slots {
                slot_status.insert(
                    slot.as_str().to_string(),
                    serde_json::Value::String("not-configured".to_string()),
                );
            }
            println!(
                "{}",
                serde_json::json!({
                    "routes": "not-configured",
                    "slots": slot_status,
                    "v1_state": "not-used",
                })
            );
            return Ok(());
        }

        println!("IronClaw Reborn model status");
        println!("routes: not-configured");
        for slot in slots {
            println!("{}: not-configured", slot);
        }
        println!("v1_state: not-used");
        Ok(())
    }
}
