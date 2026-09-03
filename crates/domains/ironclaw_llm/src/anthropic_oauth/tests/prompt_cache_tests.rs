//! Denylist-gate cache-retention regression.
//!
//! Split out of `anthropic_oauth/tests.rs` (arch file-size budget) — a child
//! of that module, so `use super::*` reaches both `anthropic_oauth.rs`'s
//! items and `tests.rs`'s own capture-server helpers
//! (`capture_one_request`, `captured_json`).

use super::*;

/// Regression for the allowlist->denylist gate change: a model family
/// released after `supports_prompt_cache` was written (matching none of the
/// old hardcoded "claude-3"/"claude-4"/"claude-sonnet"/... prefixes) must
/// still emit explicit cache breakpoints under `Short` retention instead of
/// being silently downgraded to no caching.
#[tokio::test]
async fn oauth_new_model_family_keeps_short_retention_cache_control() {
    let (base_url, captured) = capture_one_request().await;
    let mut config = RegistryProviderConfig::generic(
        crate::registry::ProviderProtocol::Anthropic,
        "anthropic_oauth",
        None,
        &base_url,
        "claude-fable-5-1",
    );
    config.oauth_token = Some(SecretString::from("test-token".to_string()));
    config.cache_retention = CacheRetention::Short;
    let provider = AnthropicOAuthProvider::new(&config).expect("provider");

    let _ = provider
        .complete(CompletionRequest::new(vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Question"),
        ]))
        .await;
    let body = captured_json(captured).await;

    let system = body["system"]
        .as_array()
        .expect("system serialized as blocks when caching is on");
    assert_eq!(
        system.last().expect("system block")["cache_control"]["type"],
        "ephemeral"
    );
}
