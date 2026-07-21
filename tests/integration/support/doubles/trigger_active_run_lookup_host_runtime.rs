/// #5886 harness-wiring seam: routes ONLY `builtin.trigger_list` dispatch to
/// a second, small `HostRuntime` built with a REAL `TriggerActiveRunLookup`
/// (see `assembly::local_dev_trigger_only_host_runtime`), while every other
/// capability keeps going through `inner` (the harness's normal runtime,
/// whose baked-in lookup is scoped to a turn-state store the group's real
/// runs never write to — `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test`
/// explains why). Both runtimes speak the SAME `HostRuntime` trait, so this
/// reuses the real `TriggerManagementToolHandler::dispatch` code path
/// (`ironclaw_host_runtime::builtin_first_party_handlers_with_trigger_create_hook`)
/// rather than reimplementing `active_hold` derivation in test code.
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityAuthResumeRequest,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeStatusRequest, VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface,
};

pub(crate) struct TriggerActiveRunLookupHostRuntime {
    inner: Arc<dyn HostRuntime>,
    trigger_runtime: Arc<dyn HostRuntime>,
    trigger_list_capability_id: CapabilityId,
}

impl TriggerActiveRunLookupHostRuntime {
    pub(crate) fn new(
        inner: Arc<dyn HostRuntime>,
        trigger_runtime: Arc<dyn HostRuntime>,
        trigger_list_capability_id: CapabilityId,
    ) -> Self {
        Self {
            inner,
            trigger_runtime,
            trigger_list_capability_id,
        }
    }
}

#[async_trait]
impl HostRuntime for TriggerActiveRunLookupHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        if request.capability_id == self.trigger_list_capability_id {
            self.trigger_runtime.invoke_capability(request).await
        } else {
            self.inner.invoke_capability(request).await
        }
    }

    // `trigger_list` is `PermissionMode::Allow` (no gate, no spawn/resume/
    // auth-resume path), so every other entry point forwards to `inner`
    // unconditionally — mirrors `ParkingHostRuntime`'s forwarding shape.
    async fn spawn_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.spawn_capability(request).await
    }

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_capability(request).await
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeCapabilityAuthResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.auth_resume_capability(request).await
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_spawn_capability(request).await
    }

    async fn visible_capabilities(
        &self,
        request: RuntimeVisibleCapabilityRequest,
    ) -> Result<RuntimeVisibleCapabilitySurface, HostRuntimeError> {
        self.inner.visible_capabilities(request).await
    }

    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        self.inner.cancel_work(request).await
    }

    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        self.inner.runtime_status(request).await
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        self.inner.health().await
    }
}
