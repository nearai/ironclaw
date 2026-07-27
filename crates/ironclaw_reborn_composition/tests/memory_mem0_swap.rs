//! End-to-end provider-swap proof for the mem0 memory provider (#3537 / #5264).
//!
//! Drives the exact build-time pipeline composition runs at startup —
//! `[memory]` config → `resolve_memory_binding_policy` →
//! `resolve_memory_provider` (the config-driven factory, which constructs
//! the mem0 provider over its transport, loads mem0's manifest bundle, and
//! builds mem0's tool handler) → `register_memory_tool_handler` (the exact
//! registration `factory.rs` performs) → registry-routed tool dispatch —
//! and shows that, with the memory binding pointed at the mem0 extension id
//! (plus the production admin override an unverified third party requires),
//! the manifest-declared tools route to the **mem0** transport, not the
//! native filesystem store, and the lifecycle resolver yields the mem0
//! provider.
//!
//! The factory builds the provider over an injected in-memory `MockMem0Transport`
//! (no live mem0 endpoint), exercising the real config → policy → factory →
//! register → dispatch path rather than hand-injecting the provider.
//!
//! Gated on `memory-mem0`: the provider it swaps in is compiled only under that
//! feature, so this proof runs with `--features memory-mem0` (the feature-off
//! build carries no mem0 code to swap).
#![cfg(feature = "memory-mem0")]

use std::sync::Arc;

use ironclaw_host_api::{
    CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    TenantId, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, register_memory_tool_handler,
};
use ironclaw_memory::{MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use ironclaw_memory_mem0::{MEM0_MEMORY_EXTENSION_ID, Mem0Transport, MockMem0Transport};
use ironclaw_reborn_composition::{
    Mem0ConnectionConfig, MemoryProviderDeps, RebornCompositionProfile, ResolvedMemoryProvider,
    resolve_memory_binding_policy, resolve_memory_provider,
};
use ironclaw_reborn_config::{MemoryAdminOverride, MemorySection};
use serde_json::{Value, json};

// Self-hosted mem0 OSS REST paths (no `/v1/` prefix; no trailing slash).
const ADD_PATH: &str = "/memories";
const SEARCH_PATH: &str = "/search";

fn filesystem() -> Arc<ironclaw_filesystem::InMemoryBackend> {
    Arc::new(ironclaw_filesystem::InMemoryBackend::new())
}

fn memory_mount() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/memory").unwrap(),
        VirtualPath::new("/memory").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap()
}

/// A loop-shaped capability request for a memory tool: scoped, carrying the
/// `/memory` mount authority and a request filesystem (which mem0's remote
/// store never touches — that's the point of the swap proof).
fn tool_request(capability_id: &str, input: Value) -> FirstPartyCapabilityRequest {
    let mut request = FirstPartyCapabilityRequest::request_for_test(
        CapabilityId::new(capability_id).unwrap(),
        ResourceScope {
            tenant_id: TenantId::new("tenant-swap").unwrap(),
            user_id: UserId::new("user-swap").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        input,
        None,
    );
    request.mounts = Some(memory_mount());
    request
}

/// Register the resolved provider's tool handler exactly the way
/// `factory.rs` does at startup, returning the production-shaped registry.
fn registry_for(resolved: &ResolvedMemoryProvider) -> FirstPartyCapabilityRegistry {
    let package = resolved
        .package
        .as_ref()
        .expect("bound provider must carry its package");
    let handler = resolved
        .tool_handler
        .as_ref()
        .expect("bound provider must carry its tool handler");
    let mut registry = FirstPartyCapabilityRegistry::new();
    register_memory_tool_handler(&mut registry, package, Arc::clone(handler));
    registry
}

async fn dispatch_tool(
    registry: &FirstPartyCapabilityRegistry,
    capability_id: &str,
    input: Value,
) -> Value {
    registry
        .get(&CapabilityId::new(capability_id).unwrap())
        .unwrap_or_else(|| panic!("bound manifest must register `{capability_id}`"))
        .dispatch(tool_request(capability_id, input))
        .await
        .unwrap_or_else(|error| panic!("`{capability_id}` dispatch failed: {error:?}"))
        .output
}

/// `[memory]` config binding memory to mem0, plus the
/// production admin override an unverified third-party provider requires.
fn mem0_section() -> MemorySection {
    MemorySection {
        provider: Some(MEM0_MEMORY_EXTENSION_ID.to_string()),
        admin_overrides: vec![MemoryAdminOverride {
            extension_id: MEM0_MEMORY_EXTENSION_ID.to_string(),
            deployment_profile: "production".to_string(),
        }],
        ..Default::default()
    }
}

/// Factory deps that build the mem0 provider over a mock transport (the test
/// seam) instead of a real reqwest client — no base URL / API key needed.
fn deps_over_mock(transport: Arc<MockMem0Transport>) -> MemoryProviderDeps {
    MemoryProviderDeps {
        filesystem: None,
        prompt_write_safety_sink: None,
        mem0: Mem0ConnectionConfig::default(),
        mem0_transport_override: Some(transport as Arc<dyn Mem0Transport>),
    }
}

#[tokio::test]
async fn config_binding_swaps_the_memory_provider_to_mem0_through_the_factory() {
    let transport = Arc::new(MockMem0Transport::always_ok(json!({
        "results": [
            { "id": "m-1", "memory": "swapped hit", "metadata": { "target": "notes/a.md" } }
        ]
    })));

    // config → policy → factory builds the mem0 provider (over the mock) and
    // registers it in the resolver.
    let policy =
        resolve_memory_binding_policy(Some(&mem0_section()), RebornCompositionProfile::Production)
            .expect("mem0 binding resolves with the production override");
    let resolved = resolve_memory_provider(Some(policy), &deps_over_mock(Arc::clone(&transport)))
        .expect("the bound mem0 provider resolves");

    // The bound provider's manifest is the single source of truth: binding
    // mem0 registers MEM0's package (its four stable ironclaw.memory.* tools)
    // and its honest lifecycle — the long-term retrieval lane and profile
    // reads only.
    let package = resolved
        .package
        .as_ref()
        .expect("binding mem0 must register mem0's package");
    assert_eq!(package.manifest.id.as_str(), MEM0_MEMORY_EXTENSION_ID);
    use ironclaw_host_api::MemoryLifecycleHook;
    assert!(
        resolved
            .lifecycle
            .declares(MemoryLifecycleHook::ReadLongTerm)
    );
    assert!(
        resolved
            .lifecycle
            .declares(MemoryLifecycleHook::ProfileRead)
    );
    assert!(
        !resolved
            .lifecycle
            .declares(MemoryLifecycleHook::ReadShortTerm)
    );
    assert!(
        !resolved
            .lifecycle
            .declares(MemoryLifecycleHook::RecordInteraction)
    );

    // The lifecycle binding now resolves to the mem0 provider, NOT native.
    assert!(
        resolved
            .resolver
            .resolve_provider(filesystem(), None)
            .is_some(),
        "memory binding must resolve to the mem0 provider for the lifecycle lanes"
    );

    // The manifest-declared tools, registered exactly the way `factory.rs`
    // registers the bound provider at startup, route through the host guard to
    // mem0's handler.
    let registry = registry_for(&resolved);

    let write = dispatch_tool(
        &registry,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({"target": "notes/a.md", "content": "swap me", "append": true}),
    )
    .await;
    assert_eq!(write["path"], "notes/a.md");

    let search = dispatch_tool(
        &registry,
        MEMORY_SEARCH_CAPABILITY_ID,
        json!({"query": "swapped", "limit": 5}),
    )
    .await;
    assert_eq!(search["result_count"], 1);
    assert_eq!(search["results"][0]["content"], "swapped hit");

    // The write and search actually reached mem0's REST surface (POST add +
    // POST search), proving the swap routed to mem0 rather than the native
    // filesystem store — which would never touch this transport.
    assert_eq!(transport.count_path(ADD_PATH), 1, "one mem0 add (write)");
    assert_eq!(
        transport.count_path(SEARCH_PATH),
        1,
        "one mem0 search (search)"
    );
}

#[tokio::test]
async fn mem0_binding_without_connection_or_transport_fails_closed() {
    // Same binding + override, but the factory has no transport override and no
    // base URL / API key, so it cannot build the provider: nothing is registered
    // and the resolver fails closed rather than silently using native.
    let policy =
        resolve_memory_binding_policy(Some(&mem0_section()), RebornCompositionProfile::Production)
            .expect("policy resolves");
    let resolved = resolve_memory_provider(
        Some(policy),
        &MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default()),
    )
    .expect("an unbuildable binding still resolves (to nothing)");
    assert!(
        resolved
            .resolver
            .resolve_provider(filesystem(), None)
            .is_none()
    );
    // Fail closed all the way: no package is registered (the model sees NO
    // memory tools), no tool handler exists for the registry, and no lifecycle
    // hook is ever called.
    assert!(resolved.package.is_none());
    assert!(resolved.tool_handler.is_none());
    assert!(resolved.lifecycle.lifecycle.is_empty());
}

#[tokio::test]
async fn mem0_binding_with_a_local_connection_and_no_key_registers_a_provider() {
    // No transport override: the factory builds the real reqwest-backed provider
    // from the connection config and registers it, so the memory binding
    // resolves to mem0. This is the default self-hosted mem0 OSS deployment — a
    // localhost base URL and NO API key (the server runs with AUTH_DISABLED=true).
    let policy =
        resolve_memory_binding_policy(Some(&mem0_section()), RebornCompositionProfile::Production)
            .expect("policy resolves");
    let deps = MemoryProviderDeps::for_third_party(Mem0ConnectionConfig {
        base_url: Some("http://localhost:8888".to_string()),
        api_key: None,
        app_id: None,
    });
    let resolved =
        resolve_memory_provider(Some(policy), &deps).expect("a local mem0 connection resolves");
    assert!(
        resolved
            .resolver
            .resolve_provider(filesystem(), None)
            .is_some(),
        "a local mem0 connection (no key) must register a provider for the binding"
    );
    assert!(
        resolved.package.is_some(),
        "a constructible mem0 binding registers mem0's tool package"
    );
    assert!(
        resolved.tool_handler.is_some(),
        "a constructible mem0 binding carries mem0's tool handler for the registry"
    );
}

#[test]
fn mem0_binding_in_production_requires_an_admin_override() {
    // Without the override, a production deployment refuses to bind an unverified
    // third-party memory provider at all — the swap is gated, not free.
    let section = MemorySection {
        provider: Some(MEM0_MEMORY_EXTENSION_ID.to_string()),
        admin_overrides: Vec::new(),
        ..Default::default()
    };
    let resolved =
        resolve_memory_binding_policy(Some(&section), RebornCompositionProfile::Production);
    assert!(
        resolved.is_err(),
        "production must reject an unverified third-party binding without an override"
    );
}

#[tokio::test]
async fn local_dev_swaps_to_mem0_without_an_override() {
    // In local-dev the third-party binding is permitted without an override, so
    // the same factory registration yields the mem0 provider.
    let section = MemorySection {
        provider: Some(MEM0_MEMORY_EXTENSION_ID.to_string()),
        admin_overrides: Vec::new(),
        ..Default::default()
    };
    let policy = resolve_memory_binding_policy(Some(&section), RebornCompositionProfile::LocalDev)
        .expect("local-dev allows the third-party binding without an override");
    let transport = Arc::new(MockMem0Transport::always_ok(json!({ "id": "m-1" })));
    let resolved = resolve_memory_provider(Some(policy), &deps_over_mock(Arc::clone(&transport)))
        .expect("local-dev mem0 binding resolves");

    assert!(
        resolved
            .resolver
            .resolve_provider(filesystem(), None)
            .is_some(),
        "local-dev mem0 binding resolves to the mem0 provider"
    );
    let registry = registry_for(&resolved);
    dispatch_tool(
        &registry,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({"target": "notes/b.md", "content": "dev swap", "append": true}),
    )
    .await;
    assert_eq!(transport.count_path(ADD_PATH), 1);
}
