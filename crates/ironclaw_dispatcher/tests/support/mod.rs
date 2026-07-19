#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    DispatchError, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatchErrorKind,
    RuntimeExecutor,
};
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, MountView, ResourceScope, ResourceUsage, RuntimeKind, RuntimeLane,
    UserId, runtime_policy::NetworkMode,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};
use serde_json::Value;

/// Behavior a [`RecordingExecutor`] applies to a configured lane.
#[derive(Clone)]
pub enum LaneBehavior {
    /// Echo the request input back as the output.
    Echo,
    /// Return a fixed output value.
    Static(Value),
    /// Fail with the given redacted runtime error kind.
    Fail(RuntimeDispatchErrorKind),
}

/// One dispatch the executor was asked to route, captured for assertions.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedRequest {
    pub lane: RuntimeLane,
    pub provider: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub network_mode: NetworkMode,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: Option<UserId>,
    pub mounts: Option<MountView>,
    pub input: Value,
}

/// Shared test double implementing the closed [`RuntimeExecutor`] port.
///
/// Replaces the former per-file `RuntimeAdapter` doubles: lanes are configured
/// per `RuntimeKind` (mapped to their execution [`RuntimeLane`]), every routed
/// dispatch is recorded (including the resolved lane), and behavior is one of
/// echo / static / fail. Reserve+reconcile mirrors the production adapters so
/// resource-tally assertions hold.
#[derive(Clone)]
pub struct RecordingExecutor {
    lanes: HashMap<RuntimeLane, LaneBehavior>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl Default for RecordingExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingExecutor {
    pub fn new() -> Self {
        Self {
            lanes: HashMap::new(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn lane_for(runtime: RuntimeKind) -> RuntimeLane {
        RuntimeLane::from_runtime_kind(runtime)
            .expect("test lane requires a runtime kind that maps to an execution lane")
    }

    /// Configure `runtime`'s lane to echo the request input.
    pub fn echo(mut self, runtime: RuntimeKind) -> Self {
        self.lanes
            .insert(Self::lane_for(runtime), LaneBehavior::Echo);
        self
    }

    /// Configure `runtime`'s lane to return a fixed output.
    pub fn static_output(mut self, runtime: RuntimeKind, output: Value) -> Self {
        self.lanes
            .insert(Self::lane_for(runtime), LaneBehavior::Static(output));
        self
    }

    /// Configure `runtime`'s lane to fail with `kind`.
    pub fn failing(mut self, runtime: RuntimeKind, kind: RuntimeDispatchErrorKind) -> Self {
        self.lanes
            .insert(Self::lane_for(runtime), LaneBehavior::Fail(kind));
        self
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimeExecutor<DiskFilesystem, InMemoryResourceGovernor> for RecordingExecutor {
    fn supports_lane(&self, lane: RuntimeLane) -> bool {
        self.lanes.contains_key(&lane)
    }

    async fn dispatch_json(
        &self,
        lane: RuntimeLane,
        request: RuntimeAdapterRequest<'_, DiskFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let runtime = request.descriptor.runtime;
        self.requests.lock().unwrap().push(RecordedRequest {
            lane,
            provider: request.package.id.clone(),
            capability_id: request.capability_id.clone(),
            runtime,
            network_mode: request.runtime_policy.network_mode,
            scope: request.scope.clone(),
            authenticated_actor_user_id: request.authenticated_actor_user_id.clone(),
            mounts: request.mounts.clone(),
            input: request.input.clone(),
        });

        let Some(behavior) = self.lanes.get(&lane) else {
            return Err(DispatchError::MissingRuntimeBackend { runtime });
        };
        let output = match behavior {
            LaneBehavior::Echo => request.input.clone(),
            LaneBehavior::Static(value) => value.clone(),
            LaneBehavior::Fail(kind) => return Err(dispatch_error_for_runtime(runtime, *kind)),
        };

        let output_bytes = serde_json::to_vec(&output).unwrap().len() as u64;
        let usage = ResourceUsage::default()
            .set_output_bytes(output_bytes)
            .set_process_count(u32::from(matches!(
                runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )));
        let reservation = request
            .governor
            .reserve(request.scope.clone(), request.estimate.clone())
            .map_err(|_| dispatch_error_for_runtime(runtime, RuntimeDispatchErrorKind::Resource))?;
        let receipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| dispatch_error_for_runtime(runtime, RuntimeDispatchErrorKind::Resource))?;

        Ok(RuntimeAdapterResult {
            output,
            display_preview: None,
            output_bytes,
            usage,
            receipt,
        })
    }
}

/// Map a redacted error kind to the runtime's `DispatchError` variant, matching
/// the production `dispatch_error_for_runtime` shape.
pub fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            safe_summary: None,
        },
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::UnsupportedRuntime {
            capability: CapabilityId::new("system.unsupported").unwrap(),
            runtime,
        },
    }
}

pub fn legacy_capability_fixture_to_v2(manifest: &str) -> String {
    if manifest.contains("schema_version") {
        return manifest.to_string();
    }
    let mut converted = "schema_version = \"reborn.extension_manifest.v2\"\n".to_string();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("parameters_schema") {
            converted.push_str("visibility = \"model\"\n");
            converted.push_str("input_schema_ref = \"schemas/test/input.v1.json\"\n");
            converted.push_str("output_schema_ref = \"schemas/test/output.v1.json\"\n");
            converted.push_str("prompt_doc_ref = \"prompts/test.md\"\n");
        } else if trimmed.starts_with("backend =") {
            converted.push_str(&line.replacen("backend", "runner", 1));
            converted.push('\n');
        } else {
            converted.push_str(line);
            converted.push('\n');
        }
    }
    converted
}
