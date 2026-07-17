//! Provider-catalog resolution for the assembled Reborn runtime.
//!
//! The three-layer LLM config model the operator sees:
//!
//! 1. **Catalog** — built-in `providers.json` + optional user-overlay
//!    `$IRONCLAW_REBORN_HOME/providers.json` (same JSON shape as
//!    v1's `~/.ironclaw/providers.json`). Loaded here via
//!    `ironclaw_llm::ProviderRegistry::load_from_path`.
//! 2. **Selection** — boot TOML's `[llm.<slot>]` section, parsed by
//!    `ironclaw_reborn_config::LlmSlotSelection`. "Use provider X
//!    for the `default` slot, with model Y."
//! 3. **Runtime config** — derived here. The resolved `ProviderDefinition`
//!    plus the selection's overrides becomes an `ironclaw_llm::LlmConfig`
//!    that `build_reborn_runtime` wires through the shared LLM provider
//!    chain.
//!
//! This module is the home of step 3. Lives behind the
//! `root-llm-provider` feature so the substrate-only composition stays
//! free of `ironclaw_llm`.
//!
//! When epic
//! [#3036](https://github.com/nearai/ironclaw/issues/3036)'s blueprint
//! apply service lands, it writes the selection into the eventual
//! `ProviderRepo` instead of into a TOML file; the runtime then reads
//! from the repo. The resolution logic in this module survives that
//! transition unchanged — the only thing that changes is whether the
//! `LlmSlotSelection` input came from a TOML reader or a repo read.

use std::path::Path;

use thiserror::Error;

use ironclaw_llm::{
    ProviderRegistry, ProviderResolutionError, ProviderSelection, registry::ProviderDefinition,
};
use ironclaw_reborn_config::{
    LlmSlotSelection, RebornBootConfig, RebornConfigFile, reject_inline_secret,
};

use crate::runtime_input::ResolvedRebornLlm;

/// Errors surfaced when resolving an `LlmSlotSelection` against the
/// merged provider catalog.
#[derive(Debug, Error)]
pub enum RebornLlmCatalogError {
    /// Selection didn't name a provider. Boot TOML carried
    /// `[llm.default]` with no `provider_id` field.
    #[error(
        "llm slot selection has no `provider_id`; set `[llm.<slot>] provider_id = \"...\"` in config.toml"
    )]
    MissingProviderId,
    /// `provider_id` doesn't exist in the merged catalog.
    #[error(
        "llm provider id `{requested}` not found in the provider catalog \
         (compiled-in + $IRONCLAW_REBORN_HOME/providers.json); known ids: [{known}]"
    )]
    UnknownProvider { requested: String, known: String },
    /// Provider requires an API key but the resolved env var isn't set.
    #[error(
        "llm provider `{provider}` requires API key env var `{env}` to be set; \
         export it (e.g. `export {env}=...`) or override with `[llm.<slot>] api_key_env = ...`"
    )]
    ApiKeyEnvUnset { provider: String, env: String },
    /// Provider says it needs an API key but doesn't expose an
    /// `api_key_env` setting (and the selection didn't override it).
    /// Theoretically impossible in a sane catalog; defensive guard.
    #[error(
        "llm provider `{provider}` requires an API key but the catalog entry has no \
         `api_key_env`; add `api_key_env` to the provider catalog entry or override via \
         `[llm.<slot>] api_key_env = ...`"
    )]
    ApiKeyEnvUnconfigured { provider: String },
    /// Provider's API-key env-var name is malformed. Do not echo it:
    /// malformed values may be pasted secret material from providers.json.
    #[error(
        "llm provider `{provider}` has an invalid api_key_env value; it must be an env-var name \
         like `OPENAI_API_KEY`, never the secret value"
    )]
    ApiKeyEnvInvalid { provider: String },
    /// Catalog field is malformed or secret-shaped. Do not echo it:
    /// values may be pasted secret material from providers.json.
    #[error(
        "llm provider `{provider}` at providers.json[{catalog_index}] has an invalid catalog field `{field}`; \
         provider catalog fields must not contain inline secret material"
    )]
    CatalogFieldInvalid {
        provider: String,
        catalog_index: usize,
        field: &'static str,
    },
    /// Provider requires a base URL (e.g. generic OpenAI-compatible) but
    /// neither the catalog nor the selection supplied one.
    #[error(
        "llm provider `{provider}` requires a base_url but neither the catalog entry's \
         `default_base_url` nor the selection's `base_url` override are set"
    )]
    BaseUrlUnconfigured { provider: String },
    /// Explicit Reborn provider overlay could not be loaded.
    #[error("could not load Reborn provider catalog: {source}")]
    CatalogLoad {
        #[source]
        source: ironclaw_llm::registry::ProviderRegistryLoadError,
    },
    /// Environment fallback could not be resolved by the LLM provider layer.
    #[error("could not resolve LLM environment fallback: {source}")]
    EnvResolution {
        #[source]
        source: ironclaw_llm::LlmError,
    },
}

/// Resolve the default Reborn runtime LLM from boot config, TOML selection,
/// and environment fallback.
///
/// This is the composition-owned provider/auth seam for standalone Reborn
/// ingress. Callers pass boot inputs; provider-specific auth details stay in
/// `ironclaw_llm` and never leak into CLI commands.
pub fn resolve_reborn_runtime_llm(
    boot: &RebornBootConfig,
    config_file: Option<&RebornConfigFile>,
) -> Result<Option<ResolvedRebornLlm>, RebornLlmCatalogError> {
    if let Some(selection) = config_file.and_then(|file| file.default_llm_slot()) {
        return resolve_llm_selection_against_catalog(
            selection,
            Some(boot.home().providers_file_path().as_path()),
        )
        .map(ResolvedRebornLlm::from_llm_config)
        .map(Some);
    }

    resolve_llm_from_env(boot)
}

fn resolve_llm_from_env(
    boot: &RebornBootConfig,
) -> Result<Option<ResolvedRebornLlm>, RebornLlmCatalogError> {
    ironclaw_llm::resolve_llm_config_from_env(Some(boot.home().providers_file_path().as_path()))
        .map(|maybe_config| maybe_config.map(ResolvedRebornLlm::from_llm_config))
        .map_err(|source| RebornLlmCatalogError::EnvResolution { source })
}

/// Resolve an `LlmSlotSelection` against the merged provider catalog.
///
/// Steps:
/// 1. Build the catalog (`ProviderRegistry::load_from_path(user)`).
/// 2. Look up the requested `provider_id`.
/// 3. Validate Reborn-specific secret/env-var policy on catalog and selection.
/// 4. Let `ironclaw_llm` resolve provider fields and build the full config.
pub fn resolve_llm_selection_against_catalog(
    selection: &LlmSlotSelection,
    user_providers_path: Option<&Path>,
) -> Result<ironclaw_llm::LlmConfig, RebornLlmCatalogError> {
    let registry = ProviderRegistry::try_load_from_path(user_providers_path)
        .map_err(|source| RebornLlmCatalogError::CatalogLoad { source })?;
    resolve_against_registry(selection, &registry)
}

/// Resolve a selection like [`resolve_llm_selection_against_catalog`], but
/// tolerate a required key whose env var isn't set — treats the provider as
/// keyless for this resolution only.
///
/// - Caller: CLI runtime seam (`build_runtime_input_with_options`), only
///   after it has independently confirmed a key is durably stored locally.
/// - Store-agnostic by design: the store lookup stays at the call site;
///   `apply_startup_stored_llm_key` overlays the stored key at startup.
/// - Retries keyless only on the exact `ApiKeyEnvUnset` outcome. Any other
///   error (e.g. `ApiKeyEnvUnconfigured`) propagates unchanged — never masks
///   a genuine catalog misconfiguration.
pub fn resolve_llm_selection_allow_missing_key(
    selection: &LlmSlotSelection,
    user_providers_path: Option<&Path>,
) -> Result<ironclaw_llm::LlmConfig, RebornLlmCatalogError> {
    let registry = ProviderRegistry::try_load_from_path(user_providers_path)
        .map_err(|source| RebornLlmCatalogError::CatalogLoad { source })?;
    resolve_allow_missing_key_against_registry(selection, &registry)
}

/// Registry-level counterpart of [`resolve_llm_selection_allow_missing_key`],
/// unit-testable against a synthetic registry without touching the filesystem.
fn resolve_allow_missing_key_against_registry(
    selection: &LlmSlotSelection,
    registry: &ProviderRegistry,
) -> Result<ironclaw_llm::LlmConfig, RebornLlmCatalogError> {
    match resolve_against_registry(selection, registry) {
        Ok(config) => Ok(config),
        Err(RebornLlmCatalogError::ApiKeyEnvUnset { .. }) => {
            let provider_id = selection
                .provider_id
                .as_deref()
                .ok_or(RebornLlmCatalogError::MissingProviderId)?;
            let keyless_registry = registry_with_provider_treated_as_keyless(registry, provider_id);
            resolve_against_registry(selection, &keyless_registry)
        }
        Err(other) => Err(other),
    }
}

/// Clones `registry`, marking the entry matching `provider_id` (by id or
/// alias, case-insensitive, matching [`ProviderRegistry::find`]) as
/// `api_key_required = false`. Nothing else changes.
fn registry_with_provider_treated_as_keyless(
    registry: &ProviderRegistry,
    provider_id: &str,
) -> ProviderRegistry {
    let providers = registry
        .all()
        .iter()
        .cloned()
        .map(|mut provider| {
            if provider.id.eq_ignore_ascii_case(provider_id)
                || provider
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(provider_id))
            {
                provider.api_key_required = false;
            }
            provider
        })
        .collect();
    ProviderRegistry::new(providers)
}

/// Validate provider-overlay bytes with the same typed definitions and
/// Reborn-specific catalog checks used by runtime composition.
#[derive(Debug, thiserror::Error)]
pub enum ProviderCatalogValidationError {
    #[error("invalid provider catalog JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(transparent)]
    Invalid(#[from] RebornLlmCatalogError),
}

pub fn validate_reborn_provider_catalog_contents(
    contents: &str,
) -> Result<(), ProviderCatalogValidationError> {
    let definitions =
        serde_json::from_str::<Vec<ironclaw_llm::registry::ProviderDefinition>>(contents)?;
    let registry = ProviderRegistry::new(definitions);
    validate_catalog(&registry)?;
    Ok(())
}

/// Resolve a selection against a pre-built registry. Useful in tests
/// where a synthetic registry can be assembled without touching the
/// filesystem.
pub fn resolve_against_registry(
    selection: &LlmSlotSelection,
    registry: &ProviderRegistry,
) -> Result<ironclaw_llm::LlmConfig, RebornLlmCatalogError> {
    validate_catalog(registry)?;

    let provider_id = selection
        .provider_id
        .as_deref()
        .ok_or(RebornLlmCatalogError::MissingProviderId)?;

    let provider =
        registry
            .find(provider_id)
            .ok_or_else(|| RebornLlmCatalogError::UnknownProvider {
                requested: provider_id.to_string(),
                known: registry
                    .all()
                    .iter()
                    .map(|provider| safe_catalog_display_value("providers.<id>", &provider.id))
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;

    let catalog_index = registry
        .all()
        .iter()
        .position(|candidate| std::ptr::eq(candidate, provider))
        .unwrap_or(0);

    validate_selection(selection, provider, catalog_index)?;

    let resolved = ironclaw_llm::resolve_provider_config_from_selection(
        ProviderSelection {
            provider_id: provider.id.clone(),
            api_key_env: selection.api_key_env.clone(),
            base_url: selection.base_url.clone(),
            model: selection.model.clone(),
        },
        registry,
    )
    .map_err(|source| map_selection_resolution_error(source, selection, provider))?;

    validate_catalog_text(
        provider,
        catalog_index,
        "resolved_base_url",
        resolved.base_url(),
    )?;
    validate_catalog_text(provider, catalog_index, "resolved_model", resolved.model())?;

    ironclaw_llm::build_llm_config_from_resolved_provider(resolved)
        .map_err(|source| RebornLlmCatalogError::EnvResolution { source })
}

/// Overlay an operator-stored API-key value onto a resolved `LlmConfig`.
///
/// The catalog/selection only ever carry an `api_key_env` *name* — values are
/// rejected as inline secrets. When the operator pastes a key through the
/// webui2 settings surface it is stored in the scoped secret store and applied
/// here, after catalog/env resolution, so a stored value takes precedence over
/// whatever `api_key_env` resolved to. Both the startup resolution and the live
/// reload path call this so the two never drift.
///
/// Custom providers that rely on a stored key must be written into the overlay
/// with `api_key_required = false`, otherwise resolution fails closed on the
/// missing env var before this injection runs.
pub(crate) fn apply_stored_api_key(
    config: &mut ironclaw_llm::LlmConfig,
    key: secrecy::SecretString,
) {
    if let Some(provider) = config.provider.as_mut() {
        provider.api_key = Some(key);
    } else if config.backend == "nearai" {
        config.nearai.api_key = Some(key);
    }
    // Dedicated codex/gemini/bedrock backends authenticate via OAuth/session or
    // AWS credential chains rather than a pasted key; surfacing those in the UI
    // is a follow-up, so they are intentionally left untouched here.
}

fn validate_selection(
    selection: &LlmSlotSelection,
    provider: &ProviderDefinition,
    catalog_index: usize,
) -> Result<(), RebornLlmCatalogError> {
    if let Some(env) = selection.api_key_env.as_deref()
        && (reject_inline_secret("llm.<slot>.api_key_env", env).is_err() || !is_env_var_name(env))
    {
        return Err(RebornLlmCatalogError::ApiKeyEnvInvalid {
            provider: provider.id.clone(),
        });
    }
    if let Some(base_url) = selection.base_url.as_deref() {
        validate_catalog_text(provider, catalog_index, "selection_base_url", base_url)?;
    }
    if let Some(model) = selection.model.as_deref() {
        validate_catalog_text(provider, catalog_index, "selection_model", model)?;
    }
    Ok(())
}

fn map_selection_resolution_error(
    source: ProviderResolutionError,
    selection: &LlmSlotSelection,
    provider: &ProviderDefinition,
) -> RebornLlmCatalogError {
    match source {
        ProviderResolutionError::MissingApiKey {
            provider: error_provider,
        } if error_provider == provider.id => {
            match selection
                .api_key_env
                .clone()
                .or_else(|| provider.api_key_env.clone())
            {
                Some(env) => RebornLlmCatalogError::ApiKeyEnvUnset {
                    provider: provider.id.clone(),
                    env,
                },
                None => RebornLlmCatalogError::ApiKeyEnvUnconfigured {
                    provider: provider.id.clone(),
                },
            }
        }
        ProviderResolutionError::MissingBaseUrl {
            provider: error_provider,
        } if error_provider == provider.id => RebornLlmCatalogError::BaseUrlUnconfigured {
            provider: provider.id.clone(),
        },
        source => RebornLlmCatalogError::EnvResolution {
            source: source.into_llm_error(),
        },
    }
}

fn validate_catalog(registry: &ProviderRegistry) -> Result<(), RebornLlmCatalogError> {
    for (catalog_index, provider) in registry.all().iter().enumerate() {
        validate_catalog_provider(provider, catalog_index)?;
    }
    Ok(())
}

fn validate_catalog_provider(
    provider: &ProviderDefinition,
    catalog_index: usize,
) -> Result<(), RebornLlmCatalogError> {
    validate_catalog_text(provider, catalog_index, "id", &provider.id)?;
    if let Some(base_url) = provider.default_base_url.as_deref() {
        validate_catalog_text(provider, catalog_index, "default_base_url", base_url)?;
    }
    validate_catalog_text(
        provider,
        catalog_index,
        "default_model",
        &provider.default_model,
    )?;
    validate_catalog_env_var(provider, catalog_index, "model_env", &provider.model_env)?;
    if let Some(base_url_env) = provider.base_url_env.as_deref() {
        validate_catalog_env_var(provider, catalog_index, "base_url_env", base_url_env)?;
    }
    if let Some(api_key_env) = provider.api_key_env.as_deref() {
        validate_catalog_env_var(provider, catalog_index, "api_key_env", api_key_env)?;
    }
    if let Some(extra_headers_env) = provider.extra_headers_env.as_deref() {
        validate_catalog_env_var(
            provider,
            catalog_index,
            "extra_headers_env",
            extra_headers_env,
        )?;
    }
    Ok(())
}

fn validate_catalog_text(
    provider: &ProviderDefinition,
    catalog_index: usize,
    field: &'static str,
    value: &str,
) -> Result<(), RebornLlmCatalogError> {
    if reject_inline_secret("provider catalog field", value).is_err() {
        return Err(RebornLlmCatalogError::CatalogFieldInvalid {
            provider: safe_catalog_display_value("providers.<id>", &provider.id),
            catalog_index,
            field,
        });
    }
    Ok(())
}

fn validate_catalog_env_var(
    provider: &ProviderDefinition,
    catalog_index: usize,
    field: &'static str,
    value: &str,
) -> Result<(), RebornLlmCatalogError> {
    if reject_inline_secret("provider catalog env-var field", value).is_err()
        || !is_env_var_name(value)
    {
        return Err(RebornLlmCatalogError::CatalogFieldInvalid {
            provider: safe_catalog_display_value("providers.<id>", &provider.id),
            catalog_index,
            field,
        });
    }
    Ok(())
}

fn safe_catalog_display_value(label: &'static str, value: &str) -> String {
    if reject_inline_secret(label, value).is_err() {
        "<redacted>".to_string()
    } else {
        value.to_string()
    }
}

fn is_env_var_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_llm::registry::{ProviderProtocol, ProviderRegistry};

    fn provider_with_required_key(id: &str, env: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("https://example.test/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: Some(env.to_string()),
            api_key_required: true,
            model_env: "TEST_MODEL".to_string(),
            default_model: "test-model".to_string(),
            description: "test".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    fn provider_no_key_required(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::Ollama,
            default_base_url: Some("http://localhost:11434".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "TEST_MODEL".to_string(),
            default_model: "llama3".to_string(),
            description: "test (no key)".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    fn provider_no_base_url_required(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: None,
            base_url_env: Some("REBORN_TEST_OPTIONAL_BASE_URL_UNSET_DO_NOT_SET_9c13".to_string()),
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "REBORN_TEST_MODEL_UNSET_DO_NOT_SET_9c13".to_string(),
            default_model: "default-model".to_string(),
            description: "test (client-default base url)".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    fn provider_base_url_required(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            base_url_required: true,
            ..provider_no_base_url_required(id)
        }
    }

    fn provider_github_copilot_no_key_required(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::GithubCopilot,
            default_base_url: Some("https://api.githubcopilot.com".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "REBORN_TEST_GITHUB_COPILOT_MODEL_UNSET_DO_NOT_SET_9c13".to_string(),
            default_model: "gpt-4o".to_string(),
            description: "test github copilot".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    fn provider_with_protocol(id: &str, protocol: ProviderProtocol) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol,
            default_base_url: None,
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "REBORN_TEST_DEDICATED_MODEL_UNSET_DO_NOT_SET_9c13".to_string(),
            default_model: "dedicated-default-model".to_string(),
            description: "test dedicated provider".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    #[test]
    fn unknown_provider_lists_known_ids() {
        let registry =
            ProviderRegistry::new(vec![provider_with_required_key("alpha", "ALPHA_KEY")]);
        let selection = LlmSlotSelection {
            provider_id: Some("does-not-exist".to_string()),
            ..Default::default()
        };
        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        match err {
            RebornLlmCatalogError::UnknownProvider { requested, known } => {
                assert_eq!(requested, "does-not-exist");
                assert!(known.contains("alpha"), "known list: {known}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn missing_required_api_key_env_fails_closed() {
        // Use a uniquely-named env var that we never set; no `set_var`
        // / `remove_var` calls (forbidden under `forbid(unsafe_code)`
        // post-edition-2024). The unique suffix means even if the test
        // environment happens to pre-set similar names, this one is
        // free.
        let env_name = "REBORN_TEST_UNSET_API_KEY_DO_NOT_SET_8a3f1c2e";
        debug_assert!(
            std::env::var(env_name).is_err(),
            "test depends on `{env_name}` being unset"
        );
        let registry = ProviderRegistry::new(vec![provider_with_required_key("alpha", env_name)]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };
        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        assert!(matches!(err, RebornLlmCatalogError::ApiKeyEnvUnset { .. }));
    }

    /// Companion to `missing_required_api_key_env_fails_closed`: with the
    /// keyless override applied, the same unset-env-var setup must resolve
    /// (no API key on the resulting config). Drives
    /// `resolve_allow_missing_key_against_registry` itself (not a
    /// hand-built keyless registry) so the test fails if the
    /// `ApiKeyEnvUnset` -> keyless-retry control flow regresses.
    #[test]
    fn allow_missing_key_resolves_a_required_key_provider_without_the_env_var_set() {
        let env_name = "REBORN_TEST_UNSET_API_KEY_ALLOW_MISSING_DO_NOT_SET_7d2b";
        debug_assert!(
            std::env::var(env_name).is_err(),
            "test depends on `{env_name}` being unset"
        );
        let registry = ProviderRegistry::new(vec![provider_with_required_key("alpha", env_name)]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };
        let config = resolve_allow_missing_key_against_registry(&selection, &registry)
            .expect("keyless resolution must succeed even though the API key env var is unset");
        let provider = config.provider.expect("registry provider config");
        assert!(
            provider.api_key.is_none(),
            "no api key should be resolved from the (still-unset) env var"
        );
    }

    /// A provider with `api_key_required = true` but no `api_key_env` must
    /// surface `ApiKeyEnvUnconfigured`, not be silently retried keyless —
    /// only the exact `ApiKeyEnvUnset` outcome may retry keyless.
    #[test]
    fn allow_missing_key_does_not_mask_a_missing_api_key_env_configuration() {
        let malformed = ProviderDefinition {
            api_key_env: None,
            ..provider_with_required_key("alpha", "UNUSED_ENV_NAME")
        };
        let registry = ProviderRegistry::new(vec![malformed]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let err = resolve_allow_missing_key_against_registry(&selection, &registry)
            .expect_err("a provider requiring a key but missing `api_key_env` must surface, not resolve keyless");
        assert!(
            matches!(err, RebornLlmCatalogError::ApiKeyEnvUnconfigured { .. }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn registry_with_provider_treated_as_keyless_only_affects_the_matched_provider() {
        let registry = ProviderRegistry::new(vec![
            provider_with_required_key("alpha", "REBORN_TEST_ALPHA_KEY_DO_NOT_SET_7d2b"),
            provider_with_required_key("beta", "REBORN_TEST_BETA_KEY_DO_NOT_SET_7d2b"),
        ]);
        let keyless = registry_with_provider_treated_as_keyless(&registry, "alpha");
        assert!(
            !keyless
                .find("alpha")
                .expect("alpha present")
                .api_key_required,
            "the requested provider must be treated as keyless"
        );
        assert!(
            keyless.find("beta").expect("beta present").api_key_required,
            "every other provider's api_key_required must be untouched"
        );
    }

    #[test]
    fn malformed_api_key_env_fails_without_echoing_value() {
        let pasted_secret = format!("{}{}", "s", "k-proj-1234567890abcdef1234567890");
        let registry =
            ProviderRegistry::new(vec![provider_with_required_key("alpha", &pasted_secret)]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        assert!(matches!(
            err,
            RebornLlmCatalogError::CatalogFieldInvalid {
                field: "api_key_env",
                ..
            }
        ));
        let rendered = err.to_string();
        assert!(
            !rendered.contains(&pasted_secret),
            "error must not echo pasted secret: {rendered}"
        );
    }

    #[test]
    fn secret_shaped_env_name_fails_without_echoing_value() {
        // `github_pat_...` is syntactically a valid env-var name, so the
        // inline-secret guard must run before missing-env errors can echo it.
        let pasted_secret = format!("{}{}", "github_", "pat_abcdef1234567890abcdef1234567890");
        let registry =
            ProviderRegistry::new(vec![provider_with_required_key("alpha", &pasted_secret)]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        assert!(matches!(
            err,
            RebornLlmCatalogError::CatalogFieldInvalid {
                field: "api_key_env",
                ..
            }
        ));
        let rendered = err.to_string();
        assert!(
            !rendered.contains(&pasted_secret),
            "error must not echo pasted secret: {rendered}"
        );
    }

    #[test]
    fn secret_shaped_default_base_url_fails_without_echoing_value() {
        let pasted_secret = format!(
            "https://proxy.example/v1?key={}{}",
            "s", "k-proj-1234567890abcdef1234567890"
        );
        let provider = ProviderDefinition {
            default_base_url: Some(pasted_secret.clone()),
            ..provider_no_key_required("alpha")
        };
        let registry = ProviderRegistry::new(vec![provider]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        assert!(matches!(
            err,
            RebornLlmCatalogError::CatalogFieldInvalid {
                field: "default_base_url",
                ..
            }
        ));
        let rendered = err.to_string();
        assert!(
            !rendered.contains(&pasted_secret),
            "error must not echo pasted secret: {rendered}"
        );
    }

    #[test]
    fn unused_secret_shaped_catalog_entry_fails_closed() {
        let pasted_secret = format!(
            "https://proxy.example/v1?key={}{}",
            "s", "k-proj-1234567890abcdef1234567890"
        );
        let unused_provider = ProviderDefinition {
            default_base_url: Some(pasted_secret.clone()),
            ..provider_no_key_required("unused")
        };
        let registry =
            ProviderRegistry::new(vec![unused_provider, provider_no_key_required("selected")]);
        let selection = LlmSlotSelection {
            provider_id: Some("selected".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry)
            .expect_err("unused secret-shaped catalog entry must fail closed");
        assert!(matches!(
            err,
            RebornLlmCatalogError::CatalogFieldInvalid {
                ref provider,
                catalog_index: 0,
                field: "default_base_url",
            } if provider == "unused"
        ));
        let rendered = err.to_string();
        assert!(
            !rendered.contains(&pasted_secret),
            "error must not echo pasted secret: {rendered}"
        );
    }

    #[test]
    fn unknown_provider_redacts_secret_shaped_catalog_ids() {
        let pasted_secret = format!("{}{}", "github_", "pat_abcdef1234567890abcdef1234567890");
        let registry = ProviderRegistry::new(vec![provider_no_key_required(&pasted_secret)]);
        let selection = LlmSlotSelection {
            provider_id: Some("missing".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        let rendered = err.to_string();
        assert!(rendered.contains("<redacted>"), "error: {rendered}");
        assert!(
            !rendered.contains(&pasted_secret),
            "error must not echo pasted secret: {rendered}"
        );
    }

    #[test]
    fn happy_path_no_key_required_uses_catalog_default_model_and_base_url() {
        let registry = ProviderRegistry::new(vec![provider_no_key_required("alpha")]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        let provider = config.provider.as_ref().expect("registry provider");
        assert_eq!(provider.provider_id, "alpha");
        assert_eq!(provider.model, "llama3"); // catalog default
        assert_eq!(provider.base_url, "http://localhost:11434"); // catalog default
        assert_eq!(provider.protocol, ProviderProtocol::Ollama);
        assert!(provider.api_key.is_none());
    }

    #[test]
    fn optional_missing_base_url_resolves_to_client_default() {
        let registry = ProviderRegistry::new(vec![provider_no_base_url_required("alpha")]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        let provider = config.provider.as_ref().expect("registry provider");
        assert_eq!(provider.base_url, "");
        assert_eq!(provider.model, "default-model");
    }

    #[test]
    fn missing_required_base_url_fails_closed() {
        let registry = ProviderRegistry::new(vec![provider_base_url_required("alpha")]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };

        let err = resolve_against_registry(&selection, &registry).expect_err("must error");
        assert!(matches!(
            err,
            RebornLlmCatalogError::BaseUrlUnconfigured { .. }
        ));
    }

    #[test]
    fn selection_overrides_take_precedence_over_catalog() {
        let registry = ProviderRegistry::new(vec![provider_no_key_required("alpha")]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            model: Some("custom-model".to_string()),
            base_url: Some("https://override.test/v1".to_string()),
            api_key_env: None,
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        let provider = config.provider.as_ref().expect("registry provider");
        assert_eq!(provider.model, "custom-model");
        assert_eq!(provider.base_url, "https://override.test/v1");
    }

    #[test]
    fn github_copilot_protocol_carries_default_headers() {
        let registry = ProviderRegistry::new(vec![provider_github_copilot_no_key_required(
            "tenant_copilot",
        )]);
        let selection = LlmSlotSelection {
            provider_id: Some("tenant_copilot".to_string()),
            ..Default::default()
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        assert!(
            config
                .provider
                .as_ref()
                .expect("registry provider")
                .extra_headers
                .iter()
                .any(|(key, _)| key == "Editor-Version"),
            "headers: {:?}",
            config.provider.as_ref().unwrap().extra_headers
        );
        assert!(
            config
                .provider
                .as_ref()
                .expect("registry provider")
                .extra_headers
                .iter()
                .any(|(key, _)| key == "Copilot-Integration-Id"),
            "headers: {:?}",
            config.provider.as_ref().unwrap().extra_headers
        );
    }

    #[test]
    fn nearai_catalog_selection_resolves_to_full_dedicated_llm_config() {
        let registry = ProviderRegistry::new(vec![provider_with_protocol(
            "nearai",
            ProviderProtocol::NearAi,
        )]);
        let selection = LlmSlotSelection {
            provider_id: Some("nearai".to_string()),
            model: Some("nearai/test-model".to_string()),
            base_url: Some("https://private.near.ai".to_string()),
            api_key_env: None,
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        assert_eq!(config.backend, "nearai");
        assert_eq!(config.nearai.model, "nearai/test-model");
        assert_eq!(config.nearai.base_url, "https://private.near.ai");
        assert!(config.provider.is_none());
    }

    #[test]
    fn openai_codex_catalog_selection_resolves_to_full_dedicated_llm_config() {
        let registry = ProviderRegistry::new(vec![provider_with_protocol(
            "openai_codex",
            ProviderProtocol::OpenAiCodex,
        )]);
        let selection = LlmSlotSelection {
            provider_id: Some("openai_codex".to_string()),
            model: Some("gpt-test-codex".to_string()),
            ..Default::default()
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        assert_eq!(config.backend, "openai_codex");
        assert_eq!(
            config.openai_codex.as_ref().expect("codex config").model,
            "gpt-test-codex"
        );
        assert!(config.provider.is_none());
    }

    #[test]
    fn gemini_oauth_catalog_selection_resolves_to_full_dedicated_llm_config() {
        let registry = ProviderRegistry::new(vec![provider_with_protocol(
            "gemini_oauth",
            ProviderProtocol::GeminiOauth,
        )]);
        let selection = LlmSlotSelection {
            provider_id: Some("gemini_oauth".to_string()),
            model: Some("gemini-test".to_string()),
            ..Default::default()
        };

        let config = resolve_against_registry(&selection, &registry).expect("must resolve");
        assert_eq!(config.backend, "gemini_oauth");
        assert_eq!(
            config
                .gemini_oauth
                .as_ref()
                .expect("gemini oauth config")
                .model,
            "gemini-test"
        );
        assert!(config.provider.is_none());
    }

    #[test]
    fn secret_shaped_extra_headers_env_name_fails_before_value_lookup() {
        let mut provider = provider_no_key_required("alpha");
        provider.extra_headers_env = Some("sk-proj-1234567890abcdef".to_string());

        let err = validate_catalog_provider(&provider, 7).expect_err("secret-shaped env must fail");
        let rendered = err.to_string();
        assert!(matches!(
            err,
            RebornLlmCatalogError::CatalogFieldInvalid { .. }
        ));
        assert!(
            !rendered.contains("sk-proj-1234567890abcdef"),
            "error must not echo secret-shaped env name: {rendered}"
        );
        assert!(
            rendered.contains("providers.json[7]"),
            "error must identify catalog index: {rendered}"
        );
    }

    #[test]
    fn apply_stored_api_key_overrides_registry_provider() {
        use secrecy::ExposeSecret as _;

        let registry = ProviderRegistry::new(vec![provider_no_key_required("alpha")]);
        let selection = LlmSlotSelection {
            provider_id: Some("alpha".to_string()),
            ..Default::default()
        };
        let mut config = resolve_against_registry(&selection, &registry).expect("resolve");
        assert!(
            config
                .provider
                .as_ref()
                .expect("provider")
                .api_key
                .is_none()
        );

        apply_stored_api_key(&mut config, secrecy::SecretString::from("sk-stored"));

        let stored = config
            .provider
            .as_ref()
            .expect("provider")
            .api_key
            .as_ref()
            .expect("api key");
        assert_eq!(stored.expose_secret(), "sk-stored");
    }

    #[test]
    fn apply_stored_api_key_targets_nearai_dedicated_config() {
        use secrecy::ExposeSecret as _;

        let registry = ProviderRegistry::new(vec![provider_with_protocol(
            "nearai",
            ProviderProtocol::NearAi,
        )]);
        let selection = LlmSlotSelection {
            provider_id: Some("nearai".to_string()),
            model: Some("nearai/test-model".to_string()),
            ..Default::default()
        };
        let mut config = resolve_against_registry(&selection, &registry).expect("resolve");
        assert!(config.provider.is_none());

        apply_stored_api_key(&mut config, secrecy::SecretString::from("sk-nearai"));

        assert_eq!(
            config
                .nearai
                .api_key
                .as_ref()
                .expect("nearai key")
                .expose_secret(),
            "sk-nearai"
        );
    }

    #[test]
    fn explicit_malformed_provider_overlay_fails_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let providers = temp.path().join("providers.json");
        std::fs::write(&providers, "not json").expect("write providers");
        let selection = LlmSlotSelection {
            provider_id: Some("openai".to_string()),
            ..Default::default()
        };

        let err = resolve_llm_selection_against_catalog(&selection, Some(&providers))
            .expect_err("malformed explicit provider overlay must fail");
        assert!(matches!(err, RebornLlmCatalogError::CatalogLoad { .. }));
    }
}
