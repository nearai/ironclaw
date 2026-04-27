//! Materialize orchestrator-served bootstrap artifacts into the worker filesystem.

use std::fs;
use std::path::Path;

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::WorkerError;
use crate::worker::api::{BootstrapArtifactDescriptor, BootstrapManifest, WorkerHttpClient};

const BOOTSTRAP_ENV: &str = "IRONCLAW_USE_BOOTSTRAP_ARTIFACTS";

pub fn bootstrap_artifacts_enabled() -> bool {
    matches!(
        std::env::var(BOOTSTRAP_ENV).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

pub fn bootstrap_start_message(
    manifest: Option<&BootstrapManifest>,
    default_message: &str,
) -> String {
    match manifest {
        Some(manifest)
            if manifest
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == "workspace_snapshot") =>
        {
            format!("Loaded project snapshot from orchestrator; {default_message}")
        }
        Some(manifest) if !manifest.artifacts.is_empty() => {
            format!("Loaded bootstrap artifacts from orchestrator; {default_message}")
        }
        _ => default_message.to_string(),
    }
}

pub async fn materialize_job_bootstrap(
    client: &WorkerHttpClient,
) -> Result<Option<BootstrapManifest>, WorkerError> {
    if !bootstrap_artifacts_enabled() {
        return Ok(None);
    }

    let manifest = client.fetch_bootstrap_manifest().await?;

    for artifact in &manifest.artifacts {
        let body = client.fetch_bootstrap_artifact(&artifact.id).await?;
        materialize_artifact(artifact, &body)?;
    }

    Ok(Some(manifest))
}

fn materialize_artifact(
    artifact: &BootstrapArtifactDescriptor,
    body: &[u8],
) -> Result<(), WorkerError> {
    match artifact.kind.as_str() {
        "workspace_snapshot" => {
            let target = artifact.target_path.as_deref().unwrap_or("/workspace");
            unpack_workspace_snapshot(body, Path::new(target))
        }
        "runtime_config" => {
            let target =
                artifact
                    .target_path
                    .as_deref()
                    .ok_or_else(|| WorkerError::ExecutionFailed {
                        reason: format!(
                            "bootstrap artifact {} is missing target_path",
                            artifact.id
                        ),
                    })?;
            write_file_artifact(body, Path::new(target))
        }
        _ => {
            tracing::debug!(
                artifact_id = %artifact.id,
                kind = %artifact.kind,
                "Skipping unsupported bootstrap artifact kind"
            );
            Ok(())
        }
    }
}

fn unpack_workspace_snapshot(body: &[u8], target_dir: &Path) -> Result<(), WorkerError> {
    fs::create_dir_all(target_dir).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!(
            "failed to create bootstrap workspace {}: {e}",
            target_dir.display()
        ),
    })?;
    clear_directory_contents(target_dir)?;

    let decoder = GzDecoder::new(body);
    let mut archive = Archive::new(decoder);
    archive.set_preserve_permissions(false);
    archive.set_unpack_xattrs(false);
    archive
        .unpack(target_dir)
        .map_err(|e| WorkerError::ExecutionFailed {
            reason: format!(
                "failed to unpack workspace bootstrap into {}: {e}",
                target_dir.display()
            ),
        })
}

fn write_file_artifact(body: &[u8], target_path: &Path) -> Result<(), WorkerError> {
    let parent = target_path
        .parent()
        .ok_or_else(|| WorkerError::ExecutionFailed {
            reason: format!(
                "bootstrap target {} has no parent directory",
                target_path.display()
            ),
        })?;
    fs::create_dir_all(parent).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!(
            "failed to create bootstrap config dir {}: {e}",
            parent.display()
        ),
    })?;
    fs::write(target_path, body).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!(
            "failed to write bootstrap artifact {}: {e}",
            target_path.display()
        ),
    })
}

fn clear_directory_contents(dir: &Path) -> Result<(), WorkerError> {
    for entry in fs::read_dir(dir).map_err(|e| WorkerError::ExecutionFailed {
        reason: format!("failed to read bootstrap workspace {}: {e}", dir.display()),
    })? {
        let entry = entry.map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to inspect bootstrap workspace entry: {e}"),
        })?;
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed to clear directory {}: {e}", path.display()),
            })?;
        } else {
            fs::remove_file(&path).map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed to clear file {}: {e}", path.display()),
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;
    use uuid::Uuid;

    use super::*;

    fn make_tarball() -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        let content = b"hello";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "file.txt", &content[..])
            .expect("tarball should be created");
        let encoder = builder.into_inner().expect("tar builder should finish");
        encoder.finish().expect("gzip stream should finish")
    }

    #[test]
    fn clear_directory_contents_removes_children_only() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        fs::write(temp.path().join("file.txt"), "hi").expect("fixture should write");
        fs::create_dir_all(temp.path().join("nested")).expect("nested dir should exist");

        clear_directory_contents(temp.path()).expect("directory should clear");

        assert!(temp.path().exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn workspace_snapshot_unpacks_into_target() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        unpack_workspace_snapshot(&make_tarball(), temp.path()).expect("snapshot should unpack");

        let content = fs::read_to_string(temp.path().join("file.txt")).expect("file should exist");
        assert_eq!(content, "hello");
    }

    #[test]
    fn runtime_config_artifact_writes_file() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let target = temp.path().join("config").join("mcp.json");

        write_file_artifact(br#"{"ok":true}"#, &target).expect("config should write");

        let content = fs::read_to_string(target).expect("config should exist");
        assert_eq!(content, "{\"ok\":true}");
    }

    #[test]
    fn bootstrap_start_message_mentions_project_snapshot_when_present() {
        let manifest = BootstrapManifest {
            job_id: Uuid::new_v4(),
            provenance: crate::worker::api::BootstrapProvenance {
                generated_at: "2026-04-14T00:00:00Z".to_string(),
                snapshot_source: "project-dir".to_string(),
                project_dir: Some("/tmp/project".to_string()),
            },
            artifacts: vec![BootstrapArtifactDescriptor {
                id: "workspace-snapshot".to_string(),
                kind: "workspace_snapshot".to_string(),
                media_type: "application/gzip".to_string(),
                file_name: "workspace.tar.gz".to_string(),
                target_path: Some("/workspace".to_string()),
            }],
        };

        assert_eq!(
            bootstrap_start_message(Some(&manifest), "Worker started"),
            "Loaded project snapshot from orchestrator; Worker started"
        );
    }
}
