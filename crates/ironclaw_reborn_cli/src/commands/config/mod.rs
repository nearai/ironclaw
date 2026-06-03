use clap::{Args, Subcommand};

use crate::context::RebornCliContext;
use crate::dto::{ConfigEntry, ConfigGetDto, ConfigListDto, ConfigValue};
use crate::render::{self, OutputMode};

mod init;

#[derive(Debug, Args)]
pub(crate) struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Show resolved Reborn configuration paths without creating state.
    Path(ConfigPathCommand),
    /// Write a commented stub `config.toml` and `providers.json` into
    /// the Reborn home directory. Refuses to clobber unless --force.
    Init(init::ConfigInitCommand),
    /// List all configuration keys and their values.
    List(ConfigListCommand),
    /// Get a single configuration value by dot-separated key.
    Get(ConfigGetCommand),
}

#[derive(Debug, Args)]
struct ConfigPathCommand;

#[derive(Debug, Args)]
struct ConfigListCommand {
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigGetCommand {
    /// Dot-separated config key (e.g. boot.profile, llm.default.model).
    key: String,
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl ConfigCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        match self.command {
            ConfigSubcommand::Path(command) => command.execute(context),
            ConfigSubcommand::Init(command) => command.execute(context),
            ConfigSubcommand::List(command) => command.execute(context),
            ConfigSubcommand::Get(command) => command.execute(context),
        }
    }
}

impl ConfigPathCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let report =
            ironclaw_reborn_config::RebornDoctorReport::from_config(context.boot_config().clone());
        let home = context.boot_config().home();

        let config_path = home.config_file_path();
        let providers_path = home.providers_file_path();
        let exists = |path: &std::path::Path| {
            if path.exists() {
                "present"
            } else {
                "absent (optional; falls back to defaults)"
            }
        };

        println!("IronClaw Reborn config path");
        println!("reborn_home: {}", report.home_path().display());
        println!("home_source: {}", report.home_source_label());
        println!("profile: {}", report.profile());
        println!(
            "config_file: {} ({})",
            config_path.display(),
            exists(&config_path)
        );
        println!(
            "providers: {} ({})",
            providers_path.display(),
            exists(&providers_path)
        );
        println!("v1_state: {}", report.v1_state());
        Ok(())
    }
}

impl ConfigListCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_config_list_dto(&context)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

impl ConfigGetCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_config_get_dto(&context, &self.key)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

fn build_config_list_dto(context: &RebornCliContext) -> anyhow::Result<ConfigListDto> {
    let config_path = context.boot_config().home().config_file_path();
    let config = load_config_file(context)?;
    let entries = flatten_config(&config);
    Ok(ConfigListDto {
        config_file: config_path,
        entries,
    })
}

fn build_config_get_dto(context: &RebornCliContext, key: &str) -> anyhow::Result<ConfigGetDto> {
    let config = load_config_file(context)?;
    let entries = flatten_config(&config);
    let value = entries
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.value)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown config key: {key}\nRun `ironclaw-reborn config list` to see all keys"
            )
        })?;
    Ok(ConfigGetDto {
        key: key.to_string(),
        value,
    })
}

fn load_config_file(
    context: &RebornCliContext,
) -> anyhow::Result<ironclaw_reborn_config::RebornConfigFile> {
    let config_path = context.boot_config().home().config_file_path();
    Ok(ironclaw_reborn_config::RebornConfigFile::load(&config_path)?.unwrap_or_default())
}

fn flatten_config(config: &ironclaw_reborn_config::RebornConfigFile) -> Vec<ConfigEntry> {
    let mut entries = Vec::new();

    entries.push(entry(
        "api_version",
        config.api_version.as_deref().map(ConfigValue::from_str),
    ));

    // boot
    let boot = config.boot.as_ref();
    entries.push(entry(
        "boot.profile",
        boot.and_then(|b| b.profile.as_deref())
            .map(ConfigValue::from_str),
    ));

    // identity
    let identity = config.identity.as_ref();
    entries.push(entry(
        "identity.tenant",
        identity
            .and_then(|i| i.tenant.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "identity.default_agent",
        identity
            .and_then(|i| i.default_agent.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "identity.default_owner",
        identity
            .and_then(|i| i.default_owner.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "identity.default_project",
        identity
            .and_then(|i| i.default_project.as_deref())
            .map(ConfigValue::from_str),
    ));

    // policy
    let policy = config.policy.as_ref();
    entries.push(entry(
        "policy.deployment_mode",
        policy
            .and_then(|p| p.deployment_mode.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "policy.default_profile",
        policy
            .and_then(|p| p.default_profile.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "policy.default_approval_policy",
        policy
            .and_then(|p| p.default_approval_policy.as_deref())
            .map(ConfigValue::from_str),
    ));

    // drivers
    let drivers = config.drivers.as_ref();
    entries.push(entry(
        "drivers.default",
        drivers
            .and_then(|d| d.default.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "drivers.additional",
        drivers.and_then(|d| d.additional.as_ref().map(|v| ConfigValue::List(v.clone()))),
    ));

    // harness
    entries.push(entry(
        "harness.id",
        config
            .harness
            .as_ref()
            .and_then(|h| h.id.as_deref())
            .map(ConfigValue::from_str),
    ));

    // runner
    let runner = config.runner.as_ref();
    entries.push(entry(
        "runner.heartbeat_interval_secs",
        runner.and_then(|r| r.heartbeat_interval_secs.map(ConfigValue::Integer)),
    ));
    entries.push(entry(
        "runner.poll_interval_ms",
        runner.and_then(|r| r.poll_interval_ms.map(ConfigValue::Integer)),
    ));

    // skills
    entries.push(entry(
        "skills.regex_activation_enabled",
        config
            .skills
            .as_ref()
            .and_then(|s| s.regex_activation_enabled.map(ConfigValue::Bool)),
    ));

    // llm slots
    if let Some(llm) = &config.llm {
        for (slot, selection) in llm {
            entries.push(entry(
                &format!("llm.{slot}.provider_id"),
                selection.provider_id.as_deref().map(ConfigValue::from_str),
            ));
            entries.push(entry(
                &format!("llm.{slot}.model"),
                selection.model.as_deref().map(ConfigValue::from_str),
            ));
            entries.push(entry(
                &format!("llm.{slot}.api_key_env"),
                selection.api_key_env.as_deref().map(ConfigValue::from_str),
            ));
            entries.push(entry(
                &format!("llm.{slot}.base_url"),
                selection.base_url.as_deref().map(ConfigValue::from_str),
            ));
        }
        if !llm.contains_key("default") {
            for field in ["provider_id", "model", "api_key_env", "base_url"] {
                entries.push(entry(&format!("llm.default.{field}"), None));
            }
        }
    } else {
        for field in ["provider_id", "model", "api_key_env", "base_url"] {
            entries.push(entry(&format!("llm.default.{field}"), None));
        }
    }

    // webui
    let webui = config.webui.as_ref();
    entries.push(entry(
        "webui.listen_host",
        webui
            .and_then(|w| w.listen_host.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "webui.listen_port",
        webui.and_then(|w| w.listen_port.map(|p| ConfigValue::Integer(u64::from(p)))),
    ));
    entries.push(entry(
        "webui.env_token_var",
        webui
            .and_then(|w| w.env_token_var.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "webui.env_user_id_var",
        webui
            .and_then(|w| w.env_user_id_var.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "webui.allowed_origins",
        webui.and_then(|w| {
            w.allowed_origins
                .as_ref()
                .map(|v| ConfigValue::List(v.clone()))
        }),
    ));
    entries.push(entry(
        "webui.csp_header_override",
        webui
            .and_then(|w| w.csp_header_override.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "webui.max_body_bytes_fallback",
        webui.and_then(|w| w.max_body_bytes_fallback.map(ConfigValue::Integer)),
    ));
    entries.push(entry(
        "webui.canonical_host",
        webui
            .and_then(|w| w.canonical_host.as_deref())
            .map(ConfigValue::from_str),
    ));

    // budget
    let budget = config.budget.as_ref();
    entries.push(entry(
        "budget.user_daily_usd",
        budget.and_then(|b| b.user_daily_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.project_daily_usd",
        budget.and_then(|b| b.project_daily_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.mission_per_tick_usd",
        budget.and_then(|b| b.mission_per_tick_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.heartbeat_per_tick_usd",
        budget.and_then(|b| b.heartbeat_per_tick_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.routine_lightweight_usd",
        budget.and_then(|b| b.routine_lightweight_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.routine_standard_usd",
        budget.and_then(|b| b.routine_standard_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.background_job_default_usd",
        budget.and_then(|b| b.background_job_default_usd.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.default_tz",
        budget
            .and_then(|b| b.default_tz.as_deref())
            .map(ConfigValue::from_str),
    ));
    entries.push(entry(
        "budget.warn_at",
        budget.and_then(|b| b.warn_at.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.pause_at",
        budget.and_then(|b| b.pause_at.map(ConfigValue::Float)),
    ));
    entries.push(entry(
        "budget.overestimate_factor",
        budget.and_then(|b| b.overestimate_factor.map(ConfigValue::Float)),
    ));

    entries
}

fn entry(key: &str, value: Option<ConfigValue>) -> ConfigEntry {
    ConfigEntry {
        key: key.to_string(),
        value,
    }
}

impl ConfigValue {
    fn from_str(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_reborn_config::RebornConfigFile;

    #[test]
    fn flatten_empty_config_has_all_keys() {
        let config = RebornConfigFile::default();
        let entries = flatten_config(&config);
        assert!(entries.iter().any(|e| e.key == "api_version"));
        assert!(entries.iter().any(|e| e.key == "boot.profile"));
        assert!(entries.iter().any(|e| e.key == "identity.tenant"));
        assert!(entries.iter().any(|e| e.key == "llm.default.provider_id"));
        assert!(entries.iter().any(|e| e.key == "webui.listen_port"));
        assert!(entries.iter().any(|e| e.key == "budget.user_daily_usd"));
        for entry in &entries {
            assert!(entry.value.is_none(), "key {} should be unset", entry.key);
        }
    }

    #[test]
    fn flatten_populated_config() {
        let toml = r#"
api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "test-operator"

[runner]
heartbeat_interval_secs = 10
poll_interval_ms = 500

[llm.default]
provider_id = "openai"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"

[budget]
user_daily_usd = 5.0
"#;
        let config = RebornConfigFile::parse_text(toml, std::path::Path::new("/test/config.toml"))
            .expect("must parse");
        let entries = flatten_config(&config);

        let find = |key: &str| entries.iter().find(|e| e.key == key).expect(key);

        assert!(matches!(
            find("api_version").value,
            Some(ConfigValue::String(ref s)) if s == "ironclaw.runtime/v1"
        ));
        assert!(matches!(
            find("boot.profile").value,
            Some(ConfigValue::String(ref s)) if s == "local-dev"
        ));
        assert!(matches!(
            find("identity.default_owner").value,
            Some(ConfigValue::String(ref s)) if s == "test-operator"
        ));
        assert!(find("identity.tenant").value.is_none());
        assert!(matches!(
            find("runner.heartbeat_interval_secs").value,
            Some(ConfigValue::Integer(10))
        ));
        assert!(matches!(
            find("llm.default.provider_id").value,
            Some(ConfigValue::String(ref s)) if s == "openai"
        ));
        assert!(matches!(
            find("budget.user_daily_usd").value,
            Some(ConfigValue::Float(v)) if (v - 5.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn config_get_unknown_key_errors() {
        let config = RebornConfigFile::default();
        let entries = flatten_config(&config);
        let result = entries.iter().find(|e| e.key == "nonexistent.key");
        assert!(result.is_none());
    }

    #[test]
    fn flatten_multi_slot_llm() {
        let toml = r#"
[llm.default]
provider_id = "openai"

[llm.mission]
provider_id = "anthropic"
model = "claude-3-5-sonnet-latest"
"#;
        let config = RebornConfigFile::parse_text(toml, std::path::Path::new("/test/config.toml"))
            .expect("must parse");
        let entries = flatten_config(&config);

        let find = |key: &str| entries.iter().find(|e| e.key == key).expect(key);
        assert!(matches!(
            find("llm.default.provider_id").value,
            Some(ConfigValue::String(ref s)) if s == "openai"
        ));
        assert!(matches!(
            find("llm.mission.provider_id").value,
            Some(ConfigValue::String(ref s)) if s == "anthropic"
        ));
        assert!(matches!(
            find("llm.mission.model").value,
            Some(ConfigValue::String(ref s)) if s == "claude-3-5-sonnet-latest"
        ));
    }
}
