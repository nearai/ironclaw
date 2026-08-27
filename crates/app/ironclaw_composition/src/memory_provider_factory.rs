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
//! [`resolve_memory_provider`] is the build-time wiring used at startup:
//! resolve policy → build + register a bound third-party provider → load the
//! BOUND provider's manifest bundle (its registrable package + declared
//! lifecycle). Production third-party bindings stay fail-closed/override-gated
//! by the upstream [`MemoryBindingPolicy`]; this factory never relaxes that —
//! it only constructs a provider for a binding the policy already permitted.

use std::num::NonZeroU32;
use std::sync::Arc;

use ironclaw_extension_contracts::memory::{
    MemoryDescriptor, MemoryLifecycleHook, MemoryScheduledTrigger,
};
use ironclaw_extension_registry::ExtensionPackage;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_runtime::memory_binding::{MemoryBindingPolicy, MemoryProviderBinding};
use ironclaw_host_runtime::memory_context::ProductionMemoryPromptContextService;
use ironclaw_host_runtime::memory_native_extension as memory_extension;
use ironclaw_host_runtime::memory_provider::MemoryServiceResolver;
use ironclaw_host_runtime::{
    FirstPartyCapabilityHandler, MemoryBackedUserProfileSource, NativeMemoryToolHandler,
};
use ironclaw_loop_contracts::MemoryPromptContextService;
use ironclaw_loop_host::HostUserProfileSource;
use ironclaw_memory::{MemoryService, PromptWriteSafetyEventSink};
#[cfg(all(test, feature = "memory-mem0"))]
use ironclaw_memory_mem0::MEM0_MEMORY_EXTENSION_ID;
#[cfg(feature = "memory-mem0")]
use ironclaw_memory_mem0::{Mem0Config, Mem0HttpTransport, Mem0MemoryService, Mem0Transport};
#[cfg(test)]
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

/// Build the provider for a resolved [`MemoryProviderBinding`],
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
#[cfg(test)]
pub(crate) fn create_provider(
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

#[cfg(test)]
fn create_third_party_provider(
    extension_id: &str,
    deps: &MemoryProviderDeps,
) -> Option<Arc<dyn MemoryService>> {
    #[cfg(feature = "memory-mem0")]
    if extension_id == MEM0_MEMORY_EXTENSION_ID {
        return create_mem0_provider(deps).map(|provider| provider as Arc<dyn MemoryService>);
    }
    // No provider is registered for this third-party id — or the `memory-mem0`
    // feature is not compiled in — so the memory binding fails closed.
    #[cfg(not(feature = "memory-mem0"))]
    let _ = deps;
    tracing::warn!(
        target: LOG_TARGET,
        extension_id,
        "no memory provider is registered for this third-party extension id (or the `memory-mem0` feature is not compiled in); the memory binding fails closed"
    );
    None
}

#[cfg(feature = "memory-mem0")]
fn create_mem0_provider(deps: &MemoryProviderDeps) -> Option<Arc<Mem0MemoryService>> {
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

/// The fully resolved memory provider for this runtime's lifetime.
///
/// The **bound provider's manifest is the single source of truth** for the
/// memory surface: `package` is that provider's registrable always-on package
/// (its `[[tools]]` are what the model sees; `None` ⇒ NO memory tools are
/// registered at all), and `lifecycle` is the hook set its `[memory]` section
/// declares (empty ⇒ the host never initiates a memory call). `resolver` is
/// what the registered tools build their `MemoryService` through at dispatch.
#[derive(Clone)]
pub struct ResolvedMemoryProvider {
    pub resolver: MemoryServiceResolver,
    pub package: Option<ExtensionPackage>,
    pub lifecycle: MemoryDescriptor,
    /// The bound provider's first-party tool handler, registered for exactly
    /// the capability ids `package` declares. `None` ⇒ no memory tools are
    /// dispatchable (matching their absence from the surface).
    pub tool_handler: Option<Arc<dyn FirstPartyCapabilityHandler>>,
    /// The bound provider's own memory guidance for the model (#7185),
    /// resolved from its bundle. `None` when unbound or the provider
    /// declares no `guidance_doc`.
    pub guidance: Option<String>,
    /// Resolved prompt text for each scheduled pass op the bound provider
    /// declares, by trigger (#7664). Empty when unbound or the provider
    /// schedules nothing. The cadence and tool selection stay on `lifecycle`;
    /// only the asset needed resolving, and it resolved fail-closed.
    pub scheduled_pass_prompts: Vec<(MemoryScheduledTrigger, String)>,
}

impl ResolvedMemoryProvider {
    fn unbound(resolver: MemoryServiceResolver) -> Self {
        Self {
            resolver,
            package: None,
            lifecycle: MemoryDescriptor::default(),
            tool_handler: None,
            guidance: None,
            scheduled_pass_prompts: Vec::new(),
        }
    }
}

/// Resolve the bound memory provider from the (optional) binding policy: build
/// and register a third-party provider instance when one is bound, and load
/// the BOUND provider's manifest bundle (package + declared lifecycle).
///
/// At startup: config → policy → (here) factory builds the provider →
/// registered in the resolver → `resolve_provider` returns it. Native is
/// resolved per-invocation by the resolver, so only its bundle is loaded here.
/// Fail-closed: a disabled binding, an unknown third-party id, or a permitted
/// third-party binding whose provider cannot be built (missing creds /
/// rejected URL) yields NO package and an EMPTY lifecycle — no memory tools
/// are advertised and no lifecycle hook is ever called, rather than
/// advertising tools that fail at call time.
pub fn resolve_memory_provider(
    policy: Option<MemoryBindingPolicy>,
    deps: &MemoryProviderDeps,
) -> Result<ResolvedMemoryProvider, crate::RebornBuildError> {
    // `deps` feeds only third-party provider construction (mem0 today); with
    // every provider feature off, no arm consumes it.
    #[cfg(not(feature = "memory-mem0"))]
    let _ = deps;
    let binding = policy
        .as_ref()
        .map(|policy| policy.binding().clone())
        .unwrap_or_default();
    let resolver = MemoryServiceResolver::from_optional_policy(policy);
    match binding {
        MemoryProviderBinding::Native => {
            let bundle = memory_extension::native_memory_provider_bundle().map_err(|error| {
                crate::RebornBuildError::InvalidConfig {
                    reason: format!("native memory provider package is invalid: {error}"),
                }
            })?;
            let tool_handler: Arc<dyn FirstPartyCapabilityHandler> =
                Arc::new(NativeMemoryToolHandler::from_package(&bundle.package));
            Ok(ResolvedMemoryProvider {
                resolver,
                package: Some(bundle.package),
                lifecycle: bundle.lifecycle,
                tool_handler: Some(tool_handler),
                guidance: bundle.guidance,
                scheduled_pass_prompts: bundle.scheduled_pass_prompts,
            })
        }
        MemoryProviderBinding::Disabled => Ok(ResolvedMemoryProvider::unbound(resolver)),
        MemoryProviderBinding::ThirdParty { extension_id } => {
            #[cfg(feature = "memory-mem0")]
            if extension_id.as_str() == memory_extension::MEM0_MEMORY_EXTENSION_ID {
                // The CONCRETE provider instance backs both halves: coerced
                // into the lifecycle resolver, and held typed by mem0's tool
                // handler (which calls its inherent tool operations).
                return Ok(match create_mem0_provider(deps) {
                    Some(provider) => {
                        let bundle = memory_extension::mem0_memory_provider_bundle(
                            ironclaw_memory_mem0::MEMORY_ASSETS,
                        )
                        .map_err(|error| {
                            crate::RebornBuildError::InvalidConfig {
                                reason: format!("mem0 memory provider package is invalid: {error}"),
                            }
                        })?;
                        let tool_handler: Arc<dyn FirstPartyCapabilityHandler> =
                            Arc::new(Mem0MemoryToolHandler {
                                provider: Arc::clone(&provider),
                            });
                        ResolvedMemoryProvider {
                            resolver: resolver.with_third_party_provider(
                                extension_id.as_str(),
                                provider as Arc<dyn MemoryService>,
                            ),
                            package: Some(bundle.package),
                            lifecycle: bundle.lifecycle,
                            tool_handler: Some(tool_handler),
                            guidance: bundle.guidance,
                            scheduled_pass_prompts: bundle.scheduled_pass_prompts,
                        }
                    }
                    // create_mem0_provider already logged why; fail closed —
                    // no tools registered, no lifecycle hooks called.
                    None => ResolvedMemoryProvider::unbound(resolver),
                });
            }
            // Unknown third-party id, or the bound provider's feature is not
            // compiled in: fail closed with no tools and no lifecycle hooks.
            tracing::warn!(
                target: LOG_TARGET,
                extension_id = extension_id.as_str(),
                "no memory provider is registered for this third-party extension id (or its \
                 provider feature is not compiled in); the memory binding fails closed"
            );
            Ok(ResolvedMemoryProvider::unbound(resolver))
        }
    }
}

/// mem0's first-party tool handler — PURE BEHAVIOR. Lives in composition (the
/// one layer allowed to name the mem0 crate) while the tool logic itself is
/// mem0's inherent operations; the wire shapes come from `ironclaw_memory`'s
/// shared output helpers. The host-owned `MemoryToolGuard` wrapped around it
/// at registration already enforced the manifest-derived mount authority and
/// input normalization, and bounds the output.
#[cfg(feature = "memory-mem0")]
#[derive(Debug)]
struct Mem0MemoryToolHandler {
    provider: Arc<Mem0MemoryService>,
}

#[cfg(feature = "memory-mem0")]
#[async_trait::async_trait]
impl FirstPartyCapabilityHandler for Mem0MemoryToolHandler {
    async fn dispatch(
        &self,
        request: ironclaw_host_runtime::FirstPartyCapabilityRequest,
    ) -> Result<
        ironclaw_host_runtime::FirstPartyCapabilityResult,
        ironclaw_host_runtime::FirstPartyCapabilityError,
    > {
        use ironclaw_host_api::dispatch::RuntimeDispatchErrorKind;
        use ironclaw_host_runtime::{
            FirstPartyCapabilityError, finish_memory_tool_result, map_memory_service_error,
            memory_invocation_for_request,
        };
        use ironclaw_memory::{
            MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
            MEMORY_WRITE_CAPABILITY_ID, MemoryServiceProfileSetRequest, MemoryServiceReadRequest,
            MemoryServiceSearchRequest, MemoryServiceTreeRequest, MemoryServiceWriteRequest,
            PROFILE_SET_CAPABILITY_ID, profile_set_response_output, read_response_output,
            search_response_output, tree_response_output, write_response_output,
        };

        let start = std::time::Instant::now();
        let invocation = memory_invocation_for_request(&request);
        let output = match request.capability_id.as_str() {
            PROFILE_SET_CAPABILITY_ID => {
                let parsed = MemoryServiceProfileSetRequest::from_tool_input(&request.input)
                    .map_err(map_memory_service_error)?;
                let response = self
                    .provider
                    .profile_set(invocation, parsed)
                    .await
                    .map_err(map_memory_service_error)?;
                profile_set_response_output(response)
            }
            MEMORY_SEARCH_CAPABILITY_ID => {
                let parsed = MemoryServiceSearchRequest::from_tool_input(&request.input)
                    .map_err(map_memory_service_error)?;
                let response = self
                    .provider
                    .search(invocation, parsed)
                    .await
                    .map_err(map_memory_service_error)?;
                search_response_output(response)
            }
            MEMORY_WRITE_CAPABILITY_ID => {
                let parsed = MemoryServiceWriteRequest::from_tool_input(&request.input)
                    .map_err(map_memory_service_error)?;
                let response = self
                    .provider
                    .write(invocation, parsed)
                    .await
                    .map_err(map_memory_service_error)?;
                write_response_output(response)
            }
            MEMORY_READ_CAPABILITY_ID => {
                let parsed = MemoryServiceReadRequest::from_tool_input(&request.input)
                    .map_err(map_memory_service_error)?;
                let response = self
                    .provider
                    .read(invocation, parsed)
                    .await
                    .map_err(map_memory_service_error)?;
                read_response_output(response)
            }
            MEMORY_TREE_CAPABILITY_ID => {
                let parsed = MemoryServiceTreeRequest::from_tool_input(&request.input)
                    .map_err(map_memory_service_error)?;
                let response = self
                    .provider
                    .tree(invocation, parsed)
                    .await
                    .map_err(map_memory_service_error)?;
                tree_response_output(response)
            }
            // Declared but not implemented by this provider: fail closed.
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OperationFailed,
                ));
            }
        };
        finish_memory_tool_result(output, start)
    }
}

/// Adapter implementing the loop-host profile trait over
/// [`MemoryBackedUserProfileSource`]. Lives here (not in `ironclaw_host_runtime`)
/// because `ironclaw_loop_host` owns the trait and already depends on
/// `ironclaw_host_runtime` — composition is the layer that can see both.
pub(crate) struct MemoryBackedUserProfileSourceAdapter(pub(crate) MemoryBackedUserProfileSource);

#[async_trait::async_trait]
impl HostUserProfileSource for MemoryBackedUserProfileSourceAdapter {
    async fn resolve_user_profile(
        &self,
        run_context: &ironclaw_loop_contracts::LoopRunContext,
    ) -> Option<ironclaw_loop_contracts::UserProfileContext> {
        self.0.resolve_user_profile(run_context).await
    }
}

/// The three host consumers of the bound memory provider, derived from the
/// provider and its DECLARED lifecycle set.
///
/// - `memory_context_service` — the prompt-context adapter (itself queries
///   only the declared retrieval lanes; `None` when no provider is bound).
/// - `after_turn_memory_writer` — `None` unless `record_interaction` is
///   declared, so the after-turn seam is skipped entirely.
/// - `user_profile_source` — `None` unless `profile_read` is declared; the
///   caller substitutes the empty source.
///
/// One derivation, shared by runtime assembly and the integration harness, so
/// the lifecycle gating cannot fork between production and its tests.
pub struct MemoryLifecycleConsumers {
    pub memory_context_service: Option<Arc<dyn MemoryPromptContextService>>,
    pub after_turn_memory_writer: Option<Arc<dyn MemoryService>>,
    pub user_profile_source: Option<Arc<dyn HostUserProfileSource>>,
}

/// Derive the host-side memory consumers from the resolved provider + its
/// declared lifecycle. An undeclared hook is never wired, so it is never
/// called (F3/F8).
pub fn memory_lifecycle_consumers(
    provider: Option<Arc<dyn MemoryService>>,
    lifecycle: &MemoryDescriptor,
) -> MemoryLifecycleConsumers {
    let memory_context_service = provider.clone().map(|provider| {
        Arc::new(ProductionMemoryPromptContextService::new(
            provider,
            lifecycle.clone(),
        )) as Arc<dyn MemoryPromptContextService>
    });
    let after_turn_memory_writer = provider
        .clone()
        .filter(|_| lifecycle.declares(MemoryLifecycleHook::RecordInteraction));
    let user_profile_source = provider
        .filter(|_| lifecycle.declares(MemoryLifecycleHook::ProfileRead))
        .map(|provider| {
            Arc::new(MemoryBackedUserProfileSourceAdapter(
                MemoryBackedUserProfileSource::new(provider),
            )) as Arc<dyn HostUserProfileSource>
        });
    MemoryLifecycleConsumers {
        memory_context_service,
        after_turn_memory_writer,
        user_profile_source,
    }
}

/// The after-turn cadence a deployment actually runs the bound provider's
/// declared upkeep at, or an operator-facing reason there is nothing to run.
///
/// The DECLARATION decides whether scheduled upkeep happens at all: a provider
/// that declares an `after_turn` op gets one, a provider that declares none
/// gets nothing. Config is an override of the cadence, not a switch — `Ok(None)`
/// here means "no override, use the declared interval", `Ok(Some)` means "the
/// operator set a different one".
///
/// `Err` is the one dishonest combination: an operator configured an interval
/// while the bound provider schedules nothing. Silently ignoring it would leave
/// a deployment believing memory is being tidied while nothing ever runs, so
/// the build fails naming the bound provider.
///
/// This replaced the binding-shaped gate (only the host-bundled native
/// provider was known to serve the standing-document REPLACEMENT a curation
/// pass performs). The declaration now carries that: a provider only schedules
/// the upkeep it can actually perform. A provider that declares a pass whose
/// writes its own backend rejects is a provider lying in its own manifest.
pub(crate) fn scheduled_after_turn_interval_override(
    interval_turns: Option<NonZeroU32>,
    lifecycle: &MemoryDescriptor,
    binding: Option<&MemoryProviderBinding>,
) -> Result<Option<NonZeroU32>, String> {
    if lifecycle
        .scheduled_op(MemoryScheduledTrigger::AfterTurn)
        .is_some()
    {
        return Ok(interval_turns);
    }
    if interval_turns.is_none() {
        return Ok(None);
    }
    let provider = match binding {
        Some(MemoryProviderBinding::Native) => {
            "the host-bundled native memory provider".to_string()
        }
        Some(MemoryProviderBinding::ThirdParty { extension_id }) => {
            extension_id.as_str().to_string()
        }
        Some(MemoryProviderBinding::Disabled) => "memory is disabled".to_string(),
        None => "no memory provider is assembled".to_string(),
    };
    Err(format!(
        "[memory].curation_interval_turns is set, but the bound memory provider ({provider}) \
         declares no after_turn scheduled op, so there is no upkeep to run at that interval. \
         Bind a memory provider that declares one, or remove \
         [memory].curation_interval_turns."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::ExtensionId;

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
        assert!(create_provider(&MemoryProviderBinding::Native, &deps).is_some());
    }

    #[test]
    fn native_binding_without_a_filesystem_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_provider(&MemoryProviderBinding::Native, &deps).is_none());
    }

    #[test]
    fn disabled_binding_is_none() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_provider(&MemoryProviderBinding::Disabled, &deps).is_none());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_without_credentials_fails_closed() {
        // The mem0 id is recognized, but with no base URL / API key and no
        // injected transport there is nothing to build → None (fail-closed).
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        assert!(create_provider(&mem0_binding(), &deps).is_none());
    }

    #[cfg(feature = "memory-mem0")]
    #[test]
    fn mem0_binding_with_a_blocked_base_url_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig {
            base_url: Some("https://169.254.169.254".to_string()),
            api_key: Some(SecretString::from("m0-key".to_string())),
            app_id: None,
        });
        assert!(create_provider(&mem0_binding(), &deps).is_none());
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
        assert!(create_provider(&mem0_binding(), &deps).is_some());
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
        assert!(create_provider(&mem0_binding(), &deps).is_some());
    }

    #[test]
    fn unknown_third_party_id_fails_closed() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        let binding = MemoryProviderBinding::ThirdParty {
            extension_id: ExtensionId::new("acme.honcho").unwrap(),
        };
        assert!(create_provider(&binding, &deps).is_none());
    }

    struct InertMemoryService;
    impl MemoryService for InertMemoryService {}

    fn inert_provider() -> Arc<dyn MemoryService> {
        Arc::new(InertMemoryService)
    }

    /// F3/F8 regression at the production wiring seam: a lifecycle hook the
    /// bound provider's manifest does not declare is never WIRED — the
    /// after-turn writer and profile source exist only when their hooks are
    /// declared, and no provider wires nothing at all. (The context adapter
    /// is wired whenever a provider exists; it gates its two retrieval lanes
    /// internally — covered by the host-runtime caller tests.)
    #[test]
    fn lifecycle_consumers_wire_only_declared_hooks() {
        let unbound = memory_lifecycle_consumers(None, &MemoryDescriptor::default());
        assert!(unbound.memory_context_service.is_none());
        assert!(unbound.after_turn_memory_writer.is_none());
        assert!(unbound.user_profile_source.is_none());

        let empty =
            memory_lifecycle_consumers(Some(inert_provider()), &MemoryDescriptor::default());
        assert!(
            empty.memory_context_service.is_some(),
            "the context adapter self-gates its lanes"
        );
        assert!(empty.after_turn_memory_writer.is_none());
        assert!(empty.user_profile_source.is_none());

        let full = memory_lifecycle_consumers(
            Some(inert_provider()),
            &MemoryDescriptor {
                lifecycle: ironclaw_extension_contracts::memory::MemoryLifecycleHook::ALL.to_vec(),
                ..MemoryDescriptor::default()
            },
        );
        assert!(full.memory_context_service.is_some());
        assert!(full.after_turn_memory_writer.is_some());
        assert!(full.user_profile_source.is_some());
    }

    fn interval(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("test interval is non-zero")
    }

    /// A descriptor declaring one after-turn pass, parsed the way a manifest
    /// carries it so the fixture cannot drift from what the parser accepts.
    fn descriptor_declaring_an_after_turn_pass() -> MemoryDescriptor {
        toml::from_str(
            r#"
lifecycle = ["read_long_term"]

[[scheduled_ops]]
trigger = "after_turn"
interval_turns = 10
pass = { prompt = "prompts/memory_curation.md", tools = ["ironclaw.memory.read"], max_model_calls = 10 }
"#,
        )
        .expect("the declaration parses")
    }

    /// The DECLARATION decides whether upkeep runs; config only re-times it.
    /// A provider that declares an op needs no operator opt-in, and an
    /// operator who set an interval gets that interval instead of the
    /// declared one.
    #[test]
    fn a_declared_op_runs_at_its_own_cadence_unless_config_overrides_it() {
        let declaring = descriptor_declaring_an_after_turn_pass();
        assert_eq!(
            scheduled_after_turn_interval_override(
                None,
                &declaring,
                Some(&MemoryProviderBinding::Native)
            ),
            Ok(None),
            "no interval configured: the armed declaration stays dormant (opt-in)"
        );
        assert_eq!(
            scheduled_after_turn_interval_override(
                Some(interval(4)),
                &declaring,
                Some(&MemoryProviderBinding::Native)
            ),
            Ok(Some(interval(4))),
            "a configured interval overrides the declared one"
        );
    }

    /// Fail closed, loudly. An operator who configured a cadence for upkeep the
    /// bound provider does not declare gets a startup error naming the provider
    /// — not a quiet deployment that believes memory is being tidied while
    /// nothing is scheduled at all.
    #[test]
    fn a_configured_interval_without_a_declared_op_fails_the_build() {
        let silent = MemoryDescriptor::default();
        let third_party = MemoryProviderBinding::ThirdParty {
            extension_id: ExtensionId::new("some-provider").expect("valid extension id"),
        };
        for (label, binding) in [
            ("third party", Some(&third_party)),
            ("disabled", Some(&MemoryProviderBinding::Disabled)),
            ("no memory runtime", None),
            ("native", Some(&MemoryProviderBinding::Native)),
        ] {
            let error =
                scheduled_after_turn_interval_override(Some(interval(10)), &silent, binding)
                    .expect_err(&format!("{label} must not silently accept an interval"));
            assert!(
                error.contains("curation_interval_turns"),
                "{label}: the message must name the setting to change: {error}"
            );
        }
        let named =
            scheduled_after_turn_interval_override(Some(interval(10)), &silent, Some(&third_party))
                .expect_err("a provider declaring nothing is rejected");
        assert!(
            named.contains("some-provider"),
            "the message must name the bound provider: {named}"
        );
        // No interval configured against a provider that declares nothing is
        // silence, not an error: nobody asked for anything.
        for binding in [
            Some(&third_party),
            Some(&MemoryProviderBinding::Disabled),
            None,
        ] {
            assert_eq!(
                scheduled_after_turn_interval_override(None, &silent, binding),
                Ok(None)
            );
        }
    }

    /// Binding-shape regression for the resolved provider: `Disabled` and an
    /// unknown third party register NO package and an EMPTY lifecycle (no
    /// tools advertised, no hooks called); native registers its package with
    /// the full declared lifecycle.
    #[test]
    fn resolve_memory_provider_native_binds_package_and_lifecycle() {
        let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default());
        let native = resolve_memory_provider(None, &deps).expect("native default resolves");
        let package = native.package.expect("native registers its package");
        assert_eq!(package.manifest.id.as_str(), "ironclaw.memory");
        assert!(!native.lifecycle.lifecycle.is_empty());

        let policy = MemoryBindingPolicy::resolve(
            ironclaw_host_runtime::memory_binding::MemoryBindingInput {
                provider: Some("memory.disabled".to_string()),
                ..ironclaw_host_runtime::memory_binding::MemoryBindingInput::native_default(
                    ironclaw_host_runtime::memory_binding::MemoryDeploymentProfile::Standalone,
                )
            },
        )
        .expect("disabled policy resolves");
        let disabled = resolve_memory_provider(Some(policy), &deps).expect("disabled resolves");
        assert!(
            disabled.package.is_none(),
            "Disabled registers NO memory tools"
        );
        assert!(disabled.lifecycle.lifecycle.is_empty());

        let policy = MemoryBindingPolicy::resolve(
            ironclaw_host_runtime::memory_binding::MemoryBindingInput {
                provider: Some("acme.honcho".to_string()),
                ..ironclaw_host_runtime::memory_binding::MemoryBindingInput::native_default(
                    ironclaw_host_runtime::memory_binding::MemoryDeploymentProfile::Standalone,
                )
            },
        )
        .expect("third-party policy resolves");
        let unknown = resolve_memory_provider(Some(policy), &deps).expect("unknown resolves");
        assert!(unknown.package.is_none());
        assert!(unknown.lifecycle.lifecycle.is_empty());
    }
}
