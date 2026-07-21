//! Finding C (§5.3.2, mutable-registry TOCTOU): dispatch must bind to the lane
//! the authorization pinned into the witness, not to whatever the hot registry
//! re-resolves at dispatch time.
//!
//! An extension update between authorize and dispatch can replace a descriptor's
//! runtime — moving it onto a DIFFERENT execution lane — while the mounts,
//! reservation, and trust prepared for the OLD descriptor still ride the request.
//! The dispatcher re-derives the descriptor + lane from a fresh
//! `SharedExtensionRegistry::snapshot()`, so it must compare that freshly-resolved
//! lane against the request's `pinned_lane` and fail CLOSED on a mismatch —
//! never executing a replacement runtime under authorization it did not receive.
//!
//! Full descriptor-content pinning (a same-lane descriptor swap) is tracked
//! separately in #6434; this contract locks the lane (runtime/executor) binding.

mod support;

use std::sync::Arc;

use crate::support::RecordingExecutor;

use ironclaw_dispatcher::*;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{
    ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::*;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use serde_json::json;

// Same extension id ("echo") and capability id ("echo.say") across both
// registries — only the `[runtime]` differs. This is exactly an extension
// update: the descriptor the authorization saw (WASM → lane Wasm) is replaced
// by a descriptor on a different lane (Script → lane Process).
const WASM_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
"#;

const SWAPPED_SCRIPT_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.2.0"
description = "Echo test extension (updated to a script runtime)"
trust = "untrusted"

[runtime]
kind = "script"
runner = "docker"
image = "example/echo:latest"
command = "cat"
args = []

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
"#;

fn registry(manifest: &str) -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(
        manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn dispatch_request(
    scope: &ResourceScope,
    pinned_lane: Option<RuntimeLane>,
) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        run_id: None,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: scope.clone(),
        authenticated_actor_user_id: None,
        estimate: ResourceEstimate::default()
            .set_concurrency_slots(1)
            .set_output_bytes(10_000),
        mounts: None,
        resource_reservation: None,
        pinned_lane,
        input: json!({"message": "hello"}),
    }
}

// A registry update that swaps the descriptor's runtime onto a DIFFERENT lane
// after the authorization pinned the original lane must fail closed with
// `LaneMismatch` and execute nothing.
#[tokio::test]
async fn registry_update_between_authorize_and_dispatch_fails_closed_on_lane_mismatch() {
    let shared = Arc::new(SharedExtensionRegistry::new(registry(WASM_MANIFEST)));
    let filesystem = Arc::new(DiskFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    // The executor supports BOTH lanes, so if the pinned-lane check were absent
    // the dispatch would happily execute on the swapped-in Process lane — the
    // test proves the check fails closed BEFORE that execution, not that the
    // lane merely happens to be unconfigured.
    let executor = RecordingExecutor::new()
        .echo(RuntimeKind::Wasm)
        .echo(RuntimeKind::Script);
    let events = InMemoryEventSink::new();

    let scope = sample_scope();
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::from_shared_registry(
        Arc::clone(&shared),
        Arc::clone(&filesystem),
        Arc::clone(&governor),
        executor.clone(),
    )
    .with_event_sink_arc(Arc::new(events.clone()));

    // Authorization pinned the WASM lane; now an extension update swaps echo.say
    // onto the Script runtime (Process lane) behind the shared registry.
    shared.replace(registry(SWAPPED_SCRIPT_MANIFEST));

    let error = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(RuntimeLane::Wasm)))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            DispatchError::LaneMismatch {
                authorized: RuntimeLane::Wasm,
                resolved: RuntimeLane::Process,
                ..
            }
        ),
        "swapped runtime lane must fail closed with LaneMismatch(Wasm→Process), got {error:?}"
    );
    assert_eq!(error.failure_kind(), DispatchFailureKind::LaneMismatch);

    // The executor must NOT have run the replacement runtime.
    assert!(
        executor.requests().is_empty(),
        "no execution may occur on a lane the authorization did not name"
    );

    // The dispatcher must have emitted the redacted failure event and NEVER a
    // RuntimeSelected/DispatchSucceeded for the swapped lane.
    let recorded = events.events();
    let kinds: Vec<_> = recorded.iter().map(|event| event.kind).collect();
    assert!(
        kinds.contains(&RuntimeEventKind::DispatchFailed),
        "a lane mismatch must emit DispatchFailed, got {kinds:?}"
    );
    assert!(
        !kinds.contains(&RuntimeEventKind::RuntimeSelected),
        "a lane mismatch must fail before RuntimeSelected, got {kinds:?}"
    );
    let failed = recorded
        .iter()
        .find(|event| event.kind == RuntimeEventKind::DispatchFailed)
        .unwrap();
    assert_eq!(failed.error_kind.as_deref(), Some("lane_mismatch"));
}

// Control: when no registry update happens, a request whose pinned lane matches
// the freshly-resolved lane dispatches normally — the pinning check does not
// break the common path.
#[tokio::test]
async fn matching_pinned_lane_dispatches_normally() {
    let shared = Arc::new(SharedExtensionRegistry::new(registry(WASM_MANIFEST)));
    let filesystem = Arc::new(DiskFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let executor = RecordingExecutor::new().echo(RuntimeKind::Wasm);

    let scope = sample_scope();
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();

    let dispatcher = RuntimeDispatcher::from_shared_registry(
        Arc::clone(&shared),
        Arc::clone(&filesystem),
        Arc::clone(&governor),
        executor.clone(),
    );

    let result = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(RuntimeLane::Wasm)))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(executor.requests().len(), 1);
    assert_eq!(executor.requests()[0].lane, RuntimeLane::Wasm);
}
