use std::collections::BTreeSet;

use ironclaw_operator::{RebornProviderAdmin, RebornProviderAdminError, RebornV1State};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

struct RuntimeEnvMaskGuard {
    snapshots: Vec<ironclaw_common::env_helpers::RuntimeEnvSnapshot>,
}

impl Drop for RuntimeEnvMaskGuard {
    fn drop(&mut self) {
        for snapshot in self.snapshots.drain(..).rev() {
            ironclaw_common::env_helpers::restore_runtime_env(snapshot);
        }
    }
}

fn mask_llm_env_for_test() -> RuntimeEnvMaskGuard {
    let registry =
        ironclaw_llm::ProviderRegistry::try_load_from_path(None).expect("builtin registry");
    let mut keys = BTreeSet::from([
        "LLM_BACKEND".to_string(),
        "LLM_MODEL".to_string(),
        "LLM_CHEAP_MODEL".to_string(),
        "LLM_USE_CODEX_AUTH".to_string(),
        "CODEX_AUTH_PATH".to_string(),
        "SMART_ROUTING_CASCADE".to_string(),
    ]);
    for provider in registry.all() {
        keys.insert(provider.model_env.clone());
        if let Some(name) = &provider.api_key_env {
            keys.insert(name.clone());
        }
        if let Some(name) = &provider.base_url_env {
            keys.insert(name.clone());
        }
        if let Some(name) = &provider.extra_headers_env {
            keys.insert(name.clone());
        }
    }

    let snapshots = keys
        .iter()
        .map(|key| ironclaw_common::env_helpers::snapshot_runtime_env(key))
        .collect();
    for key in keys {
        ironclaw_common::env_helpers::mask_runtime_env(&key);
    }
    RuntimeEnvMaskGuard { snapshots }
}

fn admin_for_home(reborn_home: &std::path::Path) -> RebornProviderAdmin {
    let home = RebornHome::resolve_from_env_parts(
        Some(reborn_home.as_os_str().to_os_string()),
        None,
        None,
    )
    .expect("valid reborn home");
    RebornProviderAdmin::new(RebornBootConfig::new(home, RebornProfile::LocalDev))
}

#[test]
fn list_unknown_provider_returns_known_provider_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let admin = admin_for_home(&temp.path().join("reborn-home"));

    let err = admin
        .list(Some("missing-provider"), false)
        .expect_err("unknown provider should reject");

    let RebornProviderAdminError::UnknownProvider {
        provider, known, ..
    } = err
    else {
        panic!("expected unknown provider error");
    };
    assert_eq!(provider, "missing-provider");
    assert!(known.contains(&"openai".to_string()), "known: {known:?}");
}

#[test]
fn set_model_empty_string_returns_invalid_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let admin = admin_for_home(&temp.path().join("reborn-home"));

    for model in ["", "   "] {
        let err = admin
            .set_model(model)
            .expect_err("empty model should reject before config access");
        assert!(matches!(
            err,
            RebornProviderAdminError::InvalidRequest { reason }
                if reason == "model name cannot be empty"
        ));
    }
}

#[test]
fn set_model_reports_active_provider_credential_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"
"#,
    )
    .expect("write config");
    let admin = admin_for_home(&reborn_home);

    let outcome = admin.set_model("gpt-5.3-codex").expect("set model");

    assert_eq!(outcome.provider_id, "openai");
    assert_eq!(outcome.model, "gpt-5.3-codex");
    assert_eq!(outcome.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
    assert!(outcome.api_key_required);
}

#[test]
fn set_provider_nearai_uses_concrete_default_model() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let admin = admin_for_home(&reborn_home);

    let outcome = admin
        .set_provider("nearai", None)
        .expect("set nearai provider");

    assert_eq!(outcome.provider_id, "nearai");
    assert_eq!(outcome.model, ironclaw_llm::DEFAULT_MODEL);
    let config = std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert!(
        config.contains(&format!("model = \"{}\"", ironclaw_llm::DEFAULT_MODEL)),
        "config: {config}"
    );
    assert!(
        !config.contains("model = \"auto\""),
        "NEAR AI must not persist the non-routable auto alias: {config}"
    );
}

#[test]
fn provider_admin_json_omits_absolute_host_paths() {
    let _env_lock = ironclaw_common::env_helpers::lock_env();
    let _env_mask = mask_llm_env_for_test();
    let temp = tempfile::tempdir().expect("tempdir");
    let admin = admin_for_home(&temp.path().join("reborn-home"));

    let list_json = serde_json::to_value(admin.list(None, false).expect("list")).expect("json");
    assert!(list_json.get("config_file").is_none(), "json: {list_json}");
    assert!(
        list_json.get("providers_file").is_none(),
        "json: {list_json}"
    );
    assert_eq!(list_json["v1_state"], RebornV1State::NotUsed.as_str());

    let status_json = serde_json::to_value(admin.status().expect("status")).expect("json");
    assert!(
        status_json.get("config_file").is_none(),
        "json: {status_json}"
    );
    assert!(
        status_json.get("providers_file").is_none(),
        "json: {status_json}"
    );
    assert_eq!(status_json["routes"], "not-configured");
    assert_eq!(status_json["v1_state"], RebornV1State::NotUsed.as_str());
}
