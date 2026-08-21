//! Exact pinned coding surface for the always-on first-party package (issue #7392).
//!
//! Registers the pinned `read`, `write`, `edit`, `glob`, `grep`, and `bash`
//! tools while retaining the structural document capabilities.
//! These capabilities use the ordinary first-party dispatch path, so authorization, approvals,
//! resource accounting, mount scoping, and durable artifact handling remain
//! host-owned.

use std::sync::Arc;
use std::time::Instant;

use super::{
    GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, MAX_FIRST_PARTY_INPUT_BYTES,
    MAX_WRITE_FILE_INPUT_BYTES, builtin_first_party_package,
};
use crate::first_party::PendingFirstPartyArtifact;
use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use async_trait::async_trait;
use ironclaw_extension_registry::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_extension_support::coding::{
    CodingCapabilityError, CodingCapabilityKind, CodingCapabilityOutput, CodingCapabilityRequest,
    CodingCapabilityState,
    pinned::{
        CodingEngineContext, CodingEngineError, CodingEngineErrorKind, CodingSnapshotRegistry,
    },
};
use ironclaw_host_api::{
    artifact::{
        ARTIFACT_INLINE_PREVIEW_MAX_BYTES, ArtifactOwnerScope, ArtifactRef, ArtifactWriteError,
        ArtifactWriteMetadata,
    },
    capability::{EffectKind, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::{CapabilityId, ProviderToolName},
    path::VirtualPath,
    resource::{ResourceCeiling, ResourceEstimate, ResourceProfile, ResourceUsage},
    result_meta::OutputDigest,
    runtime_policy::ProcessBackendKind,
};
use ironclaw_loop_contracts::ContentDigest;

/// Canonical capability id of the pinned `read` engine.
pub const CODING_READ_CAPABILITY_ID: &str = "builtin.read";
/// Canonical capability id of the pinned `write` engine.
pub const CODING_WRITE_CAPABILITY_ID: &str = "builtin.write";
/// Canonical capability id of the pinned hashline `edit` engine.
pub const CODING_EDIT_CAPABILITY_ID: &str = "builtin.edit";
/// Canonical capability id of the pinned `glob` engine.
pub const CODING_GLOB_CAPABILITY_ID: &str = GLOB_CAPABILITY_ID;
/// Canonical capability id of the pinned `grep` engine.
pub const CODING_GREP_CAPABILITY_ID: &str = GREP_CAPABILITY_ID;
/// Canonical capability id of the pinned `bash` engine.
pub const CODING_BASH_CAPABILITY_ID: &str = "builtin.bash";
/// Canonical capability id of the structural document editor.
pub const DOCUMENT_EDIT_CAPABILITY_ID: &str = "builtin.document_edit";
/// Canonical capability id of the HTML-to-PDF renderer.
pub const HTML_TO_PDF_CAPABILITY_ID: &str = "builtin.html_to_pdf";

/// Inline preview budget for a coding result that spilled to an artifact.
///
/// Stays under [`ARTIFACT_INLINE_PREVIEW_MAX_BYTES`] because
/// `ironclaw_capabilities::dispatch` tail-cuts the whole canonical JSON above
/// that ceiling once a durable artifact exists — and that cut replaces the
/// result object with a bare truncated string, so a preview which reaches it
/// loses both its footer and its shape. The gap absorbs JSON escaping (every
/// newline doubles) plus the sibling `artifact_ref`/`total_bytes` fields.
const CODING_ARTIFACT_PREVIEW_MAX_BYTES: usize = 48 * 1024;
const MAX_DOCUMENT_EDIT_INPUT_BYTES: usize = 21 * 1024 * 1024;
/// A 10 MiB source read can grow when rendered with Hashline anchors. Keep
/// bounded headroom for that representation without allowing a grep over many
/// large files to create an unbounded artifact.
const CODING_MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

/// Exact model-visible provider names (the pinned tool names).
const CODING_READ_PROVIDER_TOOL_NAME: &str = "read";
const CODING_WRITE_PROVIDER_TOOL_NAME: &str = "write";
const CODING_EDIT_PROVIDER_TOOL_NAME: &str = "edit";
const CODING_GLOB_PROVIDER_TOOL_NAME: &str = "glob";
const CODING_GREP_PROVIDER_TOOL_NAME: &str = "grep";
const CODING_BASH_PROVIDER_TOOL_NAME: &str = "bash";
const DOCUMENT_EDIT_PROVIDER_TOOL_NAME: &str = "builtin__document_edit";
const HTML_TO_PDF_PROVIDER_TOOL_NAME: &str = "builtin__html_to_pdf";

/// Model-visible descriptions. `read` uses the IronClaw-supported subset of
/// the upstream prompt; the others use the verbatim pinned prompt files (`write.md`, `hashline.md`,
/// `glob.md`, `grep.md` — upstream renders `write`/`edit` with an empty
/// context and the fixture pins the `glob`/`grep` templates raw). `bash`
/// uses the pinned `bash.md` template rendered with IronClaw's surface flags.
const CODING_READ_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_READ_DESCRIPTION;
const CODING_WRITE_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_WRITE_DESCRIPTION;
const CODING_EDIT_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_EDIT_DESCRIPTION;
const CODING_GLOB_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_GLOB_DESCRIPTION;
const CODING_GREP_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_GREP_DESCRIPTION;
const CODING_BASH_DESCRIPTION: &str =
    ironclaw_extension_support::coding::pinned::pinned_assets::CODING_BASH_DESCRIPTION;
const DOCUMENT_EDIT_DESCRIPTION: &str = "Apply structural edits to a .docx/.xlsx/.pptx (accept or reject tracked changes, set a cell formula, clone a slide) and write the result to a new file, preserving every part the edit does not touch";
const HTML_TO_PDF_DESCRIPTION: &str = "Render HTML (headings, paragraphs, lists, emphasis) to a new PDF file; existing PDFs are never edited in place";

/// Schema refs resolving through `super::schemas::resolve_builtin_input_schema_ref`
/// to the coding schema assets. `read` is narrowed to IronClaw's implemented
/// source kinds; the other schemas remain byte-identical to the pinned fixtures.
const CODING_READ_SCHEMA_REF: &str = "schemas/builtin/coding.read.input.v1.json";
const CODING_WRITE_SCHEMA_REF: &str = "schemas/builtin/coding.write.input.v1.json";
const CODING_EDIT_SCHEMA_REF: &str = "schemas/builtin/coding.edit.input.v1.json";
const CODING_GLOB_SCHEMA_REF: &str = "schemas/builtin/coding.glob.input.v1.json";
const CODING_GREP_SCHEMA_REF: &str = "schemas/builtin/coding.grep.input.v1.json";
const CODING_BASH_SCHEMA_REF: &str = "schemas/builtin/coding.bash.input.v1.json";
const DOCUMENT_EDIT_SCHEMA_REF: &str = "schemas/builtin/document_edit.input.v1.json";
const HTML_TO_PDF_SCHEMA_REF: &str = "schemas/builtin/html_to_pdf.input.v1.json";

#[derive(Debug, Clone, Copy)]
struct CodingCapabilityMetadata {
    id: &'static str,
    provider_tool_name: &'static str,
    description: &'static str,
    effects: &'static [EffectKind],
    max_input_bytes: usize,
    schema_ref: &'static str,
}

const CODING_CAPABILITIES: &[CodingCapabilityMetadata] = &[
    CodingCapabilityMetadata {
        id: CODING_READ_CAPABILITY_ID,
        provider_tool_name: CODING_READ_PROVIDER_TOOL_NAME,
        description: CODING_READ_DESCRIPTION,
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
        schema_ref: CODING_READ_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: CODING_WRITE_CAPABILITY_ID,
        provider_tool_name: CODING_WRITE_PROVIDER_TOOL_NAME,
        description: CODING_WRITE_DESCRIPTION,
        effects: &[EffectKind::WriteFilesystem],
        max_input_bytes: MAX_WRITE_FILE_INPUT_BYTES,
        schema_ref: CODING_WRITE_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: CODING_EDIT_CAPABILITY_ID,
        provider_tool_name: CODING_EDIT_PROVIDER_TOOL_NAME,
        description: CODING_EDIT_DESCRIPTION,
        effects: &[
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
        ],
        max_input_bytes: MAX_WRITE_FILE_INPUT_BYTES,
        schema_ref: CODING_EDIT_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: GLOB_CAPABILITY_ID,
        provider_tool_name: CODING_GLOB_PROVIDER_TOOL_NAME,
        description: CODING_GLOB_DESCRIPTION,
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
        schema_ref: CODING_GLOB_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: GREP_CAPABILITY_ID,
        provider_tool_name: CODING_GREP_PROVIDER_TOOL_NAME,
        description: CODING_GREP_DESCRIPTION,
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
        schema_ref: CODING_GREP_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: CODING_BASH_CAPABILITY_ID,
        provider_tool_name: CODING_BASH_PROVIDER_TOOL_NAME,
        description: CODING_BASH_DESCRIPTION,
        effects: &[
            EffectKind::DispatchCapability,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
        schema_ref: CODING_BASH_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: DOCUMENT_EDIT_CAPABILITY_ID,
        provider_tool_name: DOCUMENT_EDIT_PROVIDER_TOOL_NAME,
        description: DOCUMENT_EDIT_DESCRIPTION,
        effects: &[EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        max_input_bytes: MAX_DOCUMENT_EDIT_INPUT_BYTES,
        schema_ref: DOCUMENT_EDIT_SCHEMA_REF,
    },
    CodingCapabilityMetadata {
        id: HTML_TO_PDF_CAPABILITY_ID,
        provider_tool_name: HTML_TO_PDF_PROVIDER_TOOL_NAME,
        description: HTML_TO_PDF_DESCRIPTION,
        effects: &[EffectKind::WriteFilesystem],
        max_input_bytes: MAX_DOCUMENT_EDIT_INPUT_BYTES,
        schema_ref: HTML_TO_PDF_SCHEMA_REF,
    },
];

/// The canonical always-on first-party package with the six pinned coding
/// capabilities, restricted for the selected process backend.
pub fn coding_package(
    process_backend: ProcessBackendKind,
) -> Result<ExtensionPackage, ExtensionError> {
    let mut package = builtin_first_party_package()?;
    super::restrict_package_for_process_backend(&mut package, process_backend)?;
    let manifest = package.manifest;
    ExtensionPackage::from_manifest(manifest, VirtualPath::new("/system/extensions/builtin")?)
}

pub(super) fn coding_manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    CODING_CAPABILITIES
        .iter()
        .map(coding_capability_manifest)
        .collect()
}

fn coding_capability_manifest(
    metadata: &CodingCapabilityMetadata,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(metadata.id)?,
        description: metadata.description.to_string(),
        effects: metadata.effects.to_vec(),
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Model,
        standard_op: None,
        input_schema_ref: CapabilityProfileSchemaRef::new(metadata.schema_ref)?,
        output_schema_ref: None,
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: if metadata.id == CODING_BASH_CAPABILITY_ID {
            bash_resource_profile()
        } else {
            coding_resource_profile()
        },
        origin_gate_matrix: Some(super::first_party_origin_gate_matrix(metadata.id)),
        provider_tool_name: Some(ProviderToolName::new(metadata.provider_tool_name)?),
    })
}

fn coding_resource_profile() -> Option<ResourceProfile> {
    Some(ResourceProfile {
        default_estimate: ResourceEstimate::default()
            .set_wall_clock_ms(super::FIRST_PARTY_DEFAULT_WALL_CLOCK_MS)
            .set_output_bytes(super::FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
        hard_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: Some(super::FIRST_PARTY_MAX_WALL_CLOCK_MS),
            max_output_bytes: Some(CODING_MAX_OUTPUT_BYTES),
            sandbox: None,
        }),
    })
}

/// Resource profile for the process-backed `bash` engine. Mirrors the OMP
/// bash timeout contract (default 300s, ceiling 3600s) rather than the
/// file-tool coding profile's 5s ceiling.
fn bash_resource_profile() -> Option<ResourceProfile> {
    const BASH_DEFAULT_WALL_CLOCK_MS: u64 = 300 * 1000;
    const BASH_MAX_WALL_CLOCK_MS: u64 = 3600 * 1000;
    Some(ResourceProfile {
        default_estimate: ResourceEstimate::default()
            .set_wall_clock_ms(BASH_DEFAULT_WALL_CLOCK_MS)
            .set_output_bytes(super::FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
        hard_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: Some(BASH_MAX_WALL_CLOCK_MS),
            max_output_bytes: Some(CODING_MAX_OUTPUT_BYTES),
            sandbox: None,
        }),
    })
}

/// Register handlers for the six canonical pinned coding capabilities.
pub fn insert_coding_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    let handler = Arc::new(CodingTools::new(
        Arc::new(CodingSnapshotRegistry::default()),
    ));
    for metadata in CODING_CAPABILITIES {
        registry.insert_handler(CapabilityId::new(metadata.id)?, Arc::clone(&handler));
    }
    Ok(())
}

/// First-party handler adapter translating the six coding capability ids to the
/// `coding::pinned` engines.
///
/// Mirrors the stock coding path's resource discipline: bounded input size,
/// bounded output bytes, wall-clock + output-byte accounting. The engine
/// context is built from the already-authorized request (filesystem, mount
/// view, caller scope, run identity) plus the shared snapshot registry that
/// binds hashline edit tags to reads from the SAME run.
pub struct CodingTools {
    snapshots: Arc<CodingSnapshotRegistry>,
    document_state: CodingCapabilityState,
    post_edit_check_seen: crate::post_edit_check::PostEditCheckSeenLines,
}

impl CodingTools {
    pub fn new(snapshots: Arc<CodingSnapshotRegistry>) -> Self {
        Self {
            snapshots,
            document_state: CodingCapabilityState::default(),
            post_edit_check_seen: crate::post_edit_check::PostEditCheckSeenLines::default(),
        }
    }

    async fn dispatch_document_capability(
        &self,
        request: &FirstPartyCapabilityRequest,
        kind: CodingCapabilityKind,
    ) -> Result<CodingCapabilityOutput, FirstPartyCapabilityError> {
        let coding_request = CodingCapabilityRequest::new(
            &request.capability_id,
            kind,
            &request.scope,
            request.run_id,
            request.mounts.as_ref(),
            Arc::clone(&request.services.filesystem),
            &request.input,
        );
        self.document_state
            .dispatch(&coding_request)
            .await
            .map_err(legacy_coding_error)
    }
}

fn is_structured_document_path(input: &serde_json::Value) -> bool {
    let Some(path) = input.get("path").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = path.to_ascii_lowercase();
    [".docx", ".xlsx", ".pptx", ".pdf"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

#[async_trait]
impl FirstPartyCapabilityHandler for CodingTools {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let Some(metadata) = coding_capability_metadata(request.capability_id.as_str()) else {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            ));
        };
        super::bounded_input_size_with_max(&request.input, metadata.max_input_bytes)?;
        let start = Instant::now();
        let mounts = request.mounts.clone().ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        })?;
        let context = CodingEngineContext {
            filesystem: Arc::clone(&request.services.filesystem),
            artifact_reader: request.services.artifact_reader.clone(),
            mounts,
            scope: request.scope.clone(),
            run_id: request.run_id,
            snapshots: Arc::clone(&self.snapshots),
            process: Some(Arc::new(crate::process_port::RuntimeProcessPortExecutor(
                Arc::clone(&request.services.process),
            ))),
        };
        let mut display_preview = None;
        let mut output = match request.capability_id.as_str() {
            CODING_READ_CAPABILITY_ID if is_structured_document_path(&request.input) => {
                let result = self
                    .dispatch_document_capability(&request, CodingCapabilityKind::ReadFile)
                    .await?;
                display_preview = result.display_preview;
                Ok(result.output)
            }
            CODING_READ_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::read(&context, request.input.clone())
                    .await
            }
            CODING_WRITE_CAPABILITY_ID if is_structured_document_path(&request.input) => {
                let result = self
                    .dispatch_document_capability(&request, CodingCapabilityKind::WriteFile)
                    .await?;
                display_preview = result.display_preview;
                Ok(result.output)
            }
            CODING_WRITE_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::write(&context, request.input.clone())
                    .await
            }
            CODING_EDIT_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::edit(&context, request.input.clone())
                    .await
            }
            GLOB_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::glob(&context, request.input.clone())
                    .await
            }
            GREP_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::grep(&context, request.input.clone())
                    .await
            }
            CODING_BASH_CAPABILITY_ID => {
                ironclaw_extension_support::coding::pinned::bash(&context, request.input.clone())
                    .await
            }
            DOCUMENT_EDIT_CAPABILITY_ID => {
                let result = self
                    .dispatch_document_capability(&request, CodingCapabilityKind::DocumentEdit)
                    .await?;
                display_preview = result.display_preview;
                Ok(result.output)
            }
            HTML_TO_PDF_CAPABILITY_ID => {
                let result = self
                    .dispatch_document_capability(&request, CodingCapabilityKind::HtmlToPdf)
                    .await?;
                display_preview = result.display_preview;
                Ok(result.output)
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        }
        .map_err(coding_error)?;
        let mut process_count = 0;
        if request.capability_id.as_str() == CODING_BASH_CAPABILITY_ID {
            // The bash engine runs one command through the process port.
            process_count = 1;
        }
        if matches!(
            request.capability_id.as_str(),
            CODING_WRITE_CAPABILITY_ID | CODING_EDIT_CAPABILITY_ID
        ) && let Some(service) = &request.services.post_edit_check
        {
            // Hashline edit carries paths in section headers rather than a
            // top-level field. A multi-file edit runs one advisory check,
            // rooted at its deterministic first section.
            let edited_scoped_path = match request.capability_id.as_str() {
                CODING_WRITE_CAPABILITY_ID => request
                    .input
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                CODING_EDIT_CAPABILITY_ID => {
                    ironclaw_extension_support::coding::pinned::first_edit_target_path(
                        &request.input,
                    )
                }
                _ => None,
            };
            if let Some(check) = crate::post_edit_check::run_post_edit_check(
                &self.post_edit_check_seen,
                service.process.as_ref(),
                &request.scope,
                request.mounts.as_ref(),
                edited_scoped_path.as_deref(),
                &service.config,
            )
            .await
            {
                if let Some(object) = output.as_object_mut() {
                    object.insert("post_edit_check".to_string(), check);
                }
                process_count = 1;
            }
        }
        let canonical_output_digest = canonical_output_digest(&output)?;
        let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let (output, pending_artifact, output_bytes) = artifact_backed_output(&request, output)
            .await
            .map_err(|error| {
                error.with_usage(ResourceUsage::default().set_wall_clock_ms(wall_clock_ms))
            })?;
        let result = FirstPartyCapabilityResult::new(
            output,
            ResourceUsage::default()
                .set_wall_clock_ms(wall_clock_ms)
                .set_output_bytes(output_bytes)
                .set_process_count(process_count),
        )
        .with_canonical_output_digest(canonical_output_digest)
        .with_display_preview(display_preview);
        Ok(match pending_artifact {
            Some(artifact) => result.with_pending_artifact(artifact),
            None => result,
        })
    }
}

fn canonical_output_digest(
    output: &serde_json::Value,
) -> Result<OutputDigest, FirstPartyCapabilityError> {
    ContentDigest::from_json_value(output)
        .map(|digest| OutputDigest::new(digest.0))
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))
}

async fn artifact_backed_output(
    request: &FirstPartyCapabilityRequest,
    output: serde_json::Value,
) -> Result<(serde_json::Value, Option<PendingFirstPartyArtifact>, u64), FirstPartyCapabilityError>
{
    let serde_json::Value::Object(mut object) = output else {
        let output_bytes =
            super::bounded_output_bytes(&output, super::FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        return Ok((output, None, output_bytes));
    };
    let Some(serde_json::Value::String(raw_output)) = object.remove("output") else {
        let output = serde_json::Value::Object(object);
        let output_bytes =
            super::bounded_output_bytes(&output, super::FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        return Ok((output, None, output_bytes));
    };
    if raw_output.len() <= ARTIFACT_INLINE_PREVIEW_MAX_BYTES {
        object.insert("output".to_string(), serde_json::Value::String(raw_output));
        let output = serde_json::Value::Object(object);
        let output_bytes =
            super::bounded_output_bytes(&output, super::FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        return Ok((output, None, output_bytes));
    }

    let raw_len = coding_artifact_len(raw_output.len())?;
    let namespace = request
        .services
        .artifact_namespace
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend))?;
    let persistence = request
        .services
        .artifact_persistence
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::MethodMissing))?;
    let handle = persistence
        .allocate(ArtifactWriteMetadata {
            write_key: Some(request.scope.invocation_id),
            owner_scope: ArtifactOwnerScope::from_resource_scope(&request.scope),
            namespace,
            producer_capability_id: request.capability_id.clone(),
            content_type: "text/plain; charset=utf-8".to_string(),
            expected_bytes: Some(raw_len),
        })
        .await
        .map_err(artifact_write_error)?;
    let artifact_ref = ArtifactRef::new(handle.artifact_id());
    let preview = artifact_preview(&raw_output, &artifact_ref, request.capability_id.as_str());
    object.insert("output".to_string(), serde_json::Value::String(preview));
    object.insert(
        "artifact_ref".to_string(),
        serde_json::Value::String(artifact_ref.to_string()),
    );
    object.insert("total_bytes".to_string(), serde_json::json!(raw_len));
    let output = serde_json::Value::Object(object);
    Ok((
        output,
        Some(PendingFirstPartyArtifact {
            handle,
            bytes: raw_output.into_bytes(),
        }),
        raw_len,
    ))
}

fn coding_artifact_len(raw_len: usize) -> Result<u64, FirstPartyCapabilityError> {
    let raw_len = u64::try_from(raw_len)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Resource))?;
    if raw_len > CODING_MAX_OUTPUT_BYTES {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    Ok(raw_len)
}

fn artifact_write_error(error: ArtifactWriteError) -> FirstPartyCapabilityError {
    let kind = match error {
        ArtifactWriteError::Budget => RuntimeDispatchErrorKind::Resource,
        ArtifactWriteError::InvalidHandle
        | ArtifactWriteError::DigestMismatch
        | ArtifactWriteError::Storage => RuntimeDispatchErrorKind::OperationFailed,
    };
    FirstPartyCapabilityError::new(kind)
}

/// Bound a spilled coding result for inline transport.
///
/// The shape is capability-aware because the two output families have opposite
/// centres of gravity:
///
/// * Command output (`bash`) carries its exit-code and wall-time notices at the
///   very end, so a head-only cut would drop the part the model reads first.
///   That family keeps the pinned head+tail shape.
/// * Document windows (`read`, `grep`, `glob`) are a contiguous span the model
///   explicitly asked for. Eliding their middle returns a hole: the model then
///   spends a second call re-reading the gap it was never told about, which is
///   exactly the paging spiral measured on transcript-heavy tasks. That family
///   gets a contiguous head plus the artifact selector to resume from, matching
///   the engine's own `[… Use <artifact>:<line> to continue]` idiom rather than
///   inventing a second continuation convention.
fn artifact_preview(raw_output: &str, artifact_ref: &ArtifactRef, capability_id: &str) -> String {
    if capability_id == CODING_BASH_CAPABILITY_ID {
        return command_output_preview(raw_output, artifact_ref);
    }
    document_window_preview(raw_output, artifact_ref)
}

fn command_output_preview(raw_output: &str, artifact_ref: &ArtifactRef) -> String {
    let footer = format!("\n[raw output: {artifact_ref}]");
    let marker = "\n\n... [artifact output elided] ...\n\n";
    let content_budget = CODING_ARTIFACT_PREVIEW_MAX_BYTES
        .saturating_sub(footer.len())
        .saturating_sub(marker.len());
    let head_budget = content_budget / 2;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head_end = floor_char_boundary(raw_output, head_budget);
    let tail_start = ceil_char_boundary(raw_output, raw_output.len().saturating_sub(tail_budget));
    format!(
        "{}{marker}{}{}",
        &raw_output[..head_end],
        &raw_output[tail_start..],
        footer
    )
}

/// Contiguous head plus a resume selector. Never elides the middle.
fn document_window_preview(raw_output: &str, artifact_ref: &ArtifactRef) -> String {
    // Reserve enough room that the rendered footer cannot push the preview past
    // the budget once its counts are substituted.
    const FOOTER_RESERVE_BYTES: usize = 160;
    let content_budget = CODING_ARTIFACT_PREVIEW_MAX_BYTES.saturating_sub(FOOTER_RESERVE_BYTES);
    let mut head_end = floor_char_boundary(raw_output, content_budget);
    // Cut on a line boundary so the resumed selector starts a whole line and
    // the visible tail is never a half-rendered Hashline row.
    if let Some(last_newline) = raw_output[..head_end].rfind('\n') {
        head_end = last_newline + 1;
    }
    let head = &raw_output[..head_end];
    let remaining_bytes = raw_output.len().saturating_sub(head_end);
    if remaining_bytes == 0 {
        return head.to_string();
    }
    // Artifact selectors index the artifact's own lines, so the next unread
    // line is one past the count fully contained in the head.
    let next_line = head.lines().count().saturating_add(1);
    format!(
        "{head}\n[{remaining_bytes} more bytes in artifact. \
         Use {artifact_ref}:{next_line} to continue]"
    )
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn coding_capability_metadata(capability_id: &str) -> Option<CodingCapabilityMetadata> {
    CODING_CAPABILITIES
        .iter()
        .copied()
        .find(|metadata| metadata.id == capability_id)
}

/// Map a coding engine failure onto the first-party capability error surface.
///
/// The pinned coding error text is the model-visible contract, but it is free
/// text (paths, newlines) that the strict `SafeSummary` validator rejects,
/// so it rides the untrusted diagnostic channel exactly like the stock shell
/// path routes raw failure causes.
fn coding_error(error: CodingEngineError) -> FirstPartyCapabilityError {
    let kind = match error.kind() {
        CodingEngineErrorKind::Input => RuntimeDispatchErrorKind::InputEncode,
        CodingEngineErrorKind::FilesystemDenied | CodingEngineErrorKind::PathResolution => {
            RuntimeDispatchErrorKind::FilesystemDenied
        }
        CodingEngineErrorKind::ResourceLimit => RuntimeDispatchErrorKind::Resource,
        _ => RuntimeDispatchErrorKind::OperationFailed,
    };
    FirstPartyCapabilityError::dispatch_with_diagnostic(
        kind,
        None,
        bounded_diagnostic(
            error.message(),
            super::FIRST_PARTY_MAX_OUTPUT_BYTES as usize,
        ),
    )
}

fn legacy_coding_error(error: CodingCapabilityError) -> FirstPartyCapabilityError {
    match error.safe_summary() {
        Some(summary) => FirstPartyCapabilityError::with_safe_summary(error.kind(), summary),
        None => FirstPartyCapabilityError::new(error.kind()),
    }
}

fn bounded_diagnostic(message: &str, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message.to_string();
    }
    const MARKER: &str = "\n[diagnostic truncated]";
    let content_limit = max_bytes.saturating_sub(MARKER.len());
    let mut end = content_limit.min(message.len());
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = message[..end].to_string();
    if MARKER.len() <= max_bytes {
        bounded.push_str(MARKER);
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_artifacts_have_rendering_headroom_above_the_maximum_read_size() {
        let manifests = coding_manifests().expect("coding manifests");

        for manifest in manifests {
            let profile = manifest.resource_profile.expect("coding resource profile");
            let ceiling = profile.hard_ceiling.expect("coding hard ceiling");
            assert_eq!(
                ceiling.max_output_bytes,
                Some(CODING_MAX_OUTPUT_BYTES),
                "{} must use the bounded coding artifact ceiling",
                manifest.id
            );
        }
    }

    #[test]
    fn coding_artifact_size_is_rejected_at_the_boundary_before_allocation() {
        let at_limit = usize::try_from(CODING_MAX_OUTPUT_BYTES).expect("ceiling fits usize");
        let over_limit = at_limit + 1;

        assert_eq!(
            coding_artifact_len(at_limit).expect("artifact at ceiling"),
            CODING_MAX_OUTPUT_BYTES
        );
        let error = coding_artifact_len(over_limit).expect_err("artifact above ceiling");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Resource));
    }

    #[test]
    fn diagnostic_bound_preserves_utf8_and_never_exceeds_limit() {
        let bounded = bounded_diagnostic("é".repeat(100).as_str(), 31);

        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(bounded.len() <= 31, "{} bytes", bounded.len());
        assert!(bounded.ends_with("[diagnostic truncated]"));
    }

    #[test]
    fn canonical_digest_covers_the_full_output_before_artifact_previewing() {
        let prefix = "a".repeat(8 * 1024);
        let suffix = "z".repeat(8 * 1024);
        let first = serde_json::json!({
            "output": format!("{prefix}{}{}", "m".repeat(32 * 1024), suffix),
        });
        let same = first.clone();
        let changed_middle = serde_json::json!({
            "output": format!("{prefix}{}{}", "n".repeat(32 * 1024), suffix),
        });

        assert_eq!(
            canonical_output_digest(&first).expect("first digest"),
            canonical_output_digest(&same).expect("same digest"),
        );
        assert_ne!(
            canonical_output_digest(&first).expect("first digest"),
            canonical_output_digest(&changed_middle).expect("changed digest"),
        );
    }
}
