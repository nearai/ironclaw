use std::fs;
use std::io::Write;
use std::path::Path;

use clap::Args;
use ironclaw_reborn_config::REBORN_CONFIG_API_VERSION;

use crate::context::RebornCliContext;

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
        let home_path = home.path();
        fs::create_dir_all(home_path).map_err(|error| {
            anyhow::anyhow!("create reborn home {}: {error}", home_path.display())
        })?;

        let config_path = home.config_file_path();
        let providers_path = home.providers_file_path();
        preflight_targets(
            [
                (&config_path, "config.toml"),
                (&providers_path, "providers.json"),
            ],
            self.force,
        )?;

        write_atomic(&config_path, &config_stub(), self.force, "config.toml")?;
        write_atomic(
            &providers_path,
            PROVIDERS_STUB,
            self.force,
            "providers.json",
        )?;

        println!("wrote: {}", config_path.display());
        println!("wrote: {}", providers_path.display());
        println!();
        println!("edit them, then run `ironclaw-reborn run`.");
        Ok(())
    }
}

fn preflight_targets<const N: usize>(
    targets: [(&Path, &'static str); N],
    force: bool,
) -> anyhow::Result<()> {
    if force {
        return Ok(());
    }
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

fn write_atomic(
    path: &Path,
    contents: &str,
    force: bool,
    label: &'static str,
) -> anyhow::Result<()> {
    if path.exists() && !force {
        anyhow::bail!(
            "{label} already exists at {}; pass --force to overwrite",
            path.display()
        );
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| anyhow::anyhow!("create temp file in {}: {error}", parent.display()))?;
    tmp.write_all(contents.as_bytes())
        .map_err(|error| anyhow::anyhow!("write {}: {error}", tmp.path().display()))?;
    tmp.flush()
        .map_err(|error| anyhow::anyhow!("flush {}: {error}", tmp.path().display()))?;

    if force {
        tmp.persist(path).map_err(|error| {
            anyhow::anyhow!(
                "persist {} -> {}: {}",
                error.file.path().display(),
                path.display(),
                error.error
            )
        })?;
    } else {
        tmp.persist_noclobber(path).map_err(|error| {
            anyhow::anyhow!(
                "persist {} -> {}: {}",
                error.file.path().display(),
                path.display(),
                error.error
            )
        })?;
    }
    Ok(())
}

/// Build the commented stub TOML with the current API version baked in.
fn config_stub() -> String {
    format!(
        r#"# IronClaw Reborn boot configuration.
#
# Layout:
#   - This file (config.toml) carries the SELECTION layer:
#     identity, policy, drivers, runner timing, and LLM-slot
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
# Composition profile. One of: local-dev, production, migration-dry-run.
# Today only local-dev is wired end-to-end.
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
