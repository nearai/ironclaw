use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_common::normalize_mime_type;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_filesystem::{FileType, FilesystemError, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityId, DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError,
    PermissionMode, ResourceUsage, RuntimeDispatchErrorKind, ScopedPath,
};
use ironclaw_outbound::{OutboundError, ReplyAttachmentIntent, ReplyAttachmentIntentPort};
use serde::Deserialize;
use serde_json::json;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{first_party_capability_manifest, resource_profile};

pub const ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID: &str =
    "builtin.attach_workspace_file_to_reply";

const DESCRIPTION: &str = "Attach a file that already exists under /workspace to the final assistant reply for the current run. Use this after creating a file the user should receive through any supported channel. This registers bounded metadata only; it does not send the file immediately.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachWorkspaceFileInput {
    path: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID,
        DESCRIPTION,
        vec![EffectKind::ReadFilesystem, EffectKind::ExternalWrite],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    intent_port: Arc<dyn ReplyAttachmentIntentPort>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID)?,
        Arc::new(AttachWorkspaceFileHandler { intent_port }),
    );
    Ok(())
}

pub(super) fn insert_unavailable_handler(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    insert_handler(registry, Arc::new(UnavailableReplyAttachmentIntentPort))
}

struct AttachWorkspaceFileHandler {
    intent_port: Arc<dyn ReplyAttachmentIntentPort>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for AttachWorkspaceFileHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let input: AttachWorkspaceFileInput =
            serde_json::from_value(request.input).map_err(|_| invalid_input("path"))?;
        let run_id = request.run_id.ok_or_else(|| {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "reply attachment registration requires an active run",
            )
        })?;
        let path = ScopedPath::new(input.path).map_err(|_| invalid_input("path"))?;
        if path
            .as_str()
            .strip_prefix("/workspace/")
            .is_none_or(|relative| relative.is_empty())
        {
            return Err(invalid_input("path"));
        }
        let mounts = request.mounts.ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        })?;
        let filesystem =
            ScopedFilesystem::with_fixed_view(Arc::clone(&request.services.filesystem), mounts);
        let stat = filesystem
            .stat(&request.scope, &path)
            .await
            .map_err(map_filesystem_error)?;
        if stat.file_type != FileType::File {
            return Err(invalid_input("path"));
        }
        if stat.len > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 {
            return Err(attachment_too_large());
        }
        let bytes = filesystem
            .read_bytes_bounded(
                &request.scope,
                &path,
                DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes,
            )
            .await
            .map_err(map_filesystem_error)?
            .ok_or_else(attachment_too_large)?;
        let read_size = u64::try_from(bytes.len()).map_err(|_| attachment_too_large())?;
        if read_size != stat.len {
            return Err(FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "workspace file changed while preparing the reply attachment",
            ));
        }
        drop(bytes);

        let filename = input
            .filename
            .unwrap_or_else(|| default_filename(&path).to_string());
        let mime_type = input
            .mime_type
            .map(|mime_type| {
                let normalized = normalize_mime_type(&mime_type);
                if normalized == mime_type {
                    Ok(normalized)
                } else {
                    Err(invalid_input("mime_type"))
                }
            })
            .transpose()?
            .unwrap_or_else(|| default_mime_type(&filename).to_string());
        let intent = ReplyAttachmentIntent {
            path,
            filename,
            mime_type,
            size_bytes: stat.len,
        };
        intent.validate().map_err(map_intent_validation_error)?;
        self.intent_port
            .register(&request.scope, &run_id, intent.clone())
            .await
            .map_err(map_intent_port_error)?;

        Ok(FirstPartyCapabilityResult::new(
            json!({
                "attached": true,
                "path": intent.path,
                "filename": intent.filename,
                "mime_type": intent.mime_type,
                "size_bytes": intent.size_bytes,
            }),
            ResourceUsage::default(),
        ))
    }
}

fn default_filename(path: &ScopedPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or("attachment")
}

fn default_mime_type(filename: &str) -> &'static str {
    let extension = filename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn map_filesystem_error(error: FilesystemError) -> FirstPartyCapabilityError {
    tracing::debug!(error = %error, "reply attachment filesystem operation failed");
    match error {
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::Contract(_)
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => invalid_input("path"),
        _ => FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend),
    }
}

fn map_intent_validation_error(error: OutboundError) -> FirstPartyCapabilityError {
    tracing::debug!(error = %error, "reply attachment intent validation failed");
    match error {
        OutboundError::ReplyAttachmentIntentLimitExceeded => attachment_too_large(),
        OutboundError::InvalidRequest { .. } | OutboundError::ReplyAttachmentIntentConflict => {
            invalid_input("filename")
        }
        OutboundError::AccessDenied => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        }
        _ => FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend),
    }
}

fn map_intent_port_error(error: OutboundError) -> FirstPartyCapabilityError {
    tracing::debug!(error = %error, "reply attachment intent persistence failed");
    match error {
        OutboundError::ReplyAttachmentIntentsSealed => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                "reply attachments are already sealed for this run",
            )
        }
        OutboundError::ReplyAttachmentIntentConflict => invalid_input("path"),
        OutboundError::ReplyAttachmentIntentLimitExceeded => attachment_too_large(),
        OutboundError::InvalidRequest { .. } => invalid_input("path"),
        OutboundError::AccessDenied => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
        }
        OutboundError::Backend
        | OutboundError::Serialization
        | OutboundError::CasConflict
        | OutboundError::PreferenceTargetMissing { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::DeliveryNotFound => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
    }
}

fn attachment_too_large() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::OperationFailed,
        "workspace file exceeds the reply attachment size limit",
    )
}

fn invalid_input(field: &'static str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "reply attachment input failed validation",
        vec![
            DispatchInputIssue::new(field, DispatchInputIssueCode::InvalidValue)
                .expected("safe workspace attachment metadata"),
        ],
    )
}

struct UnavailableReplyAttachmentIntentPort;

#[async_trait]
impl ReplyAttachmentIntentPort for UnavailableReplyAttachmentIntentPort {
    async fn register(
        &self,
        _scope: &ironclaw_host_api::ResourceScope,
        _run_id: &ironclaw_host_api::RunId,
        _intent: ReplyAttachmentIntent,
    ) -> Result<(), OutboundError> {
        Err(OutboundError::Backend)
    }

    async fn seal(
        &self,
        _scope: &ironclaw_host_api::ResourceScope,
        _run_id: &ironclaw_host_api::RunId,
    ) -> Result<Vec<ReplyAttachmentIntent>, OutboundError> {
        Err(OutboundError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{
        CapabilityId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, RunId,
        RuntimeDispatchErrorKind, VirtualPath,
    };
    use ironclaw_outbound::{OutboundStateStore, ReplyAttachmentIntentPort};
    use serde_json::{Value, json};

    use super::{
        ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID, AttachWorkspaceFileHandler,
        DEFAULT_ATTACHMENT_BUDGETS,
    };
    use crate::{
        FirstPartyCapabilityHandler, FirstPartyCapabilityRequest, HostProcessPort,
        InvocationServices,
    };

    const WORKSPACE_TARGET: &str = "/projects/reply-attachment-tests";
    struct Harness {
        root: Arc<InMemoryBackend>,
        store: Arc<OutboundStateStore<InMemoryBackend>>,
        handler: AttachWorkspaceFileHandler,
    }

    fn harness() -> Harness {
        let root = Arc::new(InMemoryBackend::new());
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let intent_port: Arc<dyn ReplyAttachmentIntentPort> = store.clone();
        Harness {
            root,
            store,
            handler: AttachWorkspaceFileHandler { intent_port },
        }
    }

    fn workspace_mount() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new(WORKSPACE_TARGET).expect("workspace target"),
            MountPermissions::read_only(),
        )])
        .expect("workspace mount")
    }

    fn request(
        root: Arc<InMemoryBackend>,
        run_id: Option<RunId>,
        mounts: Option<MountView>,
        input: Value,
    ) -> FirstPartyCapabilityRequest {
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID)
                .expect("attachment capability id"),
            ResourceScope::system(),
            input,
            None,
        );
        request.run_id = run_id;
        request.mounts = mounts;
        request.services = InvocationServices {
            filesystem: root,
            runtime_http_egress: None,
            tool_call_http_egress: None,
            runtime_secret_material_stager: None,
            process: Arc::new(HostProcessPort::new()),
            secret_store: None,
            audit_sink: None,
            unsafe_raw_diagnostics_allowed: false,
            post_edit_check: None,
        };
        request
    }

    async fn seed_file(root: &InMemoryBackend, name: &str, bytes: Vec<u8>) {
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_TARGET}/{name}")).expect("file target"),
            Entry::bytes(bytes),
            CasExpectation::Absent,
        )
        .await
        .expect("seed workspace file");
    }

    #[tokio::test]
    async fn reply_attachment_rejects_missing_run_id() {
        let harness = harness();
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                None,
                Some(workspace_mount()),
                json!({"path": "/workspace/report.txt"}),
            ))
            .await
            .expect_err("missing run must fail");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::OperationFailed)
        );
    }

    #[tokio::test]
    async fn reply_attachment_rejects_missing_mount_view() {
        let harness = harness();
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                None,
                json!({"path": "/workspace/report.txt"}),
            ))
            .await
            .expect_err("missing mount must fail");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::FilesystemDenied)
        );
    }

    #[tokio::test]
    async fn reply_attachment_rejects_path_outside_workspace() {
        let harness = harness();
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({"path": "/artifacts/report.txt"}),
            ))
            .await
            .expect_err("outside-workspace path must fail");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[tokio::test]
    async fn reply_attachment_rejects_directory_path() {
        let harness = harness();
        seed_file(
            harness.root.as_ref(),
            "directory/child.txt",
            b"child".to_vec(),
        )
        .await;
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({"path": "/workspace/directory"}),
            ))
            .await
            .expect_err("directory attachment must fail");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[tokio::test]
    async fn reply_attachment_rejects_missing_file() {
        let harness = harness();
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({"path": "/workspace/missing.txt"}),
            ))
            .await
            .expect_err("missing attachment must fail");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[tokio::test]
    async fn reply_attachment_rejects_oversized_file() {
        let harness = harness();
        seed_file(
            harness.root.as_ref(),
            "oversized.bin",
            vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1],
        )
        .await;
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({"path": "/workspace/oversized.bin"}),
            ))
            .await
            .expect_err("oversized attachment must fail");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::OperationFailed)
        );
    }

    #[tokio::test]
    async fn reply_attachment_rejects_unsafe_filename() {
        let harness = harness();
        seed_file(harness.root.as_ref(), "report.txt", b"report".to_vec()).await;
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({
                    "path": "/workspace/report.txt",
                    "filename": "../report.txt"
                }),
            ))
            .await
            .expect_err("unsafe filename must fail");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[tokio::test]
    async fn reply_attachment_rejects_invalid_mime_type() {
        let harness = harness();
        seed_file(harness.root.as_ref(), "report.txt", b"report".to_vec()).await;
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(RunId::new()),
                Some(workspace_mount()),
                json!({
                    "path": "/workspace/report.txt",
                    "mime_type": "text/plain; charset=utf-8"
                }),
            ))
            .await
            .expect_err("invalid MIME type must fail");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
    }

    #[tokio::test]
    async fn reply_attachment_registers_defaults_and_duplicate_is_idempotent() {
        let harness = harness();
        seed_file(harness.root.as_ref(), "report.csv", b"a,b\n1,2\n".to_vec()).await;
        let run_id = RunId::new();
        let scope = ResourceScope::system();
        for _ in 0..2 {
            let result = harness
                .handler
                .dispatch(request(
                    Arc::clone(&harness.root),
                    Some(run_id),
                    Some(workspace_mount()),
                    json!({"path": "/workspace/report.csv"}),
                ))
                .await
                .expect("register reply attachment");
            assert_eq!(result.output["attached"], true);
            assert_eq!(result.output["mime_type"], "text/csv");
        }

        let intents = harness
            .store
            .seal(&scope, &run_id)
            .await
            .expect("seal registered intents");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].filename, "report.csv");
        assert_eq!(intents[0].size_bytes, 8);
    }

    #[tokio::test]
    async fn reply_attachment_rejects_registration_after_seal() {
        let harness = harness();
        seed_file(harness.root.as_ref(), "late.txt", b"late".to_vec()).await;
        let run_id = RunId::new();
        harness
            .store
            .seal(&ResourceScope::system(), &run_id)
            .await
            .expect("seal run before registration");
        let error = harness
            .handler
            .dispatch(request(
                harness.root,
                Some(run_id),
                Some(workspace_mount()),
                json!({"path": "/workspace/late.txt"}),
            ))
            .await
            .expect_err("post-seal registration must fail");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::OperationFailed)
        );
    }
}
