use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, ResourceUsage};
use ironclaw_memory::MemoryServiceProfileSetRequest;
use serde_json::json;

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, FirstPartyCapabilityResult};

use super::memory::{
    MemoryCapabilityState, ensure_memory_mount, invocation_for_request, map_memory_service_error,
};
use super::{first_party_capability_manifest, resource_profile};

pub const PROFILE_SET_CAPABILITY_ID: &str = "builtin.profile_set";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        PROFILE_SET_CAPABILITY_ID,
        "Record a private, local fact about the user's agent context — timezone \
         (IANA name), locale (BCP-47), or location (free label). Use this \
         (not memory_write) whenever the user states one of these so future \
         answers stay correct. This is a private local write, not a public \
         profile; it is unrelated to builtin.trace_commons.profile_set.",
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) async fn dispatch(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    // Validate the profile fields first, then authorize the `/memory` write —
    // matching the pre-lift ordering (`validated_fields` ran before
    // `ensure_memory_mount`).
    let profile_request = MemoryServiceProfileSetRequest::from_tool_input(&request.input)
        .map_err(map_memory_service_error)?;
    ensure_memory_mount(request, /* write */ true)?;
    let service = state.service_for(request)?;
    let response = service
        .profile_set(invocation_for_request(request), profile_request)
        .await
        .map_err(map_memory_service_error)?;
    Ok(FirstPartyCapabilityResult::new(
        json!({ "status": response.status }),
        ResourceUsage::default(),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
    };
    use ironclaw_memory::{
        MemoryInvocation, MemoryProfileSetStatus, MemoryService, MemoryServiceError,
        MemoryServiceProfileSetResponse,
    };
    use serde_json::{Map, Value, json};

    use crate::{FirstPartyCapabilityRequest, HostProcessPort, InvocationServices};

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingProfileMemoryService {
        seen: Mutex<Vec<(MemoryInvocation, Map<String, Value>)>>,
    }

    #[async_trait]
    impl MemoryService for RecordingProfileMemoryService {
        async fn profile_set(
            &self,
            invocation: MemoryInvocation,
            request: MemoryServiceProfileSetRequest,
        ) -> Result<MemoryServiceProfileSetResponse, MemoryServiceError> {
            self.seen
                .lock()
                .expect("recording profile service lock should not be poisoned")
                .push((invocation, request.fields));
            Ok(MemoryServiceProfileSetResponse {
                status: MemoryProfileSetStatus::Ok,
            })
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-profile-service").unwrap(),
            user_id: UserId::new("user-profile-service").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-profile-service").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn memory_mount() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/memory").unwrap(),
            VirtualPath::new("/memory").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap()
    }

    fn profile_set_request(input: Value) -> FirstPartyCapabilityRequest {
        profile_set_request_with_mounts(input, Some(memory_mount()))
    }

    fn profile_set_request_with_mounts(
        input: Value,
        mounts: Option<MountView>,
    ) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest {
            run_id: None,
            origin: None,
            capability_id: CapabilityId::new(PROFILE_SET_CAPABILITY_ID).unwrap(),
            scope: sample_scope(),
            authenticated_actor_user_id: None,
            estimate: ironclaw_host_api::ResourceEstimate::default(),
            mounts,
            services: InvocationServices {
                filesystem: Arc::new(InMemoryBackend::new()),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                runtime_secret_material_stager: None,
                process: Arc::new(HostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
                post_edit_check: None,
            },
            input,
        }
    }

    #[tokio::test]
    async fn profile_set_dispatches_closed_fields_through_memory_service_facade() {
        let memory_service = Arc::new(RecordingProfileMemoryService::default());
        let state = MemoryCapabilityState::with_memory_service_for_test(memory_service.clone());
        let request = profile_set_request(json!({"timezone": "Asia/Tokyo"}));

        let result = dispatch(&state, &request)
            .await
            .expect("profile_set should write through IronClaw memory facade");

        assert_eq!(result.output["status"], "ok");
        let seen = memory_service
            .seen
            .lock()
            .expect("recording profile service lock should not be poisoned");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.scope.tenant_id.as_str(), "tenant-profile-service");
        assert_eq!(seen[0].0.scope.user_id.as_str(), "user-profile-service");
        assert_eq!(seen[0].1["timezone"], "Asia/Tokyo");
    }

    #[tokio::test]
    async fn profile_set_rejects_unknown_fields_before_memory_service_side_effect() {
        let memory_service = Arc::new(RecordingProfileMemoryService::default());
        let state = MemoryCapabilityState::with_memory_service_for_test(memory_service.clone());
        let request = profile_set_request(json!({"always_approve": true}));

        let err = dispatch(&state, &request)
            .await
            .expect_err("unknown field should be rejected");

        assert!(matches!(
            err.kind(),
            Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
        ));
        let seen = memory_service
            .seen
            .lock()
            .expect("recording profile service lock should not be poisoned");
        assert!(seen.is_empty(), "rejected profile_set must not call facade");
    }
}
