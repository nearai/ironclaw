//! IronClaw memory service facade for Reborn.
//!
//! This module owns the host-facing IronClaw memory operation shapes. Host
//! runtime callers still resolve scope, mounts, grants, approvals, and audit
//! services before calling the service; the default native adapter keeps the
//! existing storage format.

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc};

use crate::{
    ChunkingMemoryDocumentIndexer, DocumentMetadata, FilesystemMemoryDocumentRepository,
    MemoryBackend, MemoryBackendCapabilities, MemoryBackendWriteOptions, MemoryContext,
    MemoryDocumentPath, MemoryDocumentScope, MemorySearchRequest, MemorySearchResult,
    MemoryWriteOutcome, PromptSafetyAllowanceId, PromptWriteSafetyEventSink,
    RepositoryMemoryBackend, content_bytes_sha256,
};
use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::ThreadId;
use serde_json::{Map, Value, json};

// The host-facing operation shapes + the `MemoryService` trait moved to
// `ironclaw_memory`; re-exported so `crate::service::*` and the crate's
// public API stay unchanged while `NativeMemoryService` (below) keeps the native
// adapter behavior here.
pub use ironclaw_memory::{
    MemoryContextProfileId, MemoryInteractionMessage, MemoryInteractionRole, MemoryInvocation,
    MemoryProfileSetStatus, MemoryService, MemoryServiceContextRequest,
    MemoryServiceContextSnippet, MemoryServiceError, MemoryServiceErrorKind,
    MemoryServiceProfileReadResponse, MemoryServiceProfileSetRequest,
    MemoryServiceProfileSetResponse, MemoryServiceReadRequest, MemoryServiceReadResponse,
    MemoryServiceRecordRequest, MemoryServiceRecordResponse, MemoryServiceSearchRequest,
    MemoryServiceSearchResponse, MemoryServiceSearchResult, MemoryServiceTreeRequest,
    MemoryServiceTreeResponse, MemoryServiceWriteRequest, MemoryServiceWriteResponse,
    MemoryWriteStatus, memory_context_disabled,
};

const MEMORY_PATH: &str = "MEMORY.md";
const HEARTBEAT_PATH: &str = "HEARTBEAT.md";
const BOOTSTRAP_PATH: &str = "BOOTSTRAP.md";
const PROFILE_DOCUMENT_PATH: &str = "context/profile.json";
const MAX_MEMORY_PATCH_RETRIES: usize = 8;

pub struct NativeMemoryService {
    backend: Arc<dyn MemoryBackend>,
}

impl std::fmt::Debug for NativeMemoryService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeMemoryService")
            .field("backend", &"<native-memory-backend>")
            .finish()
    }
}

impl NativeMemoryService {
    pub fn new(backend: Arc<dyn MemoryBackend>) -> Self {
        Self { backend }
    }

    pub fn from_filesystem(
        filesystem: Arc<dyn RootFilesystem>,
        prompt_write_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    ) -> Self {
        Self {
            backend: build_native_backend(filesystem, prompt_write_safety_event_sink),
        }
    }

    fn scoped_context(
        &self,
        invocation: &MemoryInvocation,
    ) -> Result<(MemoryDocumentScope, MemoryContext), MemoryServiceError> {
        let scope = MemoryDocumentScope::new_with_agent(
            invocation.scope.tenant_id.as_str(),
            invocation.scope.user_id.as_str(),
            invocation.scope.agent_id.as_ref().map(|id| id.as_str()),
            invocation.scope.project_id.as_ref().map(|id| id.as_str()),
        )
        .map_err(|_| MemoryServiceError::input())?;
        let context = MemoryContext::new(scope.clone())
            .with_audit_context(invocation.scope.clone(), invocation.correlation_id);
        Ok((scope, context))
    }
}

#[async_trait]
impl MemoryService for NativeMemoryService {
    async fn search(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceSearchRequest,
    ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
        let (_, context) = self.scoped_context(&invocation)?;
        let search_request = MemorySearchRequest::new(&request.query)
            .map_err(|_| MemoryServiceError::input())?
            .with_limit(request.limit)
            .with_pre_fusion_limit(request.limit.max(20))
            .with_vector(false);
        let results = self
            .backend
            .search(&context, search_request)
            .await
            .map_err(MemoryServiceError::operation_from)?
            .into_iter()
            .map(|result| MemoryServiceSearchResult {
                is_hybrid_match: result.is_hybrid(),
                content: result.snippet,
                score: result.score,
                path: result.path.relative_path().to_string(),
            })
            .collect();
        Ok(MemoryServiceSearchResponse {
            query: request.query,
            results,
        })
    }

    async fn write(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceWriteRequest,
    ) -> Result<MemoryServiceWriteResponse, MemoryServiceError> {
        reject_local_or_traversal_path(&request.target)?;
        let (scope, context) = self.scoped_context(&invocation)?;
        let resolved_path = resolve_target_path(&request.target, request.timezone.as_deref())?;
        // The `threads/` namespace is reserved for per-thread short-term scratch
        // written ONLY by the trusted after-turn recorder via `record_interaction`
        // (which routes through `write_reserved_document`, bypassing this guard). A
        // tool- or caller-authored `threads/...` document would be excluded from the
        // long-term lane AND unreachable from every short-term lane but its own
        // active thread — a silent retrieval black hole. Fail loud instead of
        // persisting it. (CR review / audit L1.)
        if is_thread_scoped_path(&resolved_path) {
            return Err(MemoryServiceError::operation());
        }
        let path = document_path(&scope, &resolved_path)?;
        let options = write_options(request.metadata.as_ref());

        if request.target == "bootstrap" {
            if path.relative_path() != BOOTSTRAP_PATH || resolved_path != BOOTSTRAP_PATH {
                return Err(MemoryServiceError::operation());
            }
            let context = context.clone().with_prompt_write_safety_allowance(
                PromptSafetyAllowanceId::empty_prompt_file_clear(),
            );
            self.backend
                .write_document_with_backend_options(&context, &path, b"", &options)
                .await
                .map_err(MemoryServiceError::operation_from)?;
            return Ok(MemoryServiceWriteResponse {
                status: MemoryWriteStatus::Cleared,
                path: resolved_path.clone(),
                append: false,
                content_length: 0,
                replacements: None,
                message: Some("BOOTSTRAP.md cleared.".to_string()),
            });
        }

        if let Some(old_string) = request.old_string.as_deref() {
            if old_string.is_empty() {
                return Err(MemoryServiceError::input());
            }
            let new_string = request
                .new_string
                .as_deref()
                .ok_or_else(MemoryServiceError::input)?;
            // Origin's `required_str(new_string)` rejected empty replacements;
            // preserve that — an empty `new_string` must not delete matched text.
            if new_string.is_empty() {
                return Err(MemoryServiceError::input());
            }
            return self
                .patch_document(PatchDocumentRequest {
                    context: &context,
                    path: &path,
                    resolved_path: &resolved_path,
                    options: &options,
                    old_string,
                    new_string,
                    replace_all: request.replace_all,
                })
                .await;
        }

        if request.content.trim().is_empty() {
            return Err(MemoryServiceError::input());
        }
        if request.append {
            self.backend
                .append_document_with_backend_options(
                    &context,
                    &path,
                    request.content.as_bytes(),
                    &options,
                )
                .await
                .map_err(MemoryServiceError::operation_from)?;
        } else {
            self.backend
                .write_document_with_backend_options(
                    &context,
                    &path,
                    request.content.as_bytes(),
                    &options,
                )
                .await
                .map_err(MemoryServiceError::operation_from)?;
        }

        Ok(MemoryServiceWriteResponse {
            status: MemoryWriteStatus::Written,
            path: resolved_path,
            append: request.append,
            content_length: request.content.len(),
            replacements: None,
            message: None,
        })
    }

    async fn read(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceReadRequest,
    ) -> Result<MemoryServiceReadResponse, MemoryServiceError> {
        reject_local_or_traversal_path(&request.path)?;
        let (scope, context) = self.scoped_context(&invocation)?;
        let path = document_path(&scope, &request.path)?;
        let Some(bytes) = self
            .backend
            .read_document(&context, &path)
            .await
            .map_err(MemoryServiceError::operation_from)?
        else {
            return Err(MemoryServiceError::input());
        };
        let content = String::from_utf8(bytes).map_err(MemoryServiceError::operation_from)?;
        Ok(MemoryServiceReadResponse {
            path: path.relative_path().to_string(),
            word_count: content.split_whitespace().count(),
            content,
        })
    }

    async fn tree(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceTreeRequest,
    ) -> Result<MemoryServiceTreeResponse, MemoryServiceError> {
        if !request.path.is_empty() {
            reject_local_or_traversal_path(&request.path)?;
        }
        let (scope, context) = self.scoped_context(&invocation)?;
        let mut paths = self
            .backend
            .list_documents(&context, &scope)
            .await
            .map_err(MemoryServiceError::operation_from)?
            .into_iter()
            .map(|path| path.relative_path().to_string())
            .collect::<Vec<_>>();
        paths.sort();
        Ok(MemoryServiceTreeResponse {
            entries: tree_for_paths(&paths, request.path.trim_matches('/'), request.depth),
        })
    }

    async fn profile_set(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceProfileSetRequest,
    ) -> Result<MemoryServiceProfileSetResponse, MemoryServiceError> {
        let (scope, path) = profile_scope_and_path(
            invocation.scope.tenant_id.as_str(),
            invocation.scope.user_id.as_str(),
        )?;
        let context = MemoryContext::new(scope)
            .with_audit_context(invocation.scope.clone(), invocation.correlation_id);
        let options = write_options(None);
        for _ in 0..MAX_MEMORY_PATCH_RETRIES {
            let current = self
                .backend
                .read_document(&context, &path)
                .await
                .map_err(MemoryServiceError::operation_from)?;
            let expected_hash = current.as_deref().map(content_bytes_sha256);
            let mut doc: Map<String, Value> = match &current {
                Some(bytes) => {
                    serde_json::from_slice(bytes).map_err(MemoryServiceError::operation_from)?
                }
                None => Map::new(),
            };
            for key in ["timezone", "locale", "location"] {
                if let Some(value) = doc.get(key)
                    && !value.is_string()
                {
                    return Err(MemoryServiceError::operation());
                }
            }
            for (key, value) in &request.fields {
                doc.insert(key.clone(), value.clone());
            }
            let bytes = serde_json::to_vec(&Value::Object(doc))
                .map_err(MemoryServiceError::operation_from)?;
            let outcome = self
                .backend
                .compare_and_write_document_with_backend_options(
                    &context,
                    &path,
                    expected_hash.as_deref(),
                    &bytes,
                    &options,
                )
                .await
                .map_err(MemoryServiceError::operation_from)?;
            if outcome == MemoryWriteOutcome::Written {
                return Ok(MemoryServiceProfileSetResponse {
                    status: MemoryProfileSetStatus::Ok,
                });
            }
        }
        Err(MemoryServiceError::operation())
    }

    async fn profile_read(
        &self,
        invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        // Single home for the profile scope/path decision, shared with
        // `profile_set`: keyed to the human user at `agent=None, project=None`.
        let (scope, path) = profile_scope_and_path(
            invocation.scope.tenant_id.as_str(),
            invocation.scope.user_id.as_str(),
        )?;
        let context = MemoryContext::new(scope);
        let document = self
            .backend
            .read_document(&context, &path)
            .await
            .map_err(MemoryServiceError::operation_from)?;
        Ok(MemoryServiceProfileReadResponse { document })
    }

    async fn retrieve_context(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        if request.max_snippets == 0 || memory_context_disabled(request.context_profile_id.as_str())
        {
            return Ok(Vec::new());
        }
        let (_, context) = self.scoped_context(&invocation)?;
        // Over-fetch BEFORE the lane filter below. `backend.search` caps results to
        // the search limit, so capping at `max_snippets` up front would let general
        // (long-term) hits in the global top-N starve the thread-scoped
        // (short-term) lane — a short-term call could come back short or empty
        // under normal ranking pressure. Fetch a wider candidate set, apply the
        // scope + lane retains, THEN truncate to `max_snippets` so each lane keeps
        // its own top results. (CR review: filter before limiting the short-term lane.)
        let fetch_limit = request.max_snippets.saturating_mul(8).max(64);
        let search_request = MemorySearchRequest::new(&request.query)
            .map_err(|_| MemoryServiceError::input())?
            .with_limit(fetch_limit)
            .with_pre_fusion_limit(fetch_limit.max(20))
            // Full-text only: the native backend declares vector_search=false and
            // fails closed on a vector request (matches the `search` method).
            //
            // Regression-audit note: origin's prompt-context search left
            // `vector=true`. `false` is intentional and correct for this provider —
            // the native backend is FTS-only (no embeddings wired), so a vector
            // request would fail closed and return nothing. A future
            // vector-capable provider would set this in its own `retrieve_context`.
            .with_vector(false);
        let mut results = self
            .backend
            .search(&context, search_request)
            .await
            .map_err(MemoryServiceError::unavailable_from)?;
        results.retain(|result| result.path.scope() == context.scope() && result.score.is_finite());
        // Thread-aware lane selection. The `thread_id` is supplied by the trusted
        // host run context on the invocation scope, never by the model.
        match invocation.scope.thread_id.as_ref() {
            // Short-term ("run-local") lane: restrict to the active thread's
            // memory subtree.
            Some(thread_id) => {
                let prefix = thread_memory_prefix(thread_id);
                results
                    .retain(|result| path_has_thread_prefix(result.path.relative_path(), &prefix));
            }
            // Long-term lane: the user's general/durable memory — exclude every
            // per-thread short-term scratch subtree so the two lanes stay disjoint
            // when the host concatenates them into one memory block.
            None => {
                results.retain(|result| !is_thread_scoped_path(result.path.relative_path()));
            }
        }
        results.sort_by(compare_memory_search_results);
        // Truncate to the requested count AFTER the lane filter so the over-fetch
        // above never leaks extra candidates and each lane keeps its own top N.
        results.truncate(request.max_snippets);

        // Return raw, ranked, in-scope candidates. The host sanitizes the text,
        // wraps it in the untrusted-memory envelope, builds the `memory-snippet:*`
        // reference, and enforces the per-snippet + aggregate model-visible byte
        // budgets — see `ironclaw_host_runtime::memory_context`. This provider only
        // ranks and scopes; it never shapes model-visible content, so a provider
        // cannot bypass host prompt safety.
        Ok(results
            .into_iter()
            .map(map_search_result_to_snippet)
            .collect())
    }

    async fn record_interaction(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceRecordRequest,
    ) -> Result<MemoryServiceRecordResponse, MemoryServiceError> {
        // The native provider stores the FULL turn history verbatim. Short-term
        // memory is thread-scoped: with no active thread there is no
        // `threads/<thread_id>/` subtree to record under, so degrade to a no-op
        // (not an error) — the host's after-turn seam stays best-effort.
        let Some(thread_id) = invocation.scope.thread_id.clone() else {
            tracing::debug!("record_interaction skipped: no thread_id on invocation scope");
            return Ok(MemoryServiceRecordResponse { recorded: false });
        };
        // The per-run transcript file is named by `turn_run_id` (provenance). With
        // no run id there is no per-run doc to write, so degrade to a no-op.
        let Some(turn_run_id) = request.turn_run_id.as_deref() else {
            tracing::debug!("record_interaction skipped: no turn_run_id on request");
            return Ok(MemoryServiceRecordResponse { recorded: false });
        };
        if request.messages.is_empty() {
            return Ok(MemoryServiceRecordResponse { recorded: false });
        }
        // Write the full transcript to a PER-RUN file under the SAME `threads/<T>/`
        // convention `retrieve_context`'s short-term lane filters on (reusing
        // `thread_memory_prefix`). Using a per-run path
        // (`threads/<thread_id>/<turn_run_id>.md`) with `append: false` (overwrite)
        // makes the record idempotent: a scheduler re-run of an already-`Completed`
        // run overwrites the same file instead of duplicating the exchange into an
        // unbounded shared `log.md` (CR1). Route through the existing write flow,
        // which builds the `MemoryDocumentScope`/`MemoryContext` via `scoped_context`.
        let target = format!("{}{turn_run_id}.md", thread_memory_prefix(&thread_id));
        let content = format_interaction(&request.messages);
        // Route through the reserved-namespace writer: `record_interaction` is the
        // ONLY legitimate writer of `threads/<T>/...`, so it bypasses the public
        // `write` guard that rejects tool-authored writes to that namespace.
        self.write_reserved_document(&invocation, &target, &content)
            .await?;
        Ok(MemoryServiceRecordResponse { recorded: true })
    }
}

impl NativeMemoryService {
    /// Write `content` to the reserved `threads/` namespace, bypassing the
    /// `write`-level reservation guard. ONLY the trusted per-run recorder
    /// ([`MemoryService::record_interaction`]) may write there; the public
    /// `write` rejects any `threads/`-prefixed target. Mirrors `write`'s
    /// plain-overwrite path (no append / patch / bootstrap special cases).
    async fn write_reserved_document(
        &self,
        invocation: &MemoryInvocation,
        target: &str,
        content: &str,
    ) -> Result<(), MemoryServiceError> {
        reject_local_or_traversal_path(target)?;
        if content.trim().is_empty() {
            return Err(MemoryServiceError::input());
        }
        let (scope, context) = self.scoped_context(invocation)?;
        let resolved_path = resolve_target_path(target, None)?;
        // Defense in depth: this bypass writes ONLY the reserved `threads/`
        // namespace. Reject anything else so a future caller cannot smuggle an
        // arbitrary path past the public `write` guard through this helper.
        if !is_thread_scoped_path(&resolved_path) {
            return Err(MemoryServiceError::operation());
        }
        let path = document_path(&scope, &resolved_path)?;
        let options = write_options(None);
        self.backend
            .write_document_with_backend_options(&context, &path, content.as_bytes(), &options)
            .await
            .map_err(MemoryServiceError::operation_from)?;
        Ok(())
    }

    async fn patch_document(
        &self,
        request: PatchDocumentRequest<'_>,
    ) -> Result<MemoryServiceWriteResponse, MemoryServiceError> {
        for _ in 0..MAX_MEMORY_PATCH_RETRIES {
            let Some(bytes) = self
                .backend
                .read_document(request.context, request.path)
                .await
                .map_err(MemoryServiceError::operation_from)?
            else {
                return Err(MemoryServiceError::operation());
            };
            let existing = String::from_utf8(bytes).map_err(MemoryServiceError::operation_from)?;
            let expected = content_bytes_sha256(existing.as_bytes());
            let replacements = existing.matches(request.old_string).count();
            if replacements == 0 {
                return Err(MemoryServiceError::input());
            }
            let replacement_count = if request.replace_all { replacements } else { 1 };
            let updated = if request.replace_all {
                existing.replace(request.old_string, request.new_string)
            } else {
                existing.replacen(request.old_string, request.new_string, 1)
            };
            let outcome = self
                .backend
                .compare_and_write_document_with_backend_options(
                    request.context,
                    request.path,
                    Some(&expected),
                    updated.as_bytes(),
                    request.options,
                )
                .await
                .map_err(MemoryServiceError::operation_from)?;
            if outcome == MemoryWriteOutcome::Written {
                return Ok(MemoryServiceWriteResponse {
                    status: MemoryWriteStatus::Patched,
                    path: request.resolved_path.to_string(),
                    append: false,
                    content_length: updated.len(),
                    replacements: Some(replacement_count),
                    message: None,
                });
            }
        }
        Err(MemoryServiceError::operation())
    }
}

struct PatchDocumentRequest<'a> {
    context: &'a MemoryContext,
    path: &'a MemoryDocumentPath,
    resolved_path: &'a str,
    options: &'a MemoryBackendWriteOptions,
    old_string: &'a str,
    new_string: &'a str,
    replace_all: bool,
}

fn build_native_backend(
    filesystem: Arc<dyn RootFilesystem>,
    prompt_write_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
) -> Arc<dyn MemoryBackend> {
    let repository = Arc::new(FilesystemMemoryDocumentRepository::new(filesystem));
    let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(Arc::clone(&repository)));
    let mut backend = RepositoryMemoryBackend::new(Arc::clone(&repository))
        .with_indexer(indexer)
        .with_capabilities(
            MemoryBackendCapabilities::default()
                .set_file_documents(true)
                .set_metadata(true)
                .set_versioning(true)
                .set_prompt_write_safety(true)
                .set_full_text_search(true)
                .set_delete(true)
                .set_transactions(true),
        );
    if let Some(prompt_write_safety_event_sink) = prompt_write_safety_event_sink {
        backend = backend.with_prompt_write_safety_event_sink(prompt_write_safety_event_sink);
    }
    Arc::new(backend)
}

fn resolve_target_path(target: &str, timezone: Option<&str>) -> Result<String, MemoryServiceError> {
    match target {
        "memory" => Ok(MEMORY_PATH.to_string()),
        "heartbeat" => Ok(HEARTBEAT_PATH.to_string()),
        "bootstrap" => Ok(BOOTSTRAP_PATH.to_string()),
        "daily_log" => {
            let timezone = match timezone {
                Some(value) => value
                    .parse::<Tz>()
                    .map_err(|_| MemoryServiceError::input())?,
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
) -> Result<MemoryDocumentPath, MemoryServiceError> {
    MemoryDocumentPath::new_with_agent(
        scope.tenant_id(),
        scope.user_id(),
        scope.agent_id(),
        scope.project_id(),
        relative_path,
    )
    .map_err(|_| MemoryServiceError::input())
}

fn profile_scope_and_path(
    tenant_id: &str,
    user_id: &str,
) -> Result<(MemoryDocumentScope, MemoryDocumentPath), MemoryServiceError> {
    let scope = MemoryDocumentScope::new_with_agent(tenant_id, user_id, None, None)
        .map_err(|_| MemoryServiceError::input())?;
    let path =
        MemoryDocumentPath::new_with_agent(tenant_id, user_id, None, None, PROFILE_DOCUMENT_PATH)
            .map_err(|_| MemoryServiceError::input())?;
    Ok((scope, path))
}

fn write_options(metadata_overlay: Option<&DocumentMetadata>) -> MemoryBackendWriteOptions {
    // Service writes are direct backend callers: leave
    // `prompt_safety_already_enforced` at its fail-closed default (false) so the
    // backend runs prompt-write safety itself.
    MemoryBackendWriteOptions::with_metadata_overlay(metadata_overlay.cloned())
}

fn reject_local_or_traversal_path(path: &str) -> Result<(), MemoryServiceError> {
    if path.contains('\\') || looks_like_filesystem_path(path) || contains_traversal(path) {
        return Err(MemoryServiceError::input());
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

fn tree_for_paths(paths: &[String], root: &str, max_depth: usize) -> Vec<Value> {
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{}/", root.trim_matches('/'))
    };
    let mut children = BTreeMap::<String, Vec<String>>::new();
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

/// Top-level virtual-path namespace reserved for per-thread short-term
/// ("run-local") memory. Documents under `threads/<thread_id>/` belong to the
/// short-term lane: included by thread-scoped retrieval, excluded from long-term
/// (general) retrieval. Reserved — general user memory does not use this prefix.
///
/// Enforced reservation (audit L1): a document under `threads/foo.md` is excluded
/// from the long-term lane AND matched by no short-term lane unless `foo` is the
/// active thread, so a stray write there is a silent retrieval "black hole". The
/// public [`MemoryService::write`] rejects any `threads/`-prefixed target; only the
/// trusted after-turn recorder writes there, via `write_reserved_document`.
const THREAD_MEMORY_ROOT: &str = "threads/";

/// Virtual-path prefix under which a specific thread's short-term memory lives.
/// Short-term retrieval (an invocation scope carrying a `thread_id`) restricts to
/// this prefix; the `thread_id` arrives on the trusted `MemoryInvocation` scope
/// from the host run context, never from the model.
fn thread_memory_prefix(thread_id: &ThreadId) -> String {
    format!("{THREAD_MEMORY_ROOT}{}/", thread_id.as_str())
}

/// Whether a relative memory path is per-thread short-term scratch (and so is
/// excluded from the long-term lane).
fn is_thread_scoped_path(relative_path: &str) -> bool {
    strip_thread_memory_root(relative_path).is_some()
}

fn path_has_thread_prefix(relative_path: &str, prefix: &str) -> bool {
    let Some(relative_tail) = strip_thread_memory_root(relative_path) else {
        return false;
    };
    let Some(prefix_tail) = prefix.strip_prefix(THREAD_MEMORY_ROOT) else {
        return false;
    };
    relative_tail.starts_with(prefix_tail)
}

fn strip_thread_memory_root(relative_path: &str) -> Option<&str> {
    let root = relative_path.get(..THREAD_MEMORY_ROOT.len())?;
    root.eq_ignore_ascii_case(THREAD_MEMORY_ROOT)
        .then(|| relative_path.get(THREAD_MEMORY_ROOT.len()..))
        .flatten()
}

fn compare_memory_search_results(
    left: &MemorySearchResult,
    right: &MemorySearchResult,
) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.path.relative_path().cmp(right.path.relative_path()))
}

/// Render an interaction exchange into the per-run thread transcript body. Each
/// message becomes a `## {role}` heading (with the actor `name` in parentheses
/// when present, e.g. `## user (alice)`) followed by its content, so the per-run
/// file reads as a simple Markdown transcript.
fn format_interaction(messages: &[MemoryInteractionMessage]) -> String {
    messages
        .iter()
        .map(|message| match message.name.as_deref() {
            Some(name) => format!(
                "## {} ({})\n{}\n",
                message.role.as_str(),
                name,
                message.content
            ),
            None => format!("## {}\n{}\n", message.role.as_str(), message.content),
        })
        .collect()
}

fn map_search_result_to_snippet(result: MemorySearchResult) -> MemoryServiceContextSnippet {
    // Carry raw scope/path components + raw snippet text. The host
    // (`ironclaw_host_runtime::memory_context`) owns reference hashing,
    // sanitization, untrusted-envelope wrapping, and the model-visible budgets.
    MemoryServiceContextSnippet {
        tenant_id: result.path.tenant_id().to_string(),
        user_id: result.path.user_id().to_string(),
        agent_id: result.path.agent_id().map(ToString::to_string),
        project_id: result.path.project_id().map(ToString::to_string),
        relative_path: result.path.relative_path().to_string(),
        text: result.snippet,
    }
}
