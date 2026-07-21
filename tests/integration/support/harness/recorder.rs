use std::{path::PathBuf, sync::Arc};

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CapabilityId, ResourceScope, RuntimeHttpEgressRequest};
use ironclaw_network::{NetworkHttpRequest, NetworkTransportRequest};
use ironclaw_turns::{GateRef, run_profile::CapabilityInvocation};

use super::super::doubles::RecordingTestCapabilityPort;
use super::{HarnessResult, HostRuntimeCapabilityHarness};

#[derive(Debug, Clone)]
pub struct RecordedCapabilityResult {
    pub capability_id: CapabilityId,
    pub output: serde_json::Value,
}

#[derive(Clone)]
pub(crate) enum HarnessCapabilityRecorder {
    Recording(Arc<RecordingTestCapabilityPort>),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl HarnessCapabilityRecorder {
    pub(crate) fn invocations(&self) -> Vec<CapabilityInvocation> {
        match self {
            Self::Recording(port) => port.invocations(),
            Self::HostRuntime(harness) => harness.invocations(),
        }
    }

    pub(crate) fn workspace_file_path(&self, relative: &str) -> Option<PathBuf> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => Some(harness.workspace_file_path(relative)),
        }
    }

    pub(crate) fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.capability_results(),
        }
    }

    /// E-PROFILE: local-dev memory filesystem backing the user-profile source, if any.
    /// `None` for the Echo backend and HostRuntime harnesses without a profile filesystem.
    pub(crate) fn profile_filesystem(&self) -> Option<Arc<dyn RootFilesystem>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.profile_filesystem_for_test(),
        }
    }

    /// E-SKILL: the `HostSkillContextSource` to wire as the runtime's `skill_context_source`, if any.
    /// `None` for the Echo backend and HostRuntime harnesses without skill activation.
    pub(crate) fn skill_context_source(
        &self,
    ) -> Option<Arc<dyn ironclaw_loop_host::HostSkillContextSource>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.skill_context_source_for_test(),
        }
    }

    /// C-ATTACH: the attachment read port + inbound lander, if any. `None` for the
    /// Echo backend and HostRuntime harnesses without a local-dev workspace filesystem.
    pub(crate) fn attachment_test_support(
        &self,
    ) -> Option<ironclaw_reborn_composition::AttachmentTestSupport> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.attachment_test_support_for_test(),
        }
    }

    pub(crate) fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.runtime_http_requests(),
        }
    }

    /// See [`HostRuntimeCapabilityHarness::process_commands`]; empty for the
    /// Echo recording backend.
    pub(crate) fn recorded_process_commands(&self) -> Vec<String> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.process_commands(),
        }
    }

    pub(crate) fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.network_http_requests(),
        }
    }

    pub(crate) fn install_network_response_script(
        &self,
        status: u16,
        body: Vec<u8>,
    ) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no network response script".into())
            }
            Self::HostRuntime(harness) => harness.install_network_response_script(status, body),
        }
    }

    /// S1 seam: requests that reached the real-egress-pipeline's wire-level
    /// transport recorder. Empty for every backend but
    /// `BuiltinHttpToolsRealEgress`.
    pub(crate) fn real_egress_transport_requests(&self) -> Vec<NetworkTransportRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.real_egress_transport_requests(),
        }
    }

    pub(crate) async fn approve_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev approvals".into())
            }
            Self::HostRuntime(harness) => harness.approve_local_dev_gate(gate_ref).await,
        }
    }

    pub(crate) async fn deny_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev approvals".into())
            }
            Self::HostRuntime(harness) => harness.deny_local_dev_gate(gate_ref).await,
        }
    }

    pub(crate) async fn disable_auto_approve_for(&self, scope: ResourceScope) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev auto-approve settings".into())
            }
            Self::HostRuntime(harness) => harness.disable_global_auto_approve(scope).await,
        }
    }

    pub(crate) async fn enable_auto_approve_for(&self, scope: ResourceScope) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev auto-approve settings".into())
            }
            Self::HostRuntime(harness) => harness.enable_global_auto_approve(scope).await,
        }
    }

    pub(crate) fn approval_requests_store(
        &self,
    ) -> Option<Arc<dyn ironclaw_run_state::ApprovalRequestStore>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.approval_requests_store(),
        }
    }
}
