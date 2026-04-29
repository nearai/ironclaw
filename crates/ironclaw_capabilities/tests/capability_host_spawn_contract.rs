use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_spawns_authorized_process_without_dispatching_inline() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let authorizer = SpawnAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background"}),
        })
        .await
        .unwrap();

    assert!(!dispatcher.has_request());
    let start = process_manager.take_start();
    assert_eq!(start.scope, context.resource_scope);
    assert_eq!(start.capability_id, capability_id());
    assert_eq!(start.extension_id, ExtensionId::new("echo").unwrap());
    assert_eq!(start.runtime, RuntimeKind::Wasm);
    assert_eq!(start.input, json!({"message": "background"}));
    assert_eq!(result.process.process_id, start.process_id);
}

#[derive(Default)]
struct RecordingProcessManager {
    start: Mutex<Option<ProcessStart>>,
}

impl RecordingProcessManager {
    fn take_start(&self) -> ProcessStart {
        self.start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap()
    }
}

#[async_trait]
impl ProcessManager for RecordingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        *self
            .start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(start.clone());
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        })
    }
}

struct SpawnAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for SpawnAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}
