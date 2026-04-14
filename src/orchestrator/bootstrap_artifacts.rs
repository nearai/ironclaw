//! Bootstrap artifact contracts for worker startup.
//!
//! These artifacts let the orchestrator describe and serve per-job workspace
//! and config inputs over the authenticated worker control plane instead of
//! relying on host mounts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use flate2::Compression;
use flate2::write::GzEncoder;
use tar::Builder;
use uuid::Uuid;

use crate::error::OrchestratorError;
use crate::worker::api::{BootstrapArtifactDescriptor, BootstrapManifest, BootstrapProvenance};

const WORKSPACE_ARTIFACT_ID: &str = "workspace-snapshot";
const MCP_CONFIG_ARTIFACT_ID: &str = "mcp-config";
const MCP_TARGET_PATH: &str = "/home/sandbox/.ironclaw/mcp-servers.json";

#[derive(Debug, Clone)]
pub struct ResolvedBootstrapArtifact {
    pub body: Vec<u8>,
    pub media_type: String,
    pub file_name: String,
}

#[derive(Debug, Clone)]
pub struct JobBootstrapArtifacts {
    manifest: BootstrapManifest,
    sources: HashMap<String, BootstrapArtifactSource>,
}

#[derive(Debug, Clone)]
enum BootstrapArtifactSource {
    WorkspaceSnapshot {
        project_dir: PathBuf,
    },
    Inline {
        body: Vec<u8>,
        media_type: String,
        file_name: String,
    },
}

impl JobBootstrapArtifacts {
    pub fn metadata_only(job_id: Uuid, project_dir: Option<&Path>) -> Self {
        let provenance = BootstrapProvenance {
            generated_at: Utc::now().to_rfc3339(),
            snapshot_source: if project_dir.is_some() {
                "project-dir".to_string()
            } else {
                "none".to_string()
            },
            project_dir: project_dir.map(|dir| dir.display().to_string()),
        };

        Self {
            manifest: BootstrapManifest {
                job_id,
                provenance,
                artifacts: Vec::new(),
            },
            sources: HashMap::new(),
        }
    }

    pub fn new(job_id: Uuid, project_dir: Option<&Path>, mcp_config_json: Option<String>) -> Self {
        let mut bootstrap = Self::metadata_only(job_id, project_dir);

        if project_dir.is_some() {
            bootstrap
                .manifest
                .artifacts
                .push(BootstrapArtifactDescriptor {
                    id: WORKSPACE_ARTIFACT_ID.to_string(),
                    kind: "workspace_snapshot".to_string(),
                    media_type: "application/gzip".to_string(),
                    file_name: "workspace.tar.gz".to_string(),
                    target_path: Some("/workspace".to_string()),
                });
        }

        if let Some(project_dir) = project_dir {
            bootstrap.sources.insert(
                WORKSPACE_ARTIFACT_ID.to_string(),
                BootstrapArtifactSource::WorkspaceSnapshot {
                    project_dir: project_dir.to_path_buf(),
                },
            );
        }

        if let Some(config_json) = mcp_config_json {
            let body = config_json.into_bytes();
            bootstrap
                .manifest
                .artifacts
                .push(BootstrapArtifactDescriptor {
                    id: MCP_CONFIG_ARTIFACT_ID.to_string(),
                    kind: "runtime_config".to_string(),
                    media_type: "application/json".to_string(),
                    file_name: "mcp-servers.json".to_string(),
                    target_path: Some(MCP_TARGET_PATH.to_string()),
                });
            bootstrap.sources.insert(
                MCP_CONFIG_ARTIFACT_ID.to_string(),
                BootstrapArtifactSource::Inline {
                    body,
                    media_type: "application/json".to_string(),
                    file_name: "mcp-servers.json".to_string(),
                },
            );
        }

        bootstrap
    }

    pub fn manifest(&self) -> BootstrapManifest {
        self.manifest.clone()
    }

    pub fn resolve_artifact(
        &self,
        job_id: Uuid,
        artifact_id: &str,
    ) -> Result<Option<ResolvedBootstrapArtifact>, OrchestratorError> {
        let Some(source) = self.sources.get(artifact_id) else {
            return Ok(None);
        };

        match source {
            BootstrapArtifactSource::WorkspaceSnapshot { project_dir } => {
                let body = build_workspace_snapshot(project_dir, job_id)?;
                Ok(Some(ResolvedBootstrapArtifact {
                    body,
                    media_type: "application/gzip".to_string(),
                    file_name: "workspace.tar.gz".to_string(),
                }))
            }
            BootstrapArtifactSource::Inline {
                body,
                media_type,
                file_name,
            } => Ok(Some(ResolvedBootstrapArtifact {
                body: body.clone(),
                media_type: media_type.clone(),
                file_name: file_name.clone(),
            })),
        }
    }
}

fn build_workspace_snapshot(
    project_dir: &Path,
    job_id: Uuid,
) -> Result<Vec<u8>, OrchestratorError> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);

    builder.append_dir_all(".", project_dir).map_err(|e| {
        OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "failed to package workspace snapshot from {}: {}",
                project_dir.display(),
                e
            ),
        }
    })?;

    let encoder = builder
        .into_inner()
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to finalize workspace snapshot tar stream: {e}"),
        })?;

    encoder
        .finish()
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to compress workspace snapshot: {e}"),
        })
}

/// Render a per-job MCP config JSON document from caller-provided master data.
pub fn render_worker_mcp_config_json(
    master: Option<&serde_json::Value>,
    server_names: Option<&[String]>,
    job_id: Uuid,
) -> Result<Option<String>, OrchestratorError> {
    let Some(master) = master else {
        return Ok(None);
    };

    if matches!(server_names, Some([])) {
        return Ok(None);
    }

    if let Some(names) = server_names {
        for name in names {
            if name.len() > 128 || name.contains('/') || name.contains('\\') || name.contains('\0')
            {
                return Err(OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("invalid MCP server name: {:?}", name),
                });
            }
        }
    }

    let servers_value =
        master
            .get("servers")
            .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "MCP master config is missing the required `servers` field".to_string(),
            })?;
    let servers_array =
        servers_value
            .as_array()
            .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "MCP master config `servers` field must be an array, got {}",
                    type_name_of(servers_value)
                ),
            })?;

    let servers_iter = servers_array.clone().into_iter();
    let filtered_servers: Vec<serde_json::Value> = match server_names {
        None => servers_iter
            .filter(|server| server["enabled"].as_bool().unwrap_or(true))
            .collect(),
        Some(names) => servers_iter
            .filter(|server| {
                let name_matches = server["name"]
                    .as_str()
                    .map(|name| {
                        names
                            .iter()
                            .any(|requested| requested.eq_ignore_ascii_case(name))
                    })
                    .unwrap_or(false);
                let is_enabled = server["enabled"].as_bool().unwrap_or(true);
                name_matches && is_enabled
            })
            .collect(),
    };

    if filtered_servers.is_empty() {
        if let Some(names) = server_names {
            tracing::warn!(
                job_id = %job_id,
                requested = ?names,
                "No matching MCP servers found in master config; skipping bootstrap MCP artifact"
            );
        } else {
            tracing::debug!(
                job_id = %job_id,
                "Master MCP config has no enabled servers; skipping bootstrap MCP artifact"
            );
        }
        return Ok(None);
    }

    let schema_version = master
        .get("schema_version")
        .cloned()
        .unwrap_or(serde_json::json!(1));
    let filtered = serde_json::json!({
        "servers": filtered_servers,
        "schema_version": schema_version
    });

    serde_json::to_string_pretty(&filtered)
        .map(Some)
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to serialize filtered MCP config: {e}"),
        })
}

pub async fn write_worker_mcp_config_tempfile(
    config_json: &str,
    job_id: Uuid,
) -> Result<PathBuf, OrchestratorError> {
    let tmp_dir = std::env::temp_dir().join("ironclaw-mcp-configs");
    tokio::fs::create_dir_all(&tmp_dir).await.map_err(|e| {
        OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to create MCP config temp dir: {e}"),
        }
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp_dir, std::fs::Permissions::from_mode(0o700)).await;
    }

    let tmp_path = tmp_dir.join(format!("{}.json", job_id));
    tokio::fs::write(&tmp_path, config_json)
        .await
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to write per-job MCP config: {e}"),
        })?;

    Ok(tmp_path)
}

fn type_name_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;
    use tar::Archive;

    use super::*;

    #[test]
    fn metadata_only_manifest_has_no_artifacts() {
        let manifest = JobBootstrapArtifacts::metadata_only(Uuid::nil(), None).manifest();
        assert!(manifest.artifacts.is_empty());
        assert_eq!(manifest.provenance.snapshot_source, "none");
    }

    #[test]
    fn workspace_snapshot_resolves_as_tarball() {
        let temp = tempfile::tempdir().expect("temp dir should exist");
        std::fs::write(temp.path().join("hello.txt"), "hi").expect("fixture should write");

        let artifacts = JobBootstrapArtifacts::new(Uuid::nil(), Some(temp.path()), None);
        let resolved = artifacts
            .resolve_artifact(Uuid::nil(), WORKSPACE_ARTIFACT_ID)
            .expect("workspace artifact should resolve")
            .expect("workspace artifact should exist");

        let decoder = GzDecoder::new(resolved.body.as_slice());
        let mut archive = Archive::new(decoder);
        let mut names = archive
            .entries()
            .expect("archive entries should parse")
            .map(|entry| {
                let mut entry = entry.expect("entry should parse");
                let mut content = String::new();
                if entry.header().entry_type().is_file() {
                    let _ = entry.read_to_string(&mut content);
                }
                (
                    entry
                        .path()
                        .expect("entry path should parse")
                        .to_string_lossy()
                        .to_string(),
                    content,
                )
            })
            .collect::<Vec<_>>();
        names.sort_by(|a, b| a.0.cmp(&b.0));

        assert!(
            names.iter().any(|(name, _)| name.ends_with("hello.txt")),
            "expected hello.txt in archive, got: {:?}",
            names
        );
        assert!(names.iter().any(|(_, content)| content == "hi"));
    }

    #[test]
    fn rendered_mcp_config_can_be_added_to_manifest() {
        let master = serde_json::json!({
            "schema_version": 1,
            "servers": [
                {"name": "serpstat", "enabled": true, "url": "http://localhost:8062"},
                {"name": "disabled", "enabled": false, "url": "http://localhost:9999"}
            ]
        });

        let config = render_worker_mcp_config_json(
            Some(&master),
            Some(&["serpstat".to_string()]),
            Uuid::nil(),
        )
        .expect("render should succeed")
        .expect("config should exist");

        let manifest = JobBootstrapArtifacts::new(Uuid::nil(), None, Some(config)).manifest();
        assert_eq!(manifest.artifacts.len(), 1);
        assert_eq!(manifest.artifacts[0].id, MCP_CONFIG_ARTIFACT_ID);
    }
}
