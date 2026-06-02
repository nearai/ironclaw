use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use ironclaw_events::AuditSink;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage, DecisionSummary,
    EffectKind, ExtensionId, PermissionMode, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_memory::{
    ChunkingMemoryDocumentIndexer, DocumentMetadata, FilesystemMemoryDocumentRepository,
    MemoryBackend, MemoryBackendCapabilities, MemoryBackendWriteOptions, MemoryContext,
    MemoryDocumentPath, MemoryDocumentScope, MemoryEventSinkError, MemorySearchRequest,
    MemoryWriteOutcome, PromptSafetyAllowanceId, PromptSafetyReasonCode, PromptWriteOperation,
    PromptWriteSafetyEvent, PromptWriteSafetyEventKind, PromptWriteSafetyEventSink,
    RepositoryMemoryBackend, content_bytes_sha256,
};
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, FirstPartyCapabilityResult};

use super::{first_party_capability_manifest, input_error, operation_error, resource_profile};

pub const MEMORY_SEARCH_CAPABILITY_ID: &str = "builtin.memory_search";
pub const MEMORY_WRITE_CAPABILITY_ID: &str = "builtin.memory_write";
pub const MEMORY_READ_CAPABILITY_ID: &str = "builtin.memory_read";
pub const MEMORY_TREE_CAPABILITY_ID: &str = "builtin.memory_tree";

const MEMORY_PATH: &str = "MEMORY.md";
const HEARTBEAT_PATH: &str = "HEARTBEAT.md";
const BOOTSTRAP_PATH: &str = "BOOTSTRAP.md";
const MAX_MEMORY_PATCH_RETRIES: usize = 8;
const MEMORY_PROMPT_SAFETY_EXTENSION_ID: &str = "memory.prompt_safety";

struct MemoryServices {
    scope: MemoryDocumentScope,
    context: MemoryContext,
    backend: Arc<dyn MemoryBackend>,
}

#[derive(Default)]
pub(super) struct MemoryCapabilityState {
    cached_backend: Mutex<Option<CachedMemoryBackend>>,
}

impl std::fmt::Debug for MemoryCapabilityState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryCapabilityState")
            .field("cached_backend", &"<cached-memory-backend>")
            .finish()
    }
}

struct CachedMemoryBackend {
    filesystem: Arc<dyn RootFilesystem>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    backend: Arc<dyn MemoryBackend>,
}

struct AuditPromptWriteSafetyEventSink {
    audit_sink: Arc<dyn AuditSink>,
}

#[async_trait]
impl PromptWriteSafetyEventSink for AuditPromptWriteSafetyEventSink {
    async fn record_prompt_write_safety_event(
        &self,
        event: PromptWriteSafetyEvent,
    ) -> Result<(), MemoryEventSinkError> {
        let Some(audit_context) = event.audit_context.as_ref() else {
            return Err(MemoryEventSinkError::new(
                "prompt-write safety event missing audit context",
            ));
        };
        let resource_scope = audit_context.resource_scope.clone();
        let record = AuditEnvelope {
            event_id: AuditEventId::new(),
            correlation_id: audit_context.correlation_id,
            stage: match event.kind {
                PromptWriteSafetyEventKind::Rejected => AuditStage::Denied,
                _ => AuditStage::After,
            },
            timestamp: Utc::now(),
            tenant_id: resource_scope.tenant_id,
            user_id: resource_scope.user_id,
            agent_id: resource_scope.agent_id,
            project_id: resource_scope.project_id,
            mission_id: resource_scope.mission_id,
            thread_id: resource_scope.thread_id,
            invocation_id: resource_scope.invocation_id,
            process_id: None,
            approval_request_id: None,
            extension_id: Some(ExtensionId::new(MEMORY_PROMPT_SAFETY_EXTENSION_ID).map_err(
                |error| {
                    MemoryEventSinkError::new(format!(
                        "invalid memory prompt-safety extension id: {error}"
                    ))
                },
            )?),
            action: ActionSummary {
                kind: prompt_write_action_kind(event.operation).to_string(),
                target: None,
                effects: vec![EffectKind::WriteFilesystem],
            },
            decision: DecisionSummary {
                kind: event
                    .reason_code
                    .map(prompt_safety_reason_projection_kind)
                    .unwrap_or_else(|| prompt_safety_event_kind_label(event.kind))
                    .to_string(),
                reason: None,
                actor: None,
            },
            result: Some(ActionResultSummary {
                success: event.kind != PromptWriteSafetyEventKind::Rejected,
                status: Some(encode_prompt_safety_metadata(&event)),
                output_bytes: None,
            }),
        };
        self.audit_sink
            .emit_audit(record)
            .await
            .map_err(|error| MemoryEventSinkError::new(error.to_string()))
    }
}

struct MemoryWriteCommand {
    resolved_path: String,
    path: MemoryDocumentPath,
    metadata_overlay: Option<DocumentMetadata>,
    operation: MemoryWriteOperation,
}

enum MemoryWriteOperation {
    ClearBootstrap,
    Patch {
        old_string: String,
        new_string: String,
        replace_all: bool,
    },
    Append {
        content: String,
    },
    Replace {
        content: String,
    },
}

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            MEMORY_SEARCH_CAPABILITY_ID,
            "Search Reborn persistent memory documents in the current tenant/user/agent/project scope",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            MEMORY_WRITE_CAPABILITY_ID,
            "Write, append, or patch Reborn persistent memory documents in the current tenant/user/agent/project scope",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            MEMORY_READ_CAPABILITY_ID,
            "Read a Reborn persistent memory document in the current tenant/user/agent/project scope",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            MEMORY_TREE_CAPABILITY_ID,
            "List Reborn persistent memory documents as a compact tree",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            resource_profile(),
        )?,
    ])
}

pub(super) async fn dispatch(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let start = std::time::Instant::now();
    let services = memory_services(state, request)?;
    let output = match request.capability_id.as_str() {
        MEMORY_SEARCH_CAPABILITY_ID => dispatch_search(&services, &request.input).await?,
        MEMORY_WRITE_CAPABILITY_ID => dispatch_write(&services, &request.input).await?,
        MEMORY_READ_CAPABILITY_ID => dispatch_read(&services, &request.input).await?,
        MEMORY_TREE_CAPABILITY_ID => dispatch_tree(&services, &request.input).await?,
        _ => return Err(operation_error()),
    };
    let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    Ok(FirstPartyCapabilityResult::new(
        output,
        ResourceUsage {
            wall_clock_ms,
            ..ResourceUsage::default()
        },
    ))
}

fn memory_services(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<MemoryServices, FirstPartyCapabilityError> {
    ensure_memory_mount(
        request,
        request.capability_id.as_str() == MEMORY_WRITE_CAPABILITY_ID,
    )?;
    let scope = MemoryDocumentScope::new_with_agent(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
        request.scope.agent_id.as_ref().map(|id| id.as_str()),
        request.scope.project_id.as_ref().map(|id| id.as_str()),
    )
    .map_err(|_| input_error())?;
    let context = MemoryContext::new(scope.clone()).with_audit_context(
        request.scope.clone(),
        ironclaw_host_api::CorrelationId::new(),
    );
    let backend = state.backend_for(request)?;
    Ok(MemoryServices {
        scope,
        context,
        backend,
    })
}

impl MemoryCapabilityState {
    fn backend_for(
        &self,
        request: &FirstPartyCapabilityRequest,
    ) -> Result<Arc<dyn MemoryBackend>, FirstPartyCapabilityError> {
        let mut cached_backend = self.cached_backend.lock().map_err(|_| operation_error())?;
        if let Some(cached) = cached_backend.as_ref()
            && Arc::ptr_eq(&cached.filesystem, &request.services.filesystem)
            && audit_sinks_match(
                cached.audit_sink.as_ref(),
                request.services.audit_sink.as_ref(),
            )
        {
            return Ok(Arc::clone(&cached.backend));
        }

        let filesystem = Arc::clone(&request.services.filesystem);
        let audit_sink = request.services.audit_sink.clone();
        let backend = build_backend(Arc::clone(&filesystem), audit_sink.clone());
        *cached_backend = Some(CachedMemoryBackend {
            filesystem,
            audit_sink,
            backend: Arc::clone(&backend),
        });
        Ok(backend)
    }
}

fn audit_sinks_match(
    left: Option<&Arc<dyn AuditSink>>,
    right: Option<&Arc<dyn AuditSink>>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

fn build_backend(
    filesystem: Arc<dyn RootFilesystem>,
    audit_sink: Option<Arc<dyn AuditSink>>,
) -> Arc<dyn MemoryBackend> {
    let repository = Arc::new(FilesystemMemoryDocumentRepository::new(filesystem));
    let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(Arc::clone(&repository)));
    let mut backend = RepositoryMemoryBackend::new(Arc::clone(&repository))
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            metadata: true,
            versioning: true,
            prompt_write_safety: true,
            full_text_search: true,
            delete: true,
            transactions: true,
            ..MemoryBackendCapabilities::default()
        });
    if let Some(audit_sink) = audit_sink {
        backend = backend.with_prompt_write_safety_event_sink(Arc::new(
            AuditPromptWriteSafetyEventSink { audit_sink },
        ));
    }
    Arc::new(backend)
}

fn ensure_memory_mount(
    request: &FirstPartyCapabilityRequest,
    write: bool,
) -> Result<(), FirstPartyCapabilityError> {
    let Some(mounts) = &request.mounts else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    let Some(grant) = mounts
        .mounts
        .iter()
        .find(|grant| grant.alias.as_str() == "/memory" && grant.target.as_str() == "/memory")
    else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    let permissions = &grant.permissions;
    if !permissions.read
        || !permissions.list
        || (write && (!permissions.write || !permissions.delete))
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    Ok(())
}

async fn dispatch_search(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let query = search_query(input)?;
    let limit = optional_u64(input, "limit").unwrap_or(5).clamp(1, 20) as usize;
    let request = MemorySearchRequest::new(query)
        .map_err(|_| input_error())?
        .with_limit(limit)
        .with_pre_fusion_limit(limit.max(20))
        .with_vector(false);
    let results = services
        .backend
        .search(&services.context, request)
        .await
        .map_err(|_| operation_error())?;
    let result_values = results
        .into_iter()
        .map(|result| {
            json!({
                "content": result.snippet,
                "score": result.score,
                "path": result.path.relative_path(),
                "is_hybrid_match": result.is_hybrid(),
            })
        })
        .collect::<Vec<_>>();
    let result_count = result_values.len();
    Ok(json!({
        "query": query,
        "results": result_values,
        "result_count": result_count,
    }))
}

fn search_query(input: &Value) -> Result<&str, FirstPartyCapabilityError> {
    for key in ["query", "q", "text", "pattern"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    Err(input_error())
}

async fn dispatch_write(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let MemoryWriteCommand {
        resolved_path,
        path,
        metadata_overlay,
        operation,
    } = parse_write_command(&services.scope, input)?;
    match operation {
        MemoryWriteOperation::ClearBootstrap => {
            clear_bootstrap_document(services, &path, &resolved_path, metadata_overlay.as_ref())
                .await
        }
        MemoryWriteOperation::Patch {
            old_string,
            new_string,
            replace_all,
        } => {
            patch_document(
                services,
                &path,
                &resolved_path,
                metadata_overlay.as_ref(),
                &old_string,
                &new_string,
                replace_all,
            )
            .await
        }
        MemoryWriteOperation::Append { content } => {
            append_document(
                services,
                &path,
                metadata_overlay.as_ref(),
                content.as_bytes(),
            )
            .await?;
            Ok(json!({
                "status": "written",
                "path": resolved_path,
                "append": true,
                "content_length": content.len(),
            }))
        }
        MemoryWriteOperation::Replace { content } => {
            services
                .backend
                .write_document_with_backend_options(
                    &services.context,
                    &path,
                    content.as_bytes(),
                    &write_options(metadata_overlay.as_ref()),
                )
                .await
                .map_err(|_| operation_error())?;
            Ok(json!({
                "status": "written",
                "path": resolved_path,
                "append": false,
                "content_length": content.len(),
            }))
        }
    }
}

fn parse_write_command(
    scope: &MemoryDocumentScope,
    input: &Value,
) -> Result<MemoryWriteCommand, FirstPartyCapabilityError> {
    let target = match input.get("target") {
        Some(Value::String(target)) => target.as_str(),
        Some(_) => return Err(input_error()),
        None => "daily_log",
    };
    reject_local_or_traversal_path(target)?;

    let resolved_path = resolve_target_path(target, input)?;
    let path = document_path(scope, &resolved_path)?;
    let metadata_overlay = input
        .get("metadata")
        .filter(|metadata| metadata.is_object())
        .map(DocumentMetadata::from_value);

    let operation = if target == "bootstrap" {
        MemoryWriteOperation::ClearBootstrap
    } else if let Some(old_string) = input.get("old_string").and_then(Value::as_str) {
        if old_string.is_empty() {
            return Err(input_error());
        }
        MemoryWriteOperation::Patch {
            old_string: old_string.to_string(),
            new_string: required_str(input, "new_string")?.to_string(),
            replace_all: input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    } else {
        let content = input.get("content").and_then(Value::as_str).unwrap_or("");
        if content.trim().is_empty() {
            return Err(input_error());
        }
        if target == "daily_log" || input.get("append").and_then(Value::as_bool).unwrap_or(true) {
            MemoryWriteOperation::Append {
                content: content.to_string(),
            }
        } else {
            MemoryWriteOperation::Replace {
                content: content.to_string(),
            }
        }
    };

    Ok(MemoryWriteCommand {
        resolved_path,
        path,
        metadata_overlay,
        operation,
    })
}

async fn clear_bootstrap_document(
    services: &MemoryServices,
    path: &MemoryDocumentPath,
    resolved_path: &str,
    metadata_overlay: Option<&DocumentMetadata>,
) -> Result<Value, FirstPartyCapabilityError> {
    if path.relative_path() != BOOTSTRAP_PATH || resolved_path != BOOTSTRAP_PATH {
        return Err(operation_error());
    }
    let context = services
        .context
        .clone()
        .with_prompt_write_safety_allowance(PromptSafetyAllowanceId::empty_prompt_file_clear());
    services
        .backend
        .write_document_with_backend_options(&context, path, b"", &write_options(metadata_overlay))
        .await
        .map_err(|_| operation_error())?;
    Ok(json!({
        "status": "cleared",
        "path": resolved_path,
        "message": "BOOTSTRAP.md cleared.",
    }))
}

async fn patch_document(
    services: &MemoryServices,
    path: &MemoryDocumentPath,
    resolved_path: &str,
    metadata_overlay: Option<&DocumentMetadata>,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<Value, FirstPartyCapabilityError> {
    let options = write_options(metadata_overlay);
    for _ in 0..MAX_MEMORY_PATCH_RETRIES {
        let Some(bytes) = services
            .backend
            .read_document(&services.context, path)
            .await
            .map_err(|_| operation_error())?
        else {
            return Err(operation_error());
        };
        let existing = String::from_utf8(bytes).map_err(|_| operation_error())?;
        let expected = content_bytes_sha256(existing.as_bytes());
        let replacements = existing.matches(old_string).count();
        if replacements == 0 {
            return Err(input_error());
        }
        let replacement_count = if replace_all { replacements } else { 1 };
        let updated = if replace_all {
            existing.replace(old_string, new_string)
        } else {
            existing.replacen(old_string, new_string, 1)
        };
        let outcome = services
            .backend
            .compare_and_write_document_with_backend_options(
                &services.context,
                path,
                Some(&expected),
                updated.as_bytes(),
                &options,
            )
            .await
            .map_err(|_| operation_error())?;
        if outcome == MemoryWriteOutcome::Written {
            return Ok(json!({
                "status": "patched",
                "path": resolved_path,
                "replacements": replacement_count,
                "content_length": updated.len(),
            }));
        }
    }
    Err(operation_error())
}

async fn append_document(
    services: &MemoryServices,
    path: &MemoryDocumentPath,
    metadata_overlay: Option<&DocumentMetadata>,
    bytes: &[u8],
) -> Result<(), FirstPartyCapabilityError> {
    let options = write_options(metadata_overlay);
    services
        .backend
        .append_document_with_backend_options(&services.context, path, bytes, &options)
        .await
        .map_err(|_| operation_error())
}

fn write_options(metadata_overlay: Option<&DocumentMetadata>) -> MemoryBackendWriteOptions {
    MemoryBackendWriteOptions {
        metadata_overlay: metadata_overlay.cloned(),
    }
}

async fn dispatch_read(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let path = required_str(input, "path")?;
    reject_local_or_traversal_path(path)?;
    if input.get("version").is_some()
        || input.get("list_versions").and_then(Value::as_bool) == Some(true)
    {
        return Err(input_error());
    }
    let path = document_path(&services.scope, path)?;
    let Some(bytes) = services
        .backend
        .read_document(&services.context, &path)
        .await
        .map_err(|_| operation_error())?
    else {
        return Err(input_error());
    };
    let content = String::from_utf8(bytes).map_err(|_| operation_error())?;
    Ok(json!({
        "path": path.relative_path(),
        "content": content,
        "word_count": content.split_whitespace().count(),
    }))
}

async fn dispatch_tree(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let root = input.get("path").and_then(Value::as_str).unwrap_or("");
    if !root.is_empty() {
        reject_local_or_traversal_path(root)?;
    }
    let depth = optional_u64(input, "depth").unwrap_or(1).clamp(1, 10) as usize;
    let mut paths = services
        .backend
        .list_documents(&services.context, &services.scope)
        .await
        .map_err(|_| operation_error())?
        .into_iter()
        .map(|path| path.relative_path().to_string())
        .collect::<Vec<_>>();
    paths.sort();
    Ok(Value::Array(tree_for_paths(
        &paths,
        root.trim_matches('/'),
        depth,
    )))
}

fn tree_for_paths(paths: &[String], root: &str, max_depth: usize) -> Vec<Value> {
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{}/", root.trim_matches('/'))
    };
    let mut children = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut files = Vec::new();
    for path in paths {
        let Some(remainder) = path.strip_prefix(&prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        if let Some((dir, _)) = remainder.split_once('/') {
            children
                .entry(dir.to_string())
                .or_default()
                .push(path.clone());
        } else {
            files.push(remainder.to_string());
        }
    }

    let mut output = Vec::new();
    for (dir, child_paths) in children {
        let display = format!("{dir}/");
        if max_depth <= 1 {
            output.push(Value::String(display));
        } else {
            let child_root = if root.is_empty() {
                dir
            } else {
                format!("{root}/{dir}")
            };
            let child_tree = tree_for_paths(&child_paths, &child_root, max_depth - 1);
            if child_tree.is_empty() {
                output.push(Value::String(display));
            } else {
                output.push(json!({ (display): child_tree }));
            }
        }
    }
    output.extend(files.into_iter().map(Value::String));
    output
}

fn resolve_target_path(target: &str, input: &Value) -> Result<String, FirstPartyCapabilityError> {
    match target {
        "memory" => Ok(MEMORY_PATH.to_string()),
        "heartbeat" => Ok(HEARTBEAT_PATH.to_string()),
        "bootstrap" => Ok(BOOTSTRAP_PATH.to_string()),
        "daily_log" => {
            let timezone = match input.get("timezone").and_then(Value::as_str) {
                Some(value) => value.parse::<Tz>().map_err(|_| input_error())?,
                None => Tz::UTC,
            };
            let now = Utc::now().with_timezone(&timezone);
            Ok(format!("daily/{}.md", now.format("%Y-%m-%d")))
        }
        path => Ok(path.to_string()),
    }
}

fn document_path(
    scope: &MemoryDocumentScope,
    relative_path: &str,
) -> Result<MemoryDocumentPath, FirstPartyCapabilityError> {
    MemoryDocumentPath::new_with_agent(
        scope.tenant_id(),
        scope.user_id(),
        scope.agent_id(),
        scope.project_id(),
        relative_path,
    )
    .map_err(|_| input_error())
}

fn required_str<'a>(
    input: &'a Value,
    key: &'static str,
) -> Result<&'a str, FirstPartyCapabilityError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(input_error)
}

fn optional_u64(input: &Value, key: &'static str) -> Option<u64> {
    input.get(key).and_then(Value::as_u64)
}

fn reject_local_or_traversal_path(path: &str) -> Result<(), FirstPartyCapabilityError> {
    if path.contains('\\') || looks_like_filesystem_path(path) || contains_traversal(path) {
        return Err(input_error());
    }
    Ok(())
}

fn contains_traversal(path: &str) -> bool {
    path.split('/').any(|segment| segment == "..")
}

fn looks_like_filesystem_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') || path.starts_with("~/") {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn prompt_write_action_kind(operation: PromptWriteOperation) -> &'static str {
    match operation {
        PromptWriteOperation::Write => "write_file",
        PromptWriteOperation::Append => "append_file",
        PromptWriteOperation::Patch => "patch_file",
        PromptWriteOperation::Import => "memory_import",
        PromptWriteOperation::Seed => "memory_seed",
        PromptWriteOperation::ProfileUpdate => "profile_update",
        PromptWriteOperation::AdminSystemPromptUpdate => "admin_system_prompt_update",
    }
}

fn prompt_safety_reason_projection_kind(reason: PromptSafetyReasonCode) -> &'static str {
    match reason {
        PromptSafetyReasonCode::HighRiskPromptInjection => "prompt_high_risk",
        PromptSafetyReasonCode::CriticalPromptInjection => "prompt_critical",
        PromptSafetyReasonCode::PromptWritePolicyUnavailable => "prompt_policy_unavailable",
        PromptSafetyReasonCode::PromptWritePolicyMisconfigured => "prompt_policy_misconfigured",
        PromptSafetyReasonCode::ProtectedPathRegistryUnavailable => "protected_registry_missing",
        PromptSafetyReasonCode::PromptWriteBypassNotAllowed => "prompt_bypass_denied",
        PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable => "prompt_event_unavailable",
    }
}

fn prompt_safety_event_kind_label(kind: PromptWriteSafetyEventKind) -> &'static str {
    match kind {
        PromptWriteSafetyEventKind::Checked => "prompt_write_safety_checked",
        PromptWriteSafetyEventKind::Warned => "prompt_write_safety_warned",
        PromptWriteSafetyEventKind::Rejected => "prompt_write_safety_rejected",
        PromptWriteSafetyEventKind::BypassAllowed => "prompt_write_safety_bypass_allowed",
    }
}

fn encode_prompt_safety_metadata(event: &PromptWriteSafetyEvent) -> String {
    let mut pairs = vec![format!(
        "status={}",
        match event.kind {
            PromptWriteSafetyEventKind::Checked => "checked",
            PromptWriteSafetyEventKind::Warned => "warned",
            PromptWriteSafetyEventKind::Rejected => "rejected",
            PromptWriteSafetyEventKind::BypassAllowed => "bypass_allowed",
        }
    )];
    if let Some(path_hash) = &event.relative_path_hash {
        pairs.push(format!("path_hash={path_hash}"));
    }
    if let Some(path_class) = &event.protected_path_class {
        pairs.push(format!("protected_path_class={}", path_class.as_str()));
    }
    if let Some(reason) = event.reason_code {
        pairs.push(format!("reason={}", reason.as_str()));
    }
    if let Some(severity) = event.severity {
        pairs.push(format!("severity={}", severity.as_str()));
    }
    if event.finding_count > 0 {
        pairs.push(format!("findings={}", event.finding_count));
    }
    format!("memory_prompt_safety:v1;{}", pairs.join(";"))
}
