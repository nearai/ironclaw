//! End-to-end provider-swap proof for the mem0 memory provider (#3537 / #5264).
//!
//! Drives the exact build-time pipeline composition runs at startup —
//! `[memory]` config → `resolve_memory_binding_policy` →
//! `build_memory_service_resolver` (the config-driven factory, which constructs
//! the mem0 provider over its transport and registers it) → `MemoryServiceResolver`
//! → `resolve_document_store` → a write/search routed through the resolved
//! provider — and shows that, with `memory.document_store.v1` bound to the mem0
//! extension id (plus the production admin override an unverified third party
//! requires), the resolver yields the **mem0** provider and the calls reach the
//! mem0 transport, not the native filesystem store.
//!
//! The factory builds the provider over an injected in-memory `MockMem0Transport`
//! (no live mem0 endpoint), exercising the real config → policy → factory →
//! register → resolve path rather than hand-injecting the provider.
//!
//! Gated on `memory-mem0`: the provider it swaps in is compiled only under that
//! feature, so this proof runs with `--features memory-mem0` (the feature-off
//! build carries no mem0 code to swap).
#![cfg(feature = "memory-mem0")]

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope, TenantId, UserId};
use ironclaw_memory_mem0::{
    MEM0_MEMORY_EXTENSION_ID, Mem0Transport, MemoryInvocation, MemoryServiceSearchRequest,
    MemoryServiceWriteRequest, MockMem0Transport,
};
use ironclaw_reborn_composition::{
    Mem0ConnectionConfig, MemoryProviderDeps, RebornCompositionProfile,
    build_memory_service_resolver, resolve_memory_binding_policy,
};
use ironclaw_reborn_config::{MemoryAdminOverride, MemorySection};
use serde_json::json;

// Self-hosted mem0 OSS REST paths (no `/v1/` prefix; no trailing slash).
const ADD_PATH: &str = "/memories";
const SEARCH_PATH: &str = "/search";

fn invocation() -> MemoryInvocation {
    MemoryInvocation {
        scope: ResourceScope {
            tenant_id: TenantId::new("tenant-swap").unwrap(),
            user_id: UserId::new("user-swap").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        correlation_id: CorrelationId::new(),
    }
}

fn filesystem() -> Arc<dyn RootFilesystem> {
    Arc::new(InMemoryBackend::new())
}

fn write_request(target: &str, content: &str) -> MemoryServiceWriteRequest {
    MemoryServiceWriteRequest {
        target: target.to_string(),
        content: content.to_string(),
        append: true,
        old_string: None,
        new_string: None,
        replace_all: false,
        metadata: None,
        timezone: None,
    }
}

/// `[memory]` config binding the document-store profile to mem0, plus the
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
async fn config_binding_swaps_document_store_to_mem0_through_the_factory() {
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
    let resolver =
        build_memory_service_resolver(Some(policy), &deps_over_mock(Arc::clone(&transport)));

    // The document-store profile now resolves to the mem0 provider, NOT native.
    let provider = resolver
        .resolve_document_store(filesystem(), None)
        .expect("document-store binding must resolve to the mem0 provider");

    let write = provider
        .write(invocation(), write_request("notes/a.md", "swap me"))
        .await
        .expect("write through the swapped provider");
    assert_eq!(write.path, "notes/a.md");

    let search = provider
        .search(
            invocation(),
            MemoryServiceSearchRequest {
                query: "swapped".to_string(),
                limit: 5,
            },
        )
        .await
        .expect("search through the swapped provider");
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].content, "swapped hit");

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
    let resolver = build_memory_service_resolver(
        Some(policy),
        &MemoryProviderDeps::for_third_party(Mem0ConnectionConfig::default()),
    );
    assert!(
        resolver
            .resolve_document_store(filesystem(), None)
            .is_none()
    );
}

#[tokio::test]
async fn mem0_binding_with_a_local_connection_and_no_key_registers_a_provider() {
    // No transport override: the factory builds the real reqwest-backed provider
    // from the connection config and registers it, so the document-store profile
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
    let resolver = build_memory_service_resolver(Some(policy), &deps);
    assert!(
        resolver
            .resolve_document_store(filesystem(), None)
            .is_some(),
        "a local mem0 connection (no key) must register a provider for the binding"
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
    let resolver =
        build_memory_service_resolver(Some(policy), &deps_over_mock(Arc::clone(&transport)));

    let provider = resolver
        .resolve_document_store(filesystem(), None)
        .expect("local-dev mem0 binding resolves to the mem0 provider");
    provider
        .write(invocation(), write_request("notes/b.md", "dev swap"))
        .await
        .expect("write through the dev-swapped provider");
    assert_eq!(transport.count_path(ADD_PATH), 1);
}
