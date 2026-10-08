use std::time::Duration;

use ironclaw_config::ToolSelectionSection;
use ironclaw_tool_selection_jev::{
    DEFAULT_JEV_ENDPOINT, DEFAULT_JEV_MODEL, JevApiKey, JevEndpoint, JevToolClassifier,
};

use crate::operator_env::{strict_env_var, truncate_env_value_for_display};

/// Overrides `[tool_selection] classifier`: `off` or `jev`.
pub(crate) const REBORN_TOOL_SELECTION_ENV: &str = "REBORN_TOOL_SELECTION";

const DEFAULT_MAX_TOOLS: usize = 16;
const DEFAULT_TOKEN_BUDGET: u32 = 8_000;
const DEFAULT_JEV_API_KEY_ENV: &str = "TYPESAFE_API_KEY";
const DEFAULT_JEV_TIMEOUT_MS: u64 = 2_000;
/// A classification runs inside a turn, before the model is called.
const MAX_JEV_TIMEOUT_MS: u64 = 30_000;

/// Resolved turn-start tool selection settings and the classifier to bind.
#[derive(Debug)]
pub(super) struct ToolSelectionSettings {
    pub(super) max_tools: usize,
    pub(super) token_budget: u32,
    pub(super) classifier: JevToolClassifier,
}

/// Build turn-start tool selection from `[tool_selection]` and
/// `REBORN_TOOL_SELECTION` (env wins). `None` when the classifier is `off`,
/// the default.
///
/// Fails closed: an unknown classifier, a malformed endpoint or timeout, or `jev` without its API key refuses startup instead of running
/// without selection.
pub(super) fn tool_selection_config(
    config_file: Option<&ironclaw_config::RebornConfigFile>,
) -> anyhow::Result<Option<ToolSelectionSettings>> {
    resolve_tool_selection(
        config_file.and_then(|file| file.tool_selection.as_ref()),
        strict_env_var(REBORN_TOOL_SELECTION_ENV)?,
        strict_env_var,
    )
}

fn resolve_tool_selection(
    section: Option<&ToolSelectionSection>,
    classifier_override: Option<String>,
    env: impl Fn(&str) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<Option<ToolSelectionSettings>> {
    let classifier = classifier_override
        .or_else(|| section.and_then(|section| section.classifier.clone()))
        .unwrap_or_else(|| "off".to_string());
    match classifier.trim().to_ascii_lowercase().as_str() {
        "off" => return Ok(None),
        "jev" => {}
        _ => anyhow::bail!(
            "the tool selection classifier must be one of off, jev (got {:?})",
            truncate_env_value_for_display(&classifier)
        ),
    }
    let jev = section.and_then(|section| section.jev.as_ref());
    let api_key_env = jev
        .and_then(|jev| jev.api_key_env.as_deref())
        .unwrap_or(DEFAULT_JEV_API_KEY_ENV);
    // Safe to name in the error below: the config layer only admits a plain
    // variable name here, never a secret-shaped or free-form string.
    let Some(api_key) = env(api_key_env)? else {
        anyhow::bail!(
            "the jev tool selection classifier needs its API key in {api_key_env}, which is unset"
        );
    };
    let timeout_ms = jev
        .and_then(|jev| jev.timeout_ms)
        .unwrap_or(DEFAULT_JEV_TIMEOUT_MS);
    if !(1..=MAX_JEV_TIMEOUT_MS).contains(&timeout_ms) {
        anyhow::bail!(
            "[tool_selection.jev] timeout_ms must be between 1 and {MAX_JEV_TIMEOUT_MS} (got {timeout_ms})"
        );
    }
    let classifier = JevToolClassifier::new(
        JevEndpoint::parse(
            jev.and_then(|jev| jev.endpoint.as_deref())
                .unwrap_or(DEFAULT_JEV_ENDPOINT),
        )?,
        jev.and_then(|jev| jev.model.as_deref())
            .unwrap_or(DEFAULT_JEV_MODEL),
        JevApiKey::new(api_key)?,
        Duration::from_millis(timeout_ms),
    )?;
    Ok(Some(ToolSelectionSettings {
        max_tools: section
            .and_then(|section| section.max_tools)
            .unwrap_or(DEFAULT_MAX_TOOLS),
        token_budget: section
            .and_then(|section| section.token_budget)
            .unwrap_or(DEFAULT_TOKEN_BUDGET),
        classifier,
    }))
}

#[cfg(test)]
mod tests {
    use ironclaw_config::ToolSelectionJevSection;

    use super::*;

    fn key_set(name: &str) -> anyhow::Result<Option<String>> {
        Ok((name == "MY_JEV_KEY").then(|| "secret-key".to_string()))
    }

    fn jev_section() -> ToolSelectionSection {
        ToolSelectionSection {
            classifier: Some("jev".to_string()),
            max_tools: Some(12),
            token_budget: None,
            jev: Some(ToolSelectionJevSection {
                api_key_env: Some("MY_JEV_KEY".to_string()),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn selection_is_off_unless_a_classifier_is_named() {
        assert!(
            resolve_tool_selection(None, None, key_set)
                .expect("resolves")
                .is_none()
        );
        // The env override wins over the file, in both directions.
        assert!(
            resolve_tool_selection(Some(&jev_section()), Some("off".into()), key_set)
                .expect("resolves")
                .is_none()
        );
        let mut off_in_file = jev_section();
        off_in_file.classifier = Some("off".to_string());
        assert!(
            resolve_tool_selection(Some(&off_in_file), Some("jev".into()), key_set)
                .expect("resolves")
                .is_some()
        );
        let settings = resolve_tool_selection(Some(&jev_section()), None, key_set)
            .expect("resolves")
            .expect("jev is bound");
        assert_eq!(settings.classifier.model(), DEFAULT_JEV_MODEL);
        assert_eq!(
            (settings.max_tools, settings.token_budget),
            (12, DEFAULT_TOKEN_BUDGET)
        );
    }

    #[test]
    fn misconfiguration_refuses_startup() {
        let refused = |section: ToolSelectionSection, classifier: Option<&str>| {
            resolve_tool_selection(Some(&section), classifier.map(str::to_string), key_set)
                .expect_err("refused")
                .to_string()
        };
        assert!(refused(jev_section(), Some("semantic")).contains("off, jev"));

        // Jev with the default key variable, which this lookup leaves unset.
        let mut no_key = jev_section();
        no_key.jev = None;
        let error = refused(no_key, None);
        assert!(error.contains("TYPESAFE_API_KEY"), "{error}");
        assert!(!error.contains("secret-key"));

        let mut plain_http = jev_section();
        plain_http.jev.as_mut().expect("jev").endpoint = Some("http://jev.example/v1".into());
        refused(plain_http, None);

        let mut slow = jev_section();
        slow.jev.as_mut().expect("jev").timeout_ms = Some(MAX_JEV_TIMEOUT_MS + 1);
        assert!(refused(slow, None).contains("timeout_ms"));
    }
}
