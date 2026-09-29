//! Text-embeddings provider wiring: the `[embeddings]` config section plus the
//! `EMBEDDING_*` env overrides in, the configured provider (or none) out.
//!
//! Construction rules and the provider-id catalog live in the owning crate
//! (`ironclaw_llm::embeddings::create_embedding_provider`); this module only
//! maps the config section onto its settings. Fail closed: no section, no
//! provider id, or an unknown id yields `None`, never a default vendor.

use std::sync::Arc;

use ironclaw_config::EmbeddingsSection;
use ironclaw_llm::embeddings::{
    EmbeddingProvider, EmbeddingProviderSettings, create_embedding_provider,
};

/// Build the configured embeddings provider, or `None`.
///
/// `env` returns an environment variable's value, `None` when unset or blank;
/// it supplies the overrides and the API key named by `api_key_env`.
pub async fn resolve_embedding_provider(
    section: Option<&EmbeddingsSection>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Arc<dyn EmbeddingProvider>> {
    let section = section.cloned().unwrap_or_default();
    let settings = EmbeddingProviderSettings {
        provider_id: section.provider,
        base_url: section.base_url,
        model: section.model,
        api_key_env: section.api_key_env,
        dimension: section.dimension,
        max_batch_size: section.max_batch_size,
        request_timeout_secs: section.request_timeout_secs,
    };
    create_embedding_provider(settings, env).await
}
