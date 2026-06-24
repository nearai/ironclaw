use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::{REBORN_CONFIG_API_VERSION, RebornHome};

use crate::context::RebornCliContext;
use crate::file_write::{FileWriteAction, write_atomic};

/// Write a commented stub `config.toml` and `providers.json` into the
/// Reborn home directory so an operator has something editable.
///
/// Mirrors v1's `ironclaw config init` ergonomics: refuses to clobber
/// existing files unless `--force` is given. Both files are written
/// atomically (write to `.tmp`, rename) so a partial write never
/// leaves an unreadable config on the next boot.
#[derive(Debug, Args)]
pub(crate) struct ConfigInitCommand {
    /// Overwrite existing files.
    #[arg(long = "force")]
    pub force: bool,
}

impl ConfigInitCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let home = context.boot_config().home();
        let outcome =
            write_default_config_files(home, self.force, ExistingConfigPolicy::FailIfPresent)?;

        println!("{}", outcome.config.display_line());
        println!("{}", outcome.providers.display_line());
        println!();
        println!("edit them, then run `ironclaw-reborn run`.");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingConfigPolicy {
    FailIfPresent,
    Preserve,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigFilesWriteOutcome {
    pub(crate) config: ConfigFileWrite,
    pub(crate) providers: ConfigFileWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigFileWrite {
    pub(crate) path: PathBuf,
    pub(crate) action: FileWriteAction,
}

impl ConfigFileWrite {
    pub(crate) fn display_line(&self) -> String {
        format!("{}: {}", self.action, self.path.display())
    }
}

pub(crate) fn write_default_config_files(
    home: &RebornHome,
    force: bool,
    existing_policy: ExistingConfigPolicy,
) -> anyhow::Result<ConfigFilesWriteOutcome> {
    let home_path = home.path();
    fs::create_dir_all(home_path)
        .map_err(|error| anyhow::anyhow!("create reborn home {}: {error}", home_path.display()))?;

    let config_path = home.config_file_path();
    let providers_path = home.providers_file_path();
    if existing_policy == ExistingConfigPolicy::FailIfPresent && !force {
        preflight_targets([
            (&config_path, "config.toml"),
            (&providers_path, "providers.json"),
        ])?;
    }

    let config =
        if existing_policy == ExistingConfigPolicy::Preserve && config_path.exists() && !force {
            ConfigFileWrite {
                path: config_path,
                action: FileWriteAction::Preserved,
            }
        } else {
            let action = write_atomic(&config_path, &config_stub(), force, "config.toml")?;
            ConfigFileWrite {
                path: config_path,
                action,
            }
        };

    let providers =
        if existing_policy == ExistingConfigPolicy::Preserve && providers_path.exists() && !force {
            ConfigFileWrite {
                path: providers_path,
                action: FileWriteAction::Preserved,
            }
        } else {
            let action = write_atomic(&providers_path, PROVIDERS_STUB, force, "providers.json")?;
            ConfigFileWrite {
                path: providers_path,
                action,
            }
        };

    Ok(ConfigFilesWriteOutcome { config, providers })
}

fn preflight_targets<const N: usize>(targets: [(&Path, &'static str); N]) -> anyhow::Result<()> {
    let existing = targets
        .into_iter()
        .filter(|(path, _)| path.exists())
        .map(|(path, label)| format!("{label} already exists at {}", path.display()))
        .collect::<Vec<_>>();
    if existing.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}; pass --force to overwrite", existing.join("; "))
    }
}

/// Build the commented stub TOML with the current API version baked in.
fn config_stub() -> String {
    format!(
        r#"# IronClaw Reborn boot configuration.
#
# Layout:
#   - This file (config.toml) carries the SELECTION layer:
#     identity, policy, drivers, runner timing, skills, and LLM-slot
#     selection by id.
#   - providers.json (next to this file) carries the CATALOG layer:
#     provider definitions known to the binary. The compiled-in
#     defaults are appended with the entries in this file; later
#     entries override earlier ones by id/alias.
#   - Secrets stay in environment variables. Reference them by NAME
#     here (e.g. `api_key_env = "OPENAI_API_KEY"`); never paste the
#     value itself. Pasting a value is rejected at parse time.
#
# Precedence on each field:
#   compiled defaults < this file < env vars < CLI flags.
#
# Regenerate with `ironclaw-reborn config init --force`.

api_version = "{api_version}"

[boot]
# Composition profile. One of: local-dev, local-dev-yolo, hosted-single-tenant,
# production, migration-dry-run.
# Today local-dev, local-dev-yolo, and hosted-single-tenant are wired end-to-end.
# local-dev-yolo also requires --confirm-host-access at runtime.
profile = "local-dev"

[identity]
# Owner-user scope this runtime acts under by default. This field is wired today.
default_owner  = "reborn-cli"
# Tenant / agent / project scope land with the identity substrate from epic #3036.
# Leave these commented until then; `run` rejects them in this slice rather
# than silently ignoring operator intent.
# tenant         = "reborn-cli"
# default_agent  = "reborn-cli-agent"
# default_project = "your-project"

# [policy]
# # Policy selection lands with epic #3036. Leave this section commented in
# # this slice; `run` rejects it rather than silently ignoring operator intent.
# deployment_mode         = "local_single_user"
# default_profile         = "local_dev"
# default_approval_policy = "ask_destructive"

# [drivers]
# # Driver selection lands with epic #3036. Leave this section commented in
# # this slice; `run` rejects it rather than silently ignoring operator intent.
# default     = "text_only"
# additional = ["planned"]

# [harness]
# # Active harness lands with epic #3036. Leave this section commented in
# # this slice; `run` rejects it rather than silently ignoring operator intent.
# id = "red-team"

[runner]
heartbeat_interval_secs = 5
poll_interval_ms        = 200

[skills]
# When false, regex activation criteria do not auto-load full skill
# context. Keyword/tag activation and explicit skill mentions such as
# `$code-review` still activate skills.
regex_activation_enabled = true

# [storage]
# # PostgreSQL storage selection for hosted-single-tenant / production. The
# # database URL value is env-only; this file may name the variable but must
# # never contain the raw URL.
# # Managed remote Postgres providers must use TLS, e.g. append
# # `sslmode=require` to IRONCLAW_REBORN_POSTGRES_URL.
# backend = "postgres"
# url_env = "IRONCLAW_REBORN_POSTGRES_URL"
# secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
# # Optional; defaults to 2. Keep below the PostgreSQL server or managed
# # session-pool cap after reserving capacity for restarts/operator sessions.
# pool_max_size = 2

[llm.default]
# LLM slot selection. `provider_id` references an entry in
# providers.json (built-in or user-overlay). `model` / `base_url` /
# `api_key_env` override the catalog defaults for this deployment.
provider_id = "openai"
model       = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"

# [llm.mission]
# # Reserved for the future planned-driver "mission" slot.
# provider_id = "anthropic"
# model       = "claude-3-5-sonnet-latest"
# api_key_env = "ANTHROPIC_API_KEY"

# [slack]
# # Host-beta Slack Events API route for `ironclaw-reborn serve`.
# # Requires a binary built with `--features slack-v2-host-beta`.
# enabled = false
# # Configure Slack app ids, bot token, signing secret, and channel mappings
# # from WebUI channel setup after the server starts.
"#,
        api_version = REBORN_CONFIG_API_VERSION,
    )
}

/// Minimal example overlay for `providers.json` — a tenant-pinned
/// OpenAI-compatible endpoint. Operators are expected to edit / extend
/// or delete. The compiled-in built-in providers (openai, anthropic,
/// ollama, deepseek, gemini, openrouter, …) are always loaded; this
/// file appends and overrides by id/alias.
const PROVIDERS_STUB: &str = r#"[
  {
    "id": "acme-openrouter",
    "aliases": [],
    "protocol": "open_ai_completions",
    "api_key_env": "ACME_OPENROUTER_KEY",
    "api_key_required": true,
    "default_base_url": "https://openrouter.ai/api/v1",
    "default_model": "anthropic/claude-3.5-sonnet",
    "model_env": "ACME_OPENROUTER_MODEL",
    "description": "Tenant-pinned OpenRouter route (example; rename or delete)",
    "setup": {
      "kind": "api_key",
      "secret_name": "llm_acme_openrouter_api_key",
      "key_url": "https://openrouter.ai/keys",
      "display_name": "OpenRouter (Acme)",
      "can_list_models": true
    }
  }
]
"#;
