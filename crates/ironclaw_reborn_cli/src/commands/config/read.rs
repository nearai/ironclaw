use crate::context::RebornCliContext;
use crate::dto::{ConfigEntry, ConfigGetDto, ConfigListDto, ConfigValue};
use crate::render::{Renderable, terminal_safe_text};
use std::io::Write;

pub(super) fn build_config_list_dto(context: &RebornCliContext) -> anyhow::Result<ConfigListDto> {
    let config_path = context.boot_config().home().config_file_path();
    let config = load_config_file(context)?;
    let entries = flatten_config(&config)?;
    Ok(ConfigListDto {
        config_file: config_path,
        entries,
    })
}

pub(super) fn build_config_get_dto(
    context: &RebornCliContext,
    key: &str,
) -> anyhow::Result<ConfigGetDto> {
    // Reuses flatten_config rather than a direct dot-path walker: simpler, and this is a cold CLI path.
    let config = load_config_file(context)?;
    let entries = flatten_config(&config)?;
    let value = entries
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.value)
        .ok_or_else(|| {
            anyhow::anyhow!("unknown config key: {key}\nRun `ironclaw config list` to see all keys")
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

fn flatten_config(
    config: &ironclaw_reborn_config::RebornConfigFile,
) -> anyhow::Result<Vec<ConfigEntry>> {
    use ironclaw_reborn_config::RebornConfigFile;

    // Expand all None sections to Some(Default) so every leaf key appears
    // even when unset. The struct literal means a new field added to
    // RebornConfigFile will fail to compile here until handled — that is
    // the drift prevention the manual approach lacked.
    let mut llm = config.llm.clone().unwrap_or_default();
    llm.entry("default".to_string()).or_default();

    let expanded = RebornConfigFile {
        api_version: config.api_version.clone(),
        boot: Some(config.boot.clone().unwrap_or_default()),
        identity: Some(config.identity.clone().unwrap_or_default()),
        policy: Some(config.policy.clone().unwrap_or_default()),
        drivers: Some(config.drivers.clone().unwrap_or_default()),
        harness: Some(config.harness.clone().unwrap_or_default()),
        runner: Some(config.runner.clone().unwrap_or_default()),
        skills: Some(config.skills.clone().unwrap_or_default()),
        storage: Some(config.storage.clone().unwrap_or_default()),
        llm: Some(llm),
        webui: Some(config.webui.clone().unwrap_or_default()),
        slack: Some(config.slack.clone().unwrap_or_default()),
        telegram: Some(config.telegram.clone().unwrap_or_default()),
        budget: Some(config.budget.clone().unwrap_or_default()),
        trigger_poller: Some(config.trigger_poller.clone().unwrap_or_default()),
    };

    let value = serde_json::to_value(&expanded)?;
    let mut entries = Vec::new();
    collect_leaf_entries(&value, String::new(), &mut entries);
    Ok(entries)
}

fn collect_leaf_entries(value: &serde_json::Value, prefix: String, entries: &mut Vec<ConfigEntry>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_leaf_entries(val, full_key, entries);
            }
        }
        serde_json::Value::Null => {
            entries.push(ConfigEntry {
                key: prefix,
                value: None,
            });
        }
        other => {
            entries.push(ConfigEntry {
                key: prefix,
                value: Some(json_to_config_value(other)),
            });
        }
    }
}

fn json_to_config_value(value: &serde_json::Value) -> ConfigValue {
    match value {
        serde_json::Value::String(s) => ConfigValue::String(s.clone()),
        serde_json::Value::Bool(b) => ConfigValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                ConfigValue::Integer(u)
            } else if let Some(f) = n.as_f64() {
                ConfigValue::Float(f)
            } else {
                ConfigValue::String(n.to_string())
            }
        }
        serde_json::Value::Array(arr) => ConfigValue::List(
            arr.iter()
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string())
                })
                .collect(),
        ),
        other => ConfigValue::String(other.to_string()),
    }
}

impl Renderable for ConfigListDto {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "IronClaw Reborn config ({})", self.config_file.display())?;
        writeln!(w)?;
        for entry in &self.entries {
            match &entry.value {
                Some(value) => writeln!(
                    w,
                    "{:<44} {}",
                    terminal_safe_text(&entry.key),
                    terminal_safe_text(&value.to_string())
                )?,
                None => writeln!(w, "{:<44} (not set)", terminal_safe_text(&entry.key))?,
            }
        }
        Ok(())
    }
}

impl Renderable for ConfigGetDto {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        match &self.value {
            Some(value) => writeln!(w, "{}", terminal_safe_text(&value.to_string())),
            None => writeln!(w, "(not set)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RebornCliContext;
    use ironclaw_reborn_config::RebornConfigFile;

    #[test]
    fn flatten_empty_config_has_all_keys() {
        let config = RebornConfigFile::default();
        let entries = flatten_config(&config).expect("flatten");
        assert!(entries.iter().any(|e| e.key == "api_version"));
        assert!(entries.iter().any(|e| e.key == "boot.profile"));
        assert!(entries.iter().any(|e| e.key == "identity.tenant"));
        assert!(entries.iter().any(|e| e.key == "storage.backend"));
        assert!(entries.iter().any(|e| e.key == "llm.default.provider_id"));
        assert!(entries.iter().any(|e| e.key == "webui.listen_port"));
        assert!(entries.iter().any(|e| e.key == "budget.user_daily_usd"));
        assert!(entries.iter().any(|e| e.key == "slack.enabled"));
        assert!(entries.iter().any(|e| e.key == "slack.team_id"));
        assert!(entries.iter().any(|e| e.key == "trigger_poller.enabled"));
        assert!(
            entries
                .iter()
                .any(|e| e.key == "trigger_poller.poll_interval_secs"),
        );
        for entry in &entries {
            let is_empty_list_default = matches!(
                &entry.value,
                Some(ConfigValue::List(items)) if items.is_empty()
            );
            assert!(
                entry.value.is_none() || is_empty_list_default,
                "key {} should be unset or an empty-list default",
                entry.key
            );
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
        let entries = flatten_config(&config).expect("flatten");

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
        let entries = flatten_config(&config).expect("flatten");
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
        let entries = flatten_config(&config).expect("flatten");

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

    #[test]
    fn flatten_llm_without_default_slot_still_emits_default_placeholders() {
        let toml = r#"
[llm.mission]
provider_id = "anthropic"
model = "claude-3-5-sonnet-latest"
"#;
        let config = RebornConfigFile::parse_text(toml, std::path::Path::new("/test/config.toml"))
            .expect("must parse");
        let entries = flatten_config(&config).expect("flatten");

        let find = |key: &str| entries.iter().find(|e| e.key == key);

        assert!(
            find("llm.default.provider_id").is_some(),
            "llm.default.provider_id must be present as a placeholder"
        );
        assert!(find("llm.default.provider_id").unwrap().value.is_none());
        assert!(find("llm.default.model").unwrap().value.is_none());
        assert!(find("llm.default.api_key_env").unwrap().value.is_none());
        assert!(find("llm.default.base_url").unwrap().value.is_none());

        assert!(matches!(
            find("llm.mission.provider_id").unwrap().value,
            Some(ConfigValue::String(ref s)) if s == "anthropic"
        ));
    }

    #[test]
    fn collect_leaf_entries_array_value_becomes_list() {
        let value = serde_json::json!({
            "tags": ["alpha", "beta"]
        });
        let mut entries = Vec::new();
        collect_leaf_entries(&value, String::new(), &mut entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "tags");
        assert!(
            matches!(
                &entries[0].value,
                Some(ConfigValue::List(items)) if items == &["alpha", "beta"]
            ),
            "expected List([\"alpha\", \"beta\"]), got {:?}",
            entries[0].value
        );
    }

    #[test]
    fn collect_leaf_entries_array_coerces_non_string_elements() {
        let value = serde_json::json!({
            "tags": ["alpha", 42, true, "beta"]
        });
        let mut entries = Vec::new();
        collect_leaf_entries(&value, String::new(), &mut entries);
        assert_eq!(entries.len(), 1);
        assert!(
            matches!(
                &entries[0].value,
                Some(ConfigValue::List(items)) if items == &["alpha", "42", "true", "beta"]
            ),
            "non-string elements should be coerced to strings, got {:?}",
            entries[0].value
        );
    }

    #[test]
    fn json_to_config_value_negative_integer_becomes_float() {
        let val = serde_json::json!(-5);
        let config_val = json_to_config_value(&val);
        assert!(
            matches!(config_val, ConfigValue::Float(f) if (f - (-5.0)).abs() < f64::EPSILON),
            "negative integer should coerce to Float since ConfigValue::Integer is u64"
        );
    }

    #[test]
    fn build_config_get_dto_known_key_none_value_returns_ok_none() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_config_get_dto(&context, "identity.tenant").expect("must succeed");
        assert_eq!(dto.key, "identity.tenant");
        assert!(dto.value.is_none());
    }

    #[test]
    fn build_config_get_dto_known_key_set_value_returns_ok_some() {
        let (_tmp, context) = RebornCliContext::test_context();
        let reborn_home = context.boot_config().home().path();
        std::fs::create_dir_all(reborn_home).expect("create reborn_home");
        std::fs::write(
            reborn_home.join("config.toml"),
            "[boot]\nprofile = \"custom-profile\"\n",
        )
        .expect("write config");

        let dto = build_config_get_dto(&context, "boot.profile").expect("must succeed");
        assert_eq!(dto.key, "boot.profile");
        assert!(
            matches!(dto.value, Some(ConfigValue::String(ref s)) if s == "custom-profile"),
            "expected Some(String(\"custom-profile\")), got {:?}",
            dto.value
        );
    }

    #[test]
    fn build_config_get_dto_unknown_key_returns_err() {
        let (_tmp, context) = RebornCliContext::test_context();
        let result = build_config_get_dto(&context, "nonexistent.key");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unknown config key"),
            "error should mention unknown key: {err_msg}"
        );
    }
}
