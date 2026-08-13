//! Bounded single-file copy between the scoped IronClaw workspace and a sandbox workspace.

use std::sync::Arc;

use ironclaw_filesystem::{CasExpectation, Entry, FileType, FilesystemError, ScopedFilesystem};
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind,
    mount::MountView,
    path::ScopedPath,
    process::{
        SandboxWorkspaceFileError, SandboxWorkspaceFileReadRequest, SandboxWorkspaceFileTransport,
        SandboxWorkspaceFileWriteRequest,
    },
    resource::ResourceScope,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub const MAX_SANDBOX_WORKSPACE_COPY_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct SandboxWorkspaceCopyRequest<'a> {
    pub scope: &'a ResourceScope,
    pub mounts: Option<&'a MountView>,
    pub filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem>,
    pub transport: Arc<dyn SandboxWorkspaceFileTransport>,
    pub input: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxWorkspaceCopyOutput {
    pub direction: &'static str,
    pub source_path: String,
    pub destination_path: String,
    pub bytes_copied: usize,
    pub sha256: String,
    pub already_present: bool,
    pub overwrite_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("sandbox workspace copy failed: {kind}")]
pub struct SandboxWorkspaceCopyError {
    kind: RuntimeDispatchErrorKind,
    safe_summary: Option<&'static str>,
}

impl SandboxWorkspaceCopyError {
    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn safe_summary(&self) -> Option<&'static str> {
        self.safe_summary
    }

    fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self {
            kind,
            safe_summary: None,
        }
    }

    fn with_safe_summary(kind: RuntimeDispatchErrorKind, safe_summary: &'static str) -> Self {
        Self {
            kind,
            safe_summary: Some(safe_summary),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CopyDirection {
    IronclawToSandbox,
    SandboxToIronclaw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyInput {
    direction: CopyDirection,
    source_path: String,
    destination_path: String,
    #[serde(default)]
    overwrite: bool,
}

pub async fn execute(
    request: &SandboxWorkspaceCopyRequest<'_>,
) -> Result<SandboxWorkspaceCopyOutput, SandboxWorkspaceCopyError> {
    let input: CopyInput = serde_json::from_value(request.input.clone())
        .map_err(|_| invalid_input("workspace copy input is invalid"))?;
    let source_path = workspace_path(&input.source_path)?;
    let destination_path = workspace_path(&input.destination_path)?;
    let mounts = request.mounts.cloned().ok_or_else(|| {
        SandboxWorkspaceCopyError::new(RuntimeDispatchErrorKind::FilesystemDenied)
    })?;
    let filesystem = ScopedFilesystem::with_fixed_view(Arc::clone(&request.filesystem), mounts);

    let (bytes_copied, sha256, already_present) = match input.direction {
        CopyDirection::IronclawToSandbox => {
            let stat = filesystem
                .stat(request.scope, &source_path)
                .await
                .map_err(map_filesystem_error)?;
            if stat.file_type != FileType::File {
                return Err(invalid_input("source_path must name a regular file"));
            }
            if stat.len > MAX_SANDBOX_WORKSPACE_COPY_BYTES as u64 {
                return Err(copy_too_large());
            }
            let bytes = filesystem
                .read_bytes_bounded(
                    request.scope,
                    &source_path,
                    MAX_SANDBOX_WORKSPACE_COPY_BYTES,
                )
                .await
                .map_err(map_filesystem_error)?
                .ok_or_else(copy_too_large)?;
            if bytes.len() as u64 != stat.len {
                return Err(SandboxWorkspaceCopyError::with_safe_summary(
                    RuntimeDispatchErrorKind::OperationFailed,
                    "IronClaw workspace source changed while it was being copied",
                ));
            }
            let output = request
                .transport
                .write_file(SandboxWorkspaceFileWriteRequest {
                    scope: request.scope.clone(),
                    path: destination_path.as_str().to_string(),
                    bytes,
                    overwrite: input.overwrite,
                })
                .await
                .map_err(map_transfer_error)?;
            (output.bytes_written, output.sha256, output.already_present)
        }
        CopyDirection::SandboxToIronclaw => {
            let output = request
                .transport
                .read_file(SandboxWorkspaceFileReadRequest {
                    scope: request.scope.clone(),
                    path: source_path.as_str().to_string(),
                    max_bytes: MAX_SANDBOX_WORKSPACE_COPY_BYTES,
                })
                .await
                .map_err(map_transfer_error)?;
            let transport_digest = hex::encode(Sha256::digest(&output.bytes));
            if transport_digest != output.sha256 {
                return Err(SandboxWorkspaceCopyError::with_safe_summary(
                    RuntimeDispatchErrorKind::OperationFailed,
                    "sandbox workspace source verification failed",
                ));
            }
            let expectation = if input.overwrite {
                CasExpectation::Any
            } else {
                CasExpectation::Absent
            };
            let host_already_present = match filesystem
                .put(
                    request.scope,
                    &destination_path,
                    Entry::bytes(output.bytes.clone()),
                    expectation,
                )
                .await
            {
                Ok(_) => false,
                Err(FilesystemError::VersionMismatch { .. }) if !input.overwrite => {
                    let existing = filesystem
                        .read_bytes_bounded(
                            request.scope,
                            &destination_path,
                            MAX_SANDBOX_WORKSPACE_COPY_BYTES,
                        )
                        .await
                        .map_err(map_filesystem_error)?
                        .ok_or_else(copy_too_large)?;
                    if existing != output.bytes {
                        return Err(destination_conflict());
                    }
                    true
                }
                Err(error) => return Err(map_filesystem_error(error)),
            };
            let read_back = filesystem
                .read_bytes_bounded(
                    request.scope,
                    &destination_path,
                    MAX_SANDBOX_WORKSPACE_COPY_BYTES,
                )
                .await
                .map_err(map_filesystem_error)?
                .ok_or_else(copy_too_large)?;
            let digest = hex::encode(Sha256::digest(&read_back));
            if read_back != output.bytes || digest != output.sha256 {
                return Err(SandboxWorkspaceCopyError::with_safe_summary(
                    RuntimeDispatchErrorKind::OperationFailed,
                    "IronClaw workspace destination read-back verification failed",
                ));
            }
            (read_back.len(), digest, host_already_present)
        }
    };

    Ok(SandboxWorkspaceCopyOutput {
        direction: match input.direction {
            CopyDirection::IronclawToSandbox => "ironclaw_to_sandbox",
            CopyDirection::SandboxToIronclaw => "sandbox_to_ironclaw",
        },
        source_path: input.source_path,
        destination_path: input.destination_path,
        bytes_copied,
        sha256,
        already_present,
        overwrite_enabled: input.overwrite,
    })
}

fn workspace_path(path: &str) -> Result<ScopedPath, SandboxWorkspaceCopyError> {
    let path = ScopedPath::new(path.to_string())
        .map_err(|_| invalid_input("workspace paths must be valid scoped paths"))?;
    if path
        .as_str()
        .strip_prefix("/workspace/")
        .is_none_or(str::is_empty)
    {
        return Err(invalid_input(
            "workspace paths must be descendants of /workspace",
        ));
    }
    Ok(path)
}

fn invalid_input(summary: &'static str) -> SandboxWorkspaceCopyError {
    SandboxWorkspaceCopyError::with_safe_summary(RuntimeDispatchErrorKind::InputEncode, summary)
}

fn copy_too_large() -> SandboxWorkspaceCopyError {
    SandboxWorkspaceCopyError::with_safe_summary(
        RuntimeDispatchErrorKind::Resource,
        "workspace copy is limited to 10485760 bytes",
    )
}

fn destination_conflict() -> SandboxWorkspaceCopyError {
    SandboxWorkspaceCopyError::with_safe_summary(
        RuntimeDispatchErrorKind::OperationFailed,
        "workspace copy destination already exists; set overwrite to true to replace it",
    )
}

fn map_transfer_error(error: SandboxWorkspaceFileError) -> SandboxWorkspaceCopyError {
    tracing::debug!(?error, "sandbox workspace file transfer failed");
    SandboxWorkspaceCopyError::with_safe_summary(
        RuntimeDispatchErrorKind::OperationFailed,
        match error {
            SandboxWorkspaceFileError::NotFound => "sandbox workspace source file was not found",
            SandboxWorkspaceFileError::InvalidPath => "sandbox workspace path is invalid",
            SandboxWorkspaceFileError::InvalidLimit => {
                "sandbox workspace file byte limit is invalid"
            }
            SandboxWorkspaceFileError::NotRegularFile => {
                "sandbox workspace path must name a regular file"
            }
            SandboxWorkspaceFileError::TooLarge { .. } => {
                "workspace copy is limited to 10485760 bytes"
            }
            SandboxWorkspaceFileError::Conflict => "sandbox workspace destination already exists",
            SandboxWorkspaceFileError::TransportFailed => "sandbox workspace file transfer failed",
            SandboxWorkspaceFileError::InvalidResponse => {
                "sandbox workspace file verification failed"
            }
            SandboxWorkspaceFileError::CheckpointFailed => {
                "sandbox workspace mutation could not be checkpointed"
            }
        },
    )
}

fn map_filesystem_error(error: FilesystemError) -> SandboxWorkspaceCopyError {
    tracing::debug!(error = %error, "sandbox workspace copy filesystem operation failed");
    match error {
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::Contract(_)
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. } => {
            SandboxWorkspaceCopyError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => {
            invalid_input("workspace copy source file was not found")
        }
        FilesystemError::VersionMismatch { .. } => destination_conflict(),
        _ => SandboxWorkspaceCopyError::new(RuntimeDispatchErrorKind::Backend),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        process::{SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileWriteOutput},
        resource::ResourceScope,
    };
    use serde_json::json;
    use std::sync::Mutex;

    const WORKSPACE_ROOT: &str = "/projects/sandbox-copy-tests";

    #[derive(Default)]
    struct RecordingTransfer {
        files: Mutex<std::collections::HashMap<String, Vec<u8>>>,
        writes: Mutex<Vec<SandboxWorkspaceFileWriteRequest>>,
        corrupt_read_digest: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl SandboxWorkspaceFileTransport for RecordingTransfer {
        async fn read_file(
            &self,
            request: SandboxWorkspaceFileReadRequest,
        ) -> Result<SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileError> {
            let bytes = self
                .files
                .lock()
                .expect("files lock")
                .get(&request.path)
                .cloned()
                .ok_or(SandboxWorkspaceFileError::NotFound)?;
            Ok(SandboxWorkspaceFileReadOutput {
                sha256: if self
                    .corrupt_read_digest
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    "0".repeat(64)
                } else {
                    hex::encode(Sha256::digest(&bytes))
                },
                bytes,
            })
        }

        async fn write_file(
            &self,
            request: SandboxWorkspaceFileWriteRequest,
        ) -> Result<SandboxWorkspaceFileWriteOutput, SandboxWorkspaceFileError> {
            let digest = hex::encode(Sha256::digest(&request.bytes));
            self.files
                .lock()
                .expect("files lock")
                .insert(request.path.clone(), request.bytes.clone());
            self.writes
                .lock()
                .expect("writes lock")
                .push(request.clone());
            Ok(SandboxWorkspaceFileWriteOutput {
                bytes_written: request.bytes.len(),
                sha256: digest,
                already_present: false,
            })
        }
    }

    fn mount_view() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("alias"),
            VirtualPath::new(WORKSPACE_ROOT).expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view")
    }

    async fn copy(
        root: Arc<InMemoryBackend>,
        transfer: Arc<RecordingTransfer>,
        input: serde_json::Value,
    ) -> Result<SandboxWorkspaceCopyOutput, SandboxWorkspaceCopyError> {
        let scope = ResourceScope::system();
        let mounts = mount_view();
        execute(&SandboxWorkspaceCopyRequest {
            scope: &scope,
            mounts: Some(&mounts),
            filesystem: root,
            transport: transfer,
            input: &input,
        })
        .await
    }

    async fn seed(root: &InMemoryBackend, name: &str, bytes: &[u8]) {
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_ROOT}/{name}")).expect("seed path"),
            Entry::bytes(bytes.to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed file");
    }

    #[tokio::test]
    async fn copies_binary_bytes_from_scoped_ironclaw_workspace_to_sandbox() {
        let root = Arc::new(InMemoryBackend::new());
        seed(&root, "input.pdf", b"pdf\0bytes").await;
        let transfer = Arc::new(RecordingTransfer::default());
        let result = copy(
            root,
            transfer.clone(),
            json!({
                "direction": "ironclaw_to_sandbox",
                "source_path": "/workspace/input.pdf",
                "destination_path": "/workspace/input.pdf"
            }),
        )
        .await
        .expect("copy succeeds");

        assert_eq!(result.bytes_copied, 9);
        let writes = transfer.writes.lock().expect("writes lock");
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].bytes, b"pdf\0bytes");
        assert!(!writes[0].overwrite);
    }

    #[tokio::test]
    async fn sandbox_to_ironclaw_defaults_to_atomic_no_clobber() {
        let root = Arc::new(InMemoryBackend::new());
        seed(&root, "result.bin", b"existing").await;
        let transfer = Arc::new(RecordingTransfer::default());
        transfer
            .files
            .lock()
            .expect("files lock")
            .insert("/workspace/result.bin".to_string(), b"replacement".to_vec());
        let error = copy(
            root.clone(),
            transfer,
            json!({
                "direction": "sandbox_to_ironclaw",
                "source_path": "/workspace/result.bin",
                "destination_path": "/workspace/result.bin"
            }),
        )
        .await
        .expect_err("existing different destination conflicts");

        assert!(
            error
                .safe_summary()
                .is_some_and(|summary| summary.contains("already exists"))
        );
        assert_eq!(
            root.read_file(
                &VirtualPath::new(format!("{WORKSPACE_ROOT}/result.bin")).expect("result path")
            )
            .await
            .expect("existing file remains"),
            b"existing"
        );
    }

    #[tokio::test]
    async fn sandbox_to_ironclaw_overwrites_when_explicitly_enabled() {
        let root = Arc::new(InMemoryBackend::new());
        seed(&root, "result.bin", b"existing").await;
        let transfer = Arc::new(RecordingTransfer::default());
        transfer
            .files
            .lock()
            .expect("files lock")
            .insert("/workspace/result.bin".to_string(), b"replacement".to_vec());
        let result = copy(
            root.clone(),
            transfer,
            json!({
                "direction": "sandbox_to_ironclaw",
                "source_path": "/workspace/result.bin",
                "destination_path": "/workspace/result.bin",
                "overwrite": true
            }),
        )
        .await
        .expect("explicit overwrite succeeds");

        assert!(result.overwrite_enabled);
        assert_eq!(
            root.read_file(
                &VirtualPath::new(format!("{WORKSPACE_ROOT}/result.bin")).expect("result path")
            )
            .await
            .expect("replacement file"),
            b"replacement"
        );
    }

    #[tokio::test]
    async fn copies_binary_bytes_from_sandbox_with_verified_digest() {
        let root = Arc::new(InMemoryBackend::new());
        let bytes = b"sandbox\0result".to_vec();
        let transfer = Arc::new(RecordingTransfer::default());
        transfer
            .files
            .lock()
            .expect("files lock")
            .insert("/workspace/result.bin".to_string(), bytes.clone());
        let result = copy(
            root.clone(),
            transfer,
            json!({
                "direction": "sandbox_to_ironclaw",
                "source_path": "/workspace/result.bin",
                "destination_path": "/workspace/result.bin"
            }),
        )
        .await
        .expect("copy succeeds");

        assert_eq!(result.bytes_copied, bytes.len());
        assert_eq!(result.sha256, hex::encode(Sha256::digest(&bytes)));
        assert!(!result.overwrite_enabled);
        assert_eq!(
            root.read_file(
                &VirtualPath::new(format!("{WORKSPACE_ROOT}/result.bin")).expect("result path")
            )
            .await
            .expect("copied file"),
            bytes
        );
    }

    #[tokio::test]
    async fn rejects_a_sandbox_read_with_a_corrupt_digest() {
        let root = Arc::new(InMemoryBackend::new());
        let transfer = Arc::new(RecordingTransfer::default());
        transfer
            .files
            .lock()
            .expect("files lock")
            .insert("/workspace/result.bin".to_string(), b"result".to_vec());
        transfer
            .corrupt_read_digest
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let error = copy(
            root,
            transfer,
            json!({
                "direction": "sandbox_to_ironclaw",
                "source_path": "/workspace/result.bin",
                "destination_path": "/workspace/result.bin"
            }),
        )
        .await
        .expect_err("digest mismatch fails closed");

        assert!(
            error
                .safe_summary()
                .is_some_and(|summary| summary.contains("verification"))
        );
    }

    #[tokio::test]
    async fn rejects_a_scoped_directory_as_the_ironclaw_source() {
        let root = Arc::new(InMemoryBackend::new());
        seed(&root, "folder/child.txt", b"child").await;
        let error = copy(
            root,
            Arc::new(RecordingTransfer::default()),
            json!({
                "direction": "ironclaw_to_sandbox",
                "source_path": "/workspace/folder",
                "destination_path": "/workspace/folder"
            }),
        )
        .await
        .expect_err("directories are not copied");

        assert!(
            error
                .safe_summary()
                .is_some_and(|summary| summary.contains("regular file"))
        );
    }
}
