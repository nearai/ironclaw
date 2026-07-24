//! Config-driven memory provider factory (issue #3537 / #5264).
//!
//! This is the memory analog of `ironclaw_embeddings::factory::create_provider`:
//! a pure-data connection config + a runtime [`MemoryProviderDeps`] struct feed an
//! function that `match`es the resolved [`MemoryProviderBinding`] and builds the
//! matching concrete [`MemoryService`] — native (filesystem) or a third party
//! (currently mem0, over its real `reqwest` transport). Missing credentials or an
//! unknown id yield `None` (fail-closed), exactly like the embeddings factory.
//!
//! Composition is the one layer that may name concrete provider crates
//! (`ironclaw_memory_native`, `ironclaw_memory_mem0`); `ironclaw_host_runtime`'s
//! [`MemoryServiceResolver`] stays provider-agnostic and only stores the
//! `Arc<dyn MemoryService>` instances this factory builds.
//!
//! [`build_memory_service_resolver`] is the build-time wiring used at startup:
//! resolve policy → build resolver → for a third-party document-store binding,
//! build the provider here and register it into the resolver. Production
//! third-party bindings stay fail-closed/override-gated by the upstream
//! [`MemoryBindingPolicy`]; this factory never relaxes that — it only constructs
//! a provider for a binding the policy already permitted.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_runtime::memory_binding::{MemoryBindingPolicy, MemoryProviderBinding};
use ironclaw_host_runtime::memory_provider::MemoryServiceResolver;
use ironclaw_memory::{MemoryService, PromptWriteSafetyEventSink};
#[cfg(feature = "memory-mem0")]
use ironclaw_memory_mem0::{
    MEM0_MEMORY_EXTENSION_ID, Mem0Config, Mem0HttpTransport, Mem0MemoryService, Mem0Transport,
};
use ironclaw_memory_native::NativeMemoryService;
#[cfg(feature = "memory-mem0")]
use secrecy::ExposeSecret;
use secrecy::SecretString;

const LOG_TARGET: &str = "ironclaw_reborn::memory";

/// Connection settings for the configured third-party memory provider.
///
/// Pure data, populated from the `[memory]` config section + env, mirroring
/// `EmbeddingsConfig`'s `openai_api_key` / `*_base_url` shape: the base URL comes
/// from config/env, the API key is a [`SecretString`] from an env var. Selection
/// (which provider serves a profile) stays in the binding policy; this only
/// carries the chosen provider's connection details.
#[derive(Clone, Default)]
pub struct Mem0ConnectionConfig {
    /// mem0 base URL for the self-hosted mem0 OSS server, from
    /// `[memory].mem0_base_url` or the `MEMORY_MEM0_BASE_URL` env var. There is
    /// NO default: mem0 stays off unless it is explicitly bound AND given a base
    /// URL here; a bound-but-unset mem0 fails closed in the factory.
    pub base_url: Option<String>,
    /// Optional mem0 API key, from `MEMORY_MEM0_API_KEY`. `None` for a self-hosted
    /// server with `AUTH_DISABLED=true` (the default). When set, held as a
    /// [`SecretString`] so it is redacted in `Debug`/logs and exposed only when
    /// building the transport.
    pub api_key: Option<SecretString>,
    /// Optional mem0 `app_id` partition, from `MEMORY_MEM0_APP_ID`.
    pub app_id: Option<String>,
}

impl std::fmt::Debug for Mem0ConnectionConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Mem0ConnectionConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("app_id", &self.app_id)
            .finish()
    }
}

/// Runtime wiring the memory provider factory needs (the `ProviderDeps` analog).
///
/// Each field is consulted only by the matching arm: `filesystem` /
/// `prompt_write_safety_sink` by the native arm, `mem0` / `mem0_transport_override`
/// by the mem0 arm. The startup wiring builds only third-party providers (native
/// is resolved per-invocation by the resolver), so it leaves `filesystem` /
/// `prompt_write_safety_sink` as `None`; see [`MemoryProviderDeps::for_third_party`].
pub struct MemoryProviderDeps {
    /// Native-arm filesystem. `None` when the factory builds only third-party
    /// (REST) providers — the startup path, where the resolver builds native
    /// per-invocation with the request filesystem instead.
    pub filesystem: Option<Arc<dyn RootFilesystem>>,
    /// Native-arm prompt-write safety sink. `None` at startup (the native arm is
    /// not built there); the resolver supplies it per-invocation.
    pub prompt_write_safety_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    /// mem0 connection settings for the mem0 arm.
    pub mem0: Mem0ConnectionConfig,
    /// Test seam: a pre-built mem0 transport (an in-memory mock). Production
    /// leaves this `None`, so the factory builds a real `reqwest` transport from
    /// [`Mem0ConnectionConfig`]; tests inject a mock to exercise the wiring
    /// without a live mem0 endpoint. Gated with the provider itself — there is no
    /// mem0 transport type to hold when `memory-mem0` is not compiled in.
    #[cfg(feature = "memory-mem0")]
    pub mem0_transport_override: Option<Arc<dyn Mem0Transport>>,
}

impl MemoryProviderDeps {
    /// Deps for the startup third-party registration path: no native filesystem
    /// (native is per-invocation), no transport override (build the real one).
    pub fn for_third_party(mem0: Mem0ConnectionConfig) -> Self {
        Self {
            filesystem: None,
            prompt_write_safety_sink: None,
            mem0,
            #[cfg(feature = "memory-mem0")]
            mem0_transport_override: None,
        }
    }
}

/// Build the document-store provider for a resolved [`MemoryProviderBinding`],
/// or `None` (fail-closed).
///
/// - `Native` → the host-bundled [`NativeMemoryService`] over `deps.filesystem`
///   (used by this factory's tests and any eager-native caller; the startup
///   wiring resolves native per-invocation through the resolver, so it does not
///   route native here). `None` if no filesystem was supplied.
/// - `ThirdParty(id)` → the matching provider — currently only the mem0 id — from
///   its connection config over its real transport (or an injected mock). An
///   unknown id, or missing/invalid mem0 connection settings, yield `None`.
/// - `Disabled` → `None`.
pub fn create_document_store_provider(
    binding: &MemoryProviderBinding,
    deps: &MemoryProviderDeps,
) -> Option<Arc<dyn MemoryService>> {
    match binding {
        MemoryProviderBinding::Native => deps.filesystem.clone().map(|filesystem| {
            Arc::new(NativeMemoryService::from_filesystem(
                filesystem,
                deps.prompt_write_safety_sink.clone(),
            )) as Arc<dyn MemoryService>
        }),
        MemoryProviderBinding::ThirdParty { extension_id } => {
            create_third_party_provider(extension_id.as_str(), deps)
        }
        MemoryProviderBinding::Disabled => None,
    }
}

fn create_third_party_provider(
    extension_id: &str,
    deps: &MemoryProviderDeps,
) -> Option<Arc<dyn MemoryService>> {
    #[cfg(feature = "memory-mem0")]
    if extension_id == MEM0_MEMORY_EXTENSION_ID {
        return create_mem0_provider(deps);
    }
    // No provider is registered for this third-party id — or the `memory-mem0`
    // feature is not compiled in — so the document-store binding fails closed.
    #[cfg(not(feature = "memory-mem0"))]
    let _ = deps;
    tracing::warn!(
        target: LOG_TARGET,
        extension_id,
        "no memory provider is registered for this third-party extension id (or the `memory-mem0` feature is not compiled in); the document-store binding fails closed"
    );
    None
}

#[cfg(feature = "memory-mem0")]
fn create_mem0_provider(deps: &MemoryProviderDeps) -> Option<Arc<dyn MemoryService>> {
    let config = Mem0Config {
        app_id: deps.mem0.app_id.clone(),
    };

    // Test seam: a pre-built transport (mock) bypasses real `reqwest`
    // construction and the base-URL check (there is no URL to check).
    if let Some(transport) = deps.mem0_transport_override.clone() {
        return Some(Arc::new(Mem0MemoryService::new(transport, config)));
    }

    let Some(base_url) = deps.mem0.base_url.as_deref() else {
        tracing::warn!(
            target: LOG_TARGET,
            "mem0 memory binding selected but no base URL is set (MEMORY_MEM0_BASE_URL / [memory].mem0_base_url); failing closed"
        );
        return None;
    };
    // The API key is OPTIONAL: a self-hosted mem0 OSS server with
    // `AUTH_DISABLED=true` (the default local deployment) needs none, so an unset
    // `MEMORY_MEM0_API_KEY` is no longer fail-closed. When set, it is forwarded as
    // an `Authorization: Token <key>` header (the hosted cloud / an auth-enabled
    // self-hosted server).
    let api_key = deps.mem0.api_key.as_ref().map(|key| key.expose_secret());

    match Mem0HttpTransport::new(base_url, api_key) {
        Ok(transport) => Some(Arc::new(Mem0MemoryService::new(
            Arc::new(transport) as Arc<dyn Mem0Transport>,
            config,
        ))),
        Err(error) => {
            tracing::warn!(
                target: LOG_TARGET,
                %error,
                "failed to build the mem0 transport (rejected base URL or API key); failing closed"
            );
            None
        }
    }
}

/// Build the memory resolver from the (optional) binding policy and register the
/// third-party document-store provider the policy binds, if any.
///
/// At startup: config → policy → (here) factory builds the provider → registered
/// in the resolver → `resolve_document_store` returns it. Native is resolved
/// per-invocation by the resolver, so it is not built here. Fail-closed: a
/// permitted third-party binding whose provider cannot be built (missing creds /
/// rejected URL) is left unregistered, so `resolve_document_store` returns `None`
/// rather than silently using native.
pub fn build_memory_service_resolver(
    policy: Option<MemoryBindingPolicy>,
    deps: &MemoryProviderDeps,
) -> MemoryServiceResolver {
    let resolver = MemoryServiceResolver::from_optional_policy(policy.clone());

    // `None` policy = native default; nothing third-party to register.
    let Some(policy) = policy else {
        return resolver;
    };

    match policy.binding() {
        binding @ MemoryProviderBinding::ThirdParty { extension_id } => {
            match create_document_store_provider(binding, deps) {
                Some(provider) => resolver
                    .with_third_party_document_store_provider(extension_id.as_str(), provider),
                // create_* already logged why; fail closed.
                None => resolver,
            }
        }
        // Native / Disabled → the resolver already handles these.
        _ => resolver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ExtensionId;

    #[cfg(feature = "memory-mem0")]
    fn mem0_binding() -> MemoryProviderBinding {
        MemoryProviderBinding::ThirdParty {
            extension_id: ExtensionId::new(MEM0_MEMORY_EXTENSION_ID).unwrap(),
        }
    }

    #[test]
    fn native_binding_builds_native_when_a_filesystem_is_supplied() {
        let deps = MemoryProviderDeps {
            filesystem: Some(Arc::new(InMemoryBackend::new())),
            prompt_write_safety_sink: None,
            mem0: Mem0ConnectionConfig::default(),
            #[cfg(feature = "memory-mem0")]
            mem0_transport_override: None,
        };
        assert!(create_document_store_provider(&MemoryProviderBinding::Native, &deps).is_some());
    }

    #[test]
    fn native_binding_without_a_filesystem_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_document_store_provider(&MemoryProviderBinding::Native, &deps).is_none());
    }

    #[test]
    fn disabled_binding_is_none() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_document_store_provider(&MemoryProviderBinding::Disabled, &deps).is_none());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_without_credentials_fails_closed() {
        // The mem0 id is recognized, but with no base URL / API key and no
        // injected transport there is nothing to build → None (fail-closed).
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_document_store_provider(&mem0_binding(), &deps).is_none());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_with_a_blocked_base_url_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig {
            base_url: Some("https://169.254.169.254".to_string()),
            api_key: Some(SecretString::from("m0-key".to_string())),
            app_id: None,
        });
        assert!(create_document_store_provider(&mem0_binding(), &deps).is_none());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_with_real_connection_builds_a_provider() {
        // A well-formed base URL + key builds the real transport-backed provider.
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig {
            base_url: Some("https://mem0.example.com".to_string()),
            api_key: Some(SecretString::from("m0-key".to_string())),
            app_id: Some("ironclaw-test".to_string()),
        });
        assert!(create_document_store_provider(&mem0_binding(), &deps).is_some());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_with_a_local_base_url_and_no_key_builds_a_provider() {
        // The default self-hosted mem0 OSS deployment: a localhost base URL and NO
        // API key (the server runs with AUTH_DISABLED=true). The key is optional,
        // so this must build a provider rather than fail closed.
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig {
            base_url: Some("http://localhost:8888".to_string()),
            api_key: None,
            app_id: None,
        });
        assert!(create_document_store_provider(&mem0_binding(), &deps).is_some());
    }

    #[test]
    fn unknown_third_party_id_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        let binding = MemoryProviderBinding::ThirdParty {
            extension_id: ExtensionId::new("acme.honcho").unwrap(),
        };
        assert!(create_document_store_provider(&binding, &deps).is_none());
    }
}
