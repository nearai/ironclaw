use std::io::{self, Write};

use anyhow::{Context, anyhow};
use clap::{Args, Subcommand};
use ironclaw_reborn_composition::host_api::{TenantId, UserId};
use ironclaw_reborn_composition::{
    LocalAccessState, StrandedTrigger, TriggerAccessRepairAction, TriggerAccessRepairReport,
    repair_local_trigger_access,
};

use crate::context::RebornCliContext;

/// Inspect and repair local-dev trigger-fire access (#4992).
#[derive(Debug, Args)]
pub(crate) struct TriggersCommand {
    #[command(subcommand)]
    command: TriggersSubcommand,
}

#[derive(Debug, Subcommand)]
enum TriggersSubcommand {
    /// Report (and optionally repair) scheduled triggers whose creator lacks
    /// active local-dev fire access for the trigger's exact scope.
    RepairAccess(RepairAccessCommand),
}

#[derive(Debug, Args)]
pub(crate) struct RepairAccessCommand {
    /// Seed exact-scope active access for each stranded creator. Idempotent and
    /// non-reactivating: a deliberately revoked (inactive) row stays revoked, so
    /// reseed only recovers never-seeded / DB-drop strands.
    #[arg(long = "reseed")]
    reseed: bool,

    /// Reassign each stranded trigger to this user id, then seed access. Mutates
    /// persisted `trigger_records.creator_user_id`.
    #[arg(long = "reassign-to", value_name = "USER_ID")]
    reassign_to: Option<String>,

    /// Reassign each stranded trigger to the single active SSO-admitted owner,
    /// then seed access. Fails if there is not exactly one such owner.
    #[arg(long = "reassign-to-current-sso-owner")]
    reassign_to_current_sso_owner: bool,

    /// Skip the confirmation prompt for mutating actions.
    #[arg(long = "yes", short = 'y')]
    yes: bool,
}

impl TriggersCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        match self.command {
            TriggersSubcommand::RepairAccess(command) => command.execute(context),
        }
    }
}

impl RepairAccessCommand {
    fn resolve_action(&self) -> anyhow::Result<TriggerAccessRepairAction> {
        let reassign_flags = usize::from(self.reassign_to.is_some())
            + usize::from(self.reassign_to_current_sso_owner);
        if reassign_flags > 1 {
            return Err(anyhow!(
                "pass at most one of --reassign-to or --reassign-to-current-sso-owner"
            ));
        }
        if self.reseed && reassign_flags > 0 {
            return Err(anyhow!(
                "--reseed cannot be combined with a --reassign-to* action"
            ));
        }
        if let Some(raw) = &self.reassign_to {
            let user_id = UserId::new(raw)
                .map_err(|err| anyhow!("--reassign-to `{raw}` is not a valid user id: {err}"))?;
            return Ok(TriggerAccessRepairAction::Reassign(user_id));
        }
        if self.reassign_to_current_sso_owner {
            return Ok(TriggerAccessRepairAction::ReassignToCurrentSsoOwner);
        }
        if self.reseed {
            return Ok(TriggerAccessRepairAction::Reseed);
        }
        Ok(TriggerAccessRepairAction::Report)
    }

    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let action = self.resolve_action()?;
        let boot_config = context.boot_config();

        // Tenant id is host-trusted operator config, never browser-influenced.
        // Mirror the `serve`/`run` fallback to `reborn-cli`.
        let config_file =
            ironclaw_reborn_config::RebornConfigFile::load(&boot_config.home().config_file_path())
                .map_err(anyhow::Error::from)?;
        let tenant_raw = config_file
            .as_ref()
            .and_then(|file| file.identity.as_ref())
            .and_then(|identity| identity.tenant.as_deref())
            .unwrap_or("reborn-cli");
        let tenant_id = TenantId::new(tenant_raw)
            .map_err(|err| anyhow!("[identity].tenant `{tenant_raw}` is invalid: {err}"))?;

        let db_path = boot_config
            .home()
            .path()
            .join("local-dev")
            .join("reborn-local-dev.db");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime for `triggers repair-access`")?;

        rt.block_on(async move {
            // Always report the current strand set first so the operator sees
            // exactly what a mutating action would touch.
            let report = repair_local_trigger_access(
                &db_path,
                &tenant_id,
                TriggerAccessRepairAction::Report,
            )
            .await
            .context("failed to inspect local trigger-fire access")?;
            print_report(&report);

            if matches!(action, TriggerAccessRepairAction::Report) {
                return Ok(());
            }
            if report.stranded.is_empty() {
                println!("\nNothing to repair — every trigger creator has active access.");
                return Ok(());
            }
            if !self.yes && !confirm_action(&action, report.stranded.len())? {
                println!("Aborted; no changes made.");
                return Ok(());
            }

            let applied = repair_local_trigger_access(&db_path, &tenant_id, action)
                .await
                .context("failed to repair local trigger-fire access")?;
            print_apply_summary(&applied);
            Ok(())
        })
    }
}

fn print_report(report: &TriggerAccessRepairReport) {
    println!(
        "Scanned {} trigger(s); {} stranded (creator lacks active access).",
        report.total_triggers,
        report.stranded.len()
    );
    for trigger in &report.stranded {
        println!("{}", format_stranded(trigger));
    }
}

fn format_stranded(trigger: &StrandedTrigger) -> String {
    let scope = match (&trigger.agent_id, &trigger.project_id) {
        (Some(agent), Some(project)) => format!("agent={agent} project={project}"),
        (Some(agent), None) => format!("agent={agent} project=-"),
        (None, Some(project)) => format!("agent=- project={project}"),
        (None, None) => "agent=- project=-".to_string(),
    };
    let access = match trigger.access_state {
        LocalAccessState::Absent => "absent (will self-heal on next fire; reseed to pre-grant)",
        LocalAccessState::Revoked => "revoked (intentionally deactivated; reseed will NOT restore)",
        LocalAccessState::Active => "active",
    };
    format!(
        "  - {id} \"{name}\" [{state}] creator={creator} {scope} access={access}",
        id = trigger.trigger_id,
        name = trigger.name,
        state = trigger.trigger_state,
        creator = trigger.creator_user_id,
    )
}

fn confirm_action(action: &TriggerAccessRepairAction, stranded: usize) -> anyhow::Result<bool> {
    let prompt = match action {
        TriggerAccessRepairAction::Reseed => {
            format!("Seed active access for {stranded} stranded trigger creator(s)? [y/N] ")
        }
        TriggerAccessRepairAction::Reassign(user_id) => format!(
            "Reassign {stranded} stranded trigger(s) to `{}` and seed access? \
             This rewrites persisted trigger ownership. [y/N] ",
            user_id.as_str()
        ),
        TriggerAccessRepairAction::ReassignToCurrentSsoOwner => format!(
            "Reassign {stranded} stranded trigger(s) to the current SSO owner and seed access? \
             This rewrites persisted trigger ownership. [y/N] "
        ),
        TriggerAccessRepairAction::Report => return Ok(true),
    };
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read confirmation from stdin")?;
    let answer = answer.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn print_apply_summary(report: &TriggerAccessRepairReport) {
    if report.reseeded > 0 {
        println!(
            "\nReseeded active access for {} creator(s).",
            report.reseeded
        );
    }
    if report.reassigned > 0 {
        let target = report.reassigned_to.as_deref().unwrap_or("(unknown)");
        println!(
            "\nReassigned {} trigger(s) to `{target}` and seeded access.",
            report.reassigned
        );
    }
}
