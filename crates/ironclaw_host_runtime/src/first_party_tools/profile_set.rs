use chrono_tz::Tz;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, ResourceUsage};
use ironclaw_memory::{
    MemoryBackend, MemoryContext, MemoryDocumentPath, MemoryWriteOutcome, content_bytes_sha256,
};
use ironclaw_turns::run_profile::Locale;
use serde_json::{Map, Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, FirstPartyCapabilityResult};

use super::memory::{
    MAX_MEMORY_PATCH_RETRIES, MemoryCapabilityState, ensure_memory_mount, write_options,
};
use super::{first_party_capability_manifest, input_error, operation_error, resource_profile};

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
                Locale::new(s).map_err(|_| input_error())?;
                out.insert("locale".into(), json!(s));
            }
            "location" => {
                let s = value.as_str().ok_or_else(input_error)?.trim();
                if s.is_empty() || s.chars().count() > 200 || s.len() > 800 {
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
    // Resolve backend/context/path from state+request, then run the CAS merge loop.
    profile_merge_write(state, request, fields).await
}

/// Outer function: resolves the backend, context, and profile path from
/// `state` and `request`, then delegates to the backend-independent
/// `profile_merge_into` CAS loop.
async fn profile_merge_write(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    use crate::user_profile_source::profile_scope_and_path;

    ensure_memory_mount(request, /* write */ true)?;
    let (scope, path) = profile_scope_and_path(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    )
    .map_err(|error| {
        tracing::debug!(%error, "profile_set scope construction failed");
        input_error()
    })?;
    let context = MemoryContext::new(scope).with_audit_context(
        request.scope.clone(),
        ironclaw_host_api::CorrelationId::new(),
    );
    let backend = state.backend_for(request)?;
    profile_merge_into(&*backend, &context, &path, fields).await
}

/// Inner function: CAS retry loop over an already-resolved backend/context/path.
/// Separated from `profile_merge_write` so tests can inject a fake backend
/// directly without needing to construct a full `FirstPartyCapabilityRequest`.
pub(super) async fn profile_merge_into(
    backend: &dyn MemoryBackend,
    context: &MemoryContext,
    path: &MemoryDocumentPath,
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let options = write_options(None);

    // CAS retry loop — mirrors `patch_document`'s MAX_MEMORY_PATCH_RETRIES pattern.
    // Read current bytes + hash, merge fields, compare-and-write; retry on hash
    // mismatch (a concurrent writer raced in).
    for _ in 0..MAX_MEMORY_PATCH_RETRIES {
        let current = backend
            .read_document(context, path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "profile_set read_document failed");
                operation_error()
            })?;
        let expected_hash = current.as_deref().map(content_bytes_sha256);
        let mut doc: serde_json::Map<String, serde_json::Value> = match &current {
            Some(bytes) => match serde_json::from_slice(bytes) {
                Ok(map) => map,
                Err(error) => {
                    // FIX-1: corrupt-JSON fail-loud — refuse to overwrite unknown content.
                    tracing::debug!(%error, "profile doc is not valid JSON; refusing to overwrite");
                    return Err(operation_error());
                }
            },
            None => serde_json::Map::new(),
        };
        // Refuse to overwrite a doc whose KNOWN fields are type-corrupt. The reader
        // (`ProfileJson`) hard-fails its typed parse on a non-string known field, so
        // silently merging onto it would brick the profile to None on every future
        // load. Fail loud instead (consistent with the corrupt-JSON guard above).
        for key in ["timezone", "locale", "location"] {
            if let Some(value) = doc.get(key)
                && !value.is_string()
            {
                tracing::debug!(
                    field = key,
                    "profile doc has a non-string known field; refusing to overwrite"
                );
                return Err(operation_error());
            }
        }
        for (k, v) in &fields {
            doc.insert(k.clone(), v.clone());
        }
        let bytes =
            serde_json::to_vec(&serde_json::Value::Object(doc)).map_err(|_| operation_error())?;

        let outcome = backend
            .compare_and_write_document_with_backend_options(
                context,
                path,
                expected_hash.as_deref(),
                &bytes,
                &options,
            )
            .await
            .map_err(|error| {
                tracing::debug!(%error, "profile_set compare_and_write failed");
                operation_error()
            })?;
        if outcome == MemoryWriteOutcome::Written {
            return Ok(FirstPartyCapabilityResult::new(
                json!({ "status": "ok" }),
                ResourceUsage::default(),
            ));
        }
        // else: hash moved under us — loop and re-merge onto the newer doc.
    }
    // FIX-2: CAS-exhaustion debug log.
    tracing::debug!(
        retries = MAX_MEMORY_PATCH_RETRIES,
        "profile merge CAS retries exhausted"
    );
    Err(operation_error())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_filesystem::{FilesystemError, InMemoryBackend};
    use ironclaw_host_api::{
        CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
    };
    use ironclaw_memory::{
        FilesystemMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
        MemoryBackendWriteOptions, MemoryContext, MemoryDocumentPath, MemoryWriteOutcome,
        RepositoryMemoryBackend,
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

    #[tokio::test]
    async fn rejects_non_object_input() {
        let state = MemoryCapabilityState::default();
        // Input must be a JSON object; a plain string must be rejected.
        let req = profile_set_request(json!("timezone"));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "non-object input must produce InputEncode error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_empty_object_input() {
        let state = MemoryCapabilityState::default();
        // An empty object contains no valid fields and must be rejected.
        let req = profile_set_request(json!({}));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "empty object must produce InputEncode error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_invalid_locale() {
        let state = MemoryCapabilityState::default();
        // A locale with a space is not valid BCP-47 (must be ascii-alnum/hyphen only).
        let req = profile_set_request(json!({"locale": "en US"}));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "locale with space must produce InputEncode error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn profile_set_rejects_empty_or_whitespace_only_location() {
        // validated_fields trims location and rejects if the result is empty.
        // Verify both an empty string and a whitespace-only string are rejected
        // at the dispatch level.
        for input in [json!({"location": ""}), json!({"location": "   "})] {
            let state = MemoryCapabilityState::default();
            let req = profile_set_request(input.clone());
            let err = dispatch(&state, &req).await.unwrap_err();
            assert!(
                matches!(
                    err.kind(),
                    Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
                ),
                "location {:?} must produce InputEncode error, got: {err:?}",
                input
            );
        }
    }

    #[tokio::test]
    async fn rejects_too_long_locale() {
        let state = MemoryCapabilityState::default();
        // A 36-character locale exceeds the 35-character limit enforced by Locale::new.
        let too_long = "a".repeat(36);
        let req = profile_set_request(json!({"locale": too_long}));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "36-char locale must produce InputEncode error via Locale::new, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn location_at_200_chars_ok_201_rejected() {
        let state = MemoryCapabilityState::default();

        // A 200-character location (exactly at the limit) must succeed.
        let location_200: String = "A".repeat(200);
        let req = profile_set_request(json!({"location": location_200}));
        let result = dispatch(&state, &req).await.unwrap();
        assert_eq!(
            result.output["status"], "ok",
            "200-char location must succeed"
        );
        let doc = read_profile_doc(&req).await;
        assert_eq!(
            doc["location"].as_str().map(|s| s.len()),
            Some(200),
            "200-char location must be persisted"
        );

        // A 201-character location (one past the limit) must be rejected.
        let state2 = MemoryCapabilityState::default();
        let location_201: String = "A".repeat(201);
        let req2 = profile_set_request(json!({"location": location_201}));
        let err = dispatch(&state2, &req2).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::InputEncode)
            ),
            "201-char location must produce InputEncode error, got: {err:?}"
        );
    }

    // ── CAS-exhaustion coverage ───────────────────────────────────────────────

    /// A fake `MemoryBackend` whose `compare_and_write_document_with_backend_options`
    /// always returns `Conflict`, simulating a write that is perpetually raced.
    /// `read_document` returns a stable empty-object document so the merge loop
    /// can parse it on every iteration without error.
    struct AlwaysConflictBackend {
        attempt_count: Arc<AtomicUsize>,
    }

    impl AlwaysConflictBackend {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let counter = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    attempt_count: Arc::clone(&counter),
                },
                counter,
            )
        }
    }

    #[async_trait]
    impl MemoryBackend for AlwaysConflictBackend {
        fn capabilities(&self) -> MemoryBackendCapabilities {
            MemoryBackendCapabilities {
                file_documents: true,
                ..MemoryBackendCapabilities::default()
            }
        }

        async fn read_document(
            &self,
            _context: &MemoryContext,
            _path: &MemoryDocumentPath,
        ) -> Result<Option<Vec<u8>>, FilesystemError> {
            // Return a stable, valid empty-object JSON document.
            Ok(Some(b"{}".to_vec()))
        }

        async fn compare_and_write_document_with_backend_options(
            &self,
            _context: &MemoryContext,
            _path: &MemoryDocumentPath,
            _expected_previous_hash: Option<&str>,
            _bytes: &[u8],
            _backend_options: &MemoryBackendWriteOptions,
        ) -> Result<MemoryWriteOutcome, FilesystemError> {
            self.attempt_count.fetch_add(1, Ordering::Relaxed);
            // Always report a hash conflict so the caller retries.
            Ok(MemoryWriteOutcome::Conflict)
        }
    }

    #[tokio::test]
    async fn refuses_write_when_existing_known_field_is_corrupt() {
        let state = MemoryCapabilityState::default();
        // Seed a corrupt profile doc ({"timezone": 123}) on the request's filesystem.
        let req = profile_set_request(json!({"locale": "en-US"}));
        let scope = sample_scope();
        let (doc_scope, path) =
            profile_scope_and_path(scope.tenant_id.as_str(), scope.user_id.as_str()).unwrap();
        let context = MemoryContext::new(doc_scope);
        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(
            &req.services.filesystem,
        )));
        let backend =
            RepositoryMemoryBackend::new(repository).with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                ..MemoryBackendCapabilities::default()
            });
        backend
            .write_document_with_backend_options(
                &context,
                &path,
                br#"{"timezone": 123}"#,
                &MemoryBackendWriteOptions::default(),
            )
            .await
            .expect("seed corrupt doc");

        // A profile_set write must refuse rather than merge onto the corruption.
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::OperationFailed)
            ),
            "corrupt known field must produce OperationFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn dispatch_rejects_non_json_existing_profile_document() {
        let state = MemoryCapabilityState::default();
        // Seed a non-JSON document (raw bytes) at the profile scope/path on the request's filesystem.
        let req = profile_set_request(json!({"locale": "en-US"}));
        let scope = sample_scope();
        let (doc_scope, path) =
            profile_scope_and_path(scope.tenant_id.as_str(), scope.user_id.as_str()).unwrap();
        let context = MemoryContext::new(doc_scope);
        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(
            &req.services.filesystem,
        )));
        let backend =
            RepositoryMemoryBackend::new(repository).with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                ..MemoryBackendCapabilities::default()
            });
        backend
            .write_document_with_backend_options(
                &context,
                &path,
                b"this is not json at all",
                &MemoryBackendWriteOptions::default(),
            )
            .await
            .expect("seed non-JSON doc");

        // A profile_set write must fail closed rather than overwrite unknown content.
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::OperationFailed)
            ),
            "non-JSON existing profile document must produce OperationFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn profile_merge_into_returns_err_after_cas_budget_exhausted() {
        use crate::user_profile_source::profile_scope_and_path;

        let (backend, attempt_counter) = AlwaysConflictBackend::new();
        let (scope, path) = profile_scope_and_path("tenant-cas-test", "user-cas-test").unwrap();
        let context = MemoryContext::new(scope);
        let mut fields = serde_json::Map::new();
        fields.insert("timezone".into(), json!("UTC"));

        let err = profile_merge_into(&backend, &context, &path, fields)
            .await
            .unwrap_err();

        assert!(
            matches!(
                err.kind(),
                Some(ironclaw_host_api::RuntimeDispatchErrorKind::OperationFailed)
            ),
            "CAS exhaustion must produce OperationFailed, got: {err:?}"
        );
        assert_eq!(
            attempt_counter.load(Ordering::Relaxed),
            super::MAX_MEMORY_PATCH_RETRIES,
            "must attempt exactly MAX_MEMORY_PATCH_RETRIES times before giving up"
        );
    }
}
