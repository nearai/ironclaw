use chrono_tz::Tz;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};
use serde_json::{Map, Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, FirstPartyCapabilityResult};

use super::memory::MemoryCapabilityState;
use super::{first_party_capability_manifest, input_error, resource_profile};

pub const PROFILE_SET_CAPABILITY_ID: &str = "builtin.profile_set";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        PROFILE_SET_CAPABILITY_ID,
        "Record a known structured fact about the user: timezone (IANA name), \
         locale (BCP-47), or location (free label). Use \
         whenever the user states one of these so future answers stay correct.",
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        PermissionMode::Allow,
        resource_profile(),
    )
}

/// Validate the closed field set into a JSON object to merge. Unknown fields and
/// invalid values are rejected here — this typed boundary is the authoritative
/// enforcement (the doc JSON-schema is defense-in-depth).
fn validated_fields(input: &Value) -> Result<Map<String, Value>, FirstPartyCapabilityError> {
    let obj = input.as_object().ok_or_else(input_error)?;
    let mut out = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "timezone" => {
                let s = value.as_str().ok_or_else(input_error)?;
                s.trim().parse::<Tz>().map_err(|_| input_error())?;
                out.insert("timezone".into(), json!(s.trim()));
            }
            "locale" => {
                let s = value.as_str().ok_or_else(input_error)?;
                // Light BCP-47 shape check: non-empty, ascii-alnum/hyphen.
                if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Err(input_error());
                }
                out.insert("locale".into(), json!(s));
            }
            "location" => {
                let s = value.as_str().ok_or_else(input_error)?;
                if s.is_empty() || s.chars().count() > 200 {
                    return Err(input_error());
                }
                out.insert("location".into(), json!(s));
            }
            // Closed surface: refuse unknown fields including any system-config fields
            // (e.g. always_approve, provider, model). This is the authoritative enforcement
            // per spec §9 and the plan's review-fix note on duplicate-truth.
            _ => return Err(input_error()),
        }
    }
    if out.is_empty() {
        return Err(input_error());
    }
    Ok(out)
}

pub(super) async fn dispatch(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let fields = validated_fields(&request.input)?;
    // Reuse the memory services/backend resolution; profile_merge_write keys the
    // doc via the shared profile_scope_and_path helper (agent=None, project=None).
    super::memory::profile_merge_write(state, request, fields).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
    };
    use ironclaw_memory::{
        FilesystemMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
        MemoryContext, RepositoryMemoryBackend,
    };
    use serde_json::{Value, json};

    use crate::{
        FirstPartyCapabilityRequest, InvocationServices, LocalHostProcessPort,
        first_party_tools::memory::MemoryCapabilityState,
        user_profile_source::profile_scope_and_path,
    };

    use super::*;

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            user_id: UserId::new("user-test").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-profile-set-test").unwrap()),
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
        let fs = Arc::new(InMemoryBackend::new());
        let scope = sample_scope();
        FirstPartyCapabilityRequest {
            capability_id: CapabilityId::new(PROFILE_SET_CAPABILITY_ID).unwrap(),
            scope,
            estimate: ironclaw_host_api::ResourceEstimate::default(),
            mounts: Some(memory_mount()),
            services: InvocationServices {
                filesystem: fs,
                runtime_http_egress: None,
                tool_call_http_egress: None,
                process: Arc::new(LocalHostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
            },
            input,
        }
    }

    /// Read back the profile document from the request's filesystem.
    async fn read_profile_doc(request: &FirstPartyCapabilityRequest) -> Value {
        let scope = sample_scope();
        let (doc_scope, path) =
            profile_scope_and_path(scope.tenant_id.as_str(), scope.user_id.as_str()).unwrap();
        let context = MemoryContext::new(doc_scope);
        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(
            &request.services.filesystem,
        )));
        let backend =
            RepositoryMemoryBackend::new(repository).with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                ..MemoryBackendCapabilities::default()
            });
        let bytes = backend
            .read_document(&context, &path)
            .await
            .expect("read_profile_doc: read failed")
            .expect("read_profile_doc: document not found");
        serde_json::from_slice(&bytes).expect("read_profile_doc: JSON parse failed")
    }

    #[tokio::test]
    async fn sets_timezone_and_persists() {
        let state = MemoryCapabilityState::default();
        let req = profile_set_request(json!({"timezone": "Asia/Tokyo"}));
        let result = dispatch(&state, &req).await.unwrap();
        assert_eq!(result.output["status"], "ok");

        let doc = read_profile_doc(&req).await;
        assert_eq!(doc["timezone"], "Asia/Tokyo", "timezone must be persisted");
    }

    #[tokio::test]
    async fn set_locale_after_timezone_merges_without_clobber() {
        let state = MemoryCapabilityState::default();

        // First call: set timezone
        let req1 = profile_set_request(json!({"timezone": "Asia/Tokyo"}));
        dispatch(&state, &req1).await.unwrap();

        // Second call on the SAME filesystem: set locale
        let req2 = profile_set_request(json!({"locale": "ja-JP"}));
        // Reuse the same filesystem so the second write can see the first
        let req2 = FirstPartyCapabilityRequest {
            services: crate::InvocationServices {
                filesystem: Arc::clone(&req1.services.filesystem),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                process: Arc::new(LocalHostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
            },
            ..req2
        };
        dispatch(&state, &req2).await.unwrap();

        let doc = read_profile_doc(&req2).await;
        assert_eq!(
            doc["timezone"], "Asia/Tokyo",
            "timezone must be preserved across the second write"
        );
        assert_eq!(doc["locale"], "ja-JP", "locale must be set");
    }

    #[tokio::test]
    async fn rejects_invalid_timezone() {
        let state = MemoryCapabilityState::default();
        let req = profile_set_request(json!({"timezone": "Pacific Time"}));
        let err = dispatch(&state, &req).await.unwrap_err();
        // Should be an InputEncode error from the closed validator
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "invalid timezone must produce InputEncode error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_unknown_field() {
        let state = MemoryCapabilityState::default();
        // System-config-style field must be refused by the closed surface.
        let req = profile_set_request(json!({"always_approve": true}));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "unknown field must produce InputEncode error, got: {err:?}"
        );
    }
}
