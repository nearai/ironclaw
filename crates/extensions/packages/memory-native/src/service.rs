//! IronClaw memory service service for Reborn.
//!
//! This module owns the host-facing IronClaw memory operation shapes. Host
//! runtime callers still resolve scope, mounts, grants, approvals, and audit
//! services before calling the service; the default native adapter keeps the
//! existing storage format.

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc};

use crate::{
    ChunkingMemoryDocumentIndexer, FilesystemMemoryDocumentRepository, MemoryAppendOutcome,
    MemoryBackend, MemoryBackendCapabilities, MemoryBackendWriteOptions, MemorySearchRequest,
    MemorySearchResult, MemoryWriteOutcome, RepositoryMemoryBackend,
};
use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::ids::ThreadId;
use ironclaw_memory::{
    DocumentMetadata, MemoryContext, MemoryDocumentPath, MemoryDocumentScope,
    PromptSafetyAllowanceId, PromptWriteSafetyEventSink, content_bytes_sha256,
};
use ironclaw_memory::{
    MemoryInteractionMessage, MemoryInvocation, MemoryProfileSetStatus, MemoryService,
    MemoryServiceContextRequest, MemoryServiceContextSnippet, MemoryServiceError,
    MemoryServiceProfileReadResponse, MemoryServiceProfileSetRequest,
    MemoryServiceProfileSetResponse, MemoryServiceReadRequest, MemoryServiceReadResponse,
    MemoryServiceRecordRequest, MemoryServiceRecordResponse, MemoryServiceSearchRequest,
    MemoryServiceSearchResponse, MemoryServiceSearchResult, MemoryServiceTreeRequest,
    MemoryServiceTreeResponse, MemoryServiceWriteRequest, MemoryServiceWriteResponse,
    MemoryWriteStatus, memory_context_disabled,
};
use serde_json::{Map, Value, json};

// The host-facing operation shapes and the `MemoryService` trait are owned by
// `ironclaw_memory`; consumers import them from there. The imports above are
// private to this module — #6943 deleted the re-export shim that used to
// republish them under `ironclaw_memory_native::` — and this module exports
// only `NativeMemoryService`, the native adapter implemented below, plus this
// package's own memory-guidance asset.

/// The `[memory].guidance_doc` ref this package's manifest declares, paired
/// with the text it names.
///
/// Exported rather than reached into: the host resolves a bound provider's
/// declared guidance ref through the owning package's public API, so no host
/// crate compiles this package's asset tree into its own
/// (`reborn_cross_crate_include_scan` §11.2.7 is shrink-only). Keeping the ref
/// and the text together here is also what makes the manifest and the asset
/// impossible to drift apart — a rename that touches one and not the other
/// fails `native_bundle_declares_guidance_that_resolves_to_the_bundled_asset`.
pub const MEMORY_GUIDANCE_DOC_REF: &str = "prompts/memory-guidance.md";

/// Model-facing memory guidance this provider ships: when a durable fact is
/// worth saving, how to phrase it, and what never to save. Appended to the
/// system prompt by composition while this provider is the bound one (#7185).
pub const MEMORY_GUIDANCE: &str = include_str!("../prompts/memory-guidance.md");

/// The `[memory].scheduled_ops` pass prompt ref this package's manifest
/// declares, paired with the text it names.
///
/// The instruction for this provider's own upkeep pass: re-read the standing
/// memory document, merge what says the same thing, resolve what has been
/// superseded, and report. It describes THIS provider's document shape and
/// names THIS provider's tools, so it ships beside the manifest that declares
/// the pass rather than in the product tier that runs it — the same reason
/// [`MEMORY_GUIDANCE_DOC_REF`] lives here.
pub const MEMORY_CURATION_PASS_PROMPT_REF: &str = "prompts/memory_curation.md";

/// Text named by [`MEMORY_CURATION_PASS_PROMPT_REF`].
pub const MEMORY_CURATION_PASS_PROMPT: &str = include_str!("../prompts/memory_curation.md");

/// This package's `[memory]` asset table: every asset ref its own manifest may
/// declare — the `guidance_doc`, and each `scheduled_ops` pass prompt — paired
/// with the text it names. The host resolves a bound provider's declared refs
/// against exactly this table — generically, not by naming this package's
/// constants in a host-side match — so a manifest ref this table does not
/// carry is a manifest/asset desync, not a silently dropped asset.
pub const MEMORY_ASSETS: &[(&str, &str)] = &[
    (MEMORY_GUIDANCE_DOC_REF, MEMORY_GUIDANCE),
    (MEMORY_CURATION_PASS_PROMPT_REF, MEMORY_CURATION_PASS_PROMPT),
];

const MEMORY_PATH: &str = "MEMORY.md";
const HEARTBEAT_PATH: &str = "HEARTBEAT.md";
const BOOTSTRAP_PATH: &str = "BOOTSTRAP.md";
const PROFILE_DOCUMENT_PATH: &str = "context/profile.json";
const MAX_MEMORY_PATCH_RETRIES: usize = 8;

/// Snippets of the standing [`MEMORY_PATH`] document this provider prepends to
/// its own long-term lane, ahead of the full-text hits (#7185).
///
/// The lane's job is "the user's general, durable memory", and full-text search
/// can only answer that when the current message happens to share vocabulary
/// with the stored fact — open a new chat on an unrelated subject and a saved
/// preference is invisible. The curated document is what the write guidance
/// tells the model to maintain, so this provider serves it unconditionally as
/// part of the same lane rather than making the host know about document paths.
/// Capped so the standing document cannot consume the caller's whole
/// `max_snippets` allowance and starve the search hits behind it.
const MAX_CURATED_SNIPPETS: usize = 4;

/// Raw bytes per curated chunk, before the host sanitizes and wraps it.
///
/// The host caps a model-visible snippet at 512 bytes and runs the prompt
/// denylist there, so a whole standing document admitted as one wide snippet
/// would be denylist-checked only at its head. Splitting into chunks this size
/// leaves room for the untrusted envelope inside that cap, so every byte the
/// model sees passes the same check a search hit passes — and a line carrying a
/// denylisted secret is dropped on its own instead of taking the document with
/// it.
const CURATED_CHUNK_RAW_BYTES: usize = 400;

/// Appended to the last admitted curated chunk when the standing document did
/// not fit [`MAX_CURATED_SNIPPETS`], so the model can tell a clipped document
/// from a complete one. Bracket and delimiter characters are rejected by the
/// host's prompt safe-summary rule, so the marker is plain words.
const CURATED_TRUNCATION_MARKER: &str = " (truncated)";

/// Joins the lines inside one curated chunk. A raw newline is a control
/// character and is stripped when the host sanitizes the snippet, which would
/// run two facts together into one, so lines are joined with a visible
/// separator instead.
const CURATED_LINE_SEPARATOR: &str = "; ";

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

/// The provider's conventional tool operations — INHERENT methods, not part
/// of any memory contract. The model reaches them only through the tools the
/// manifest declares, served by this provider's first-party capability
/// handler.
impl NativeMemoryService {
    pub async fn search(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceSearchRequest,
    ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
        let (_, context) = self.scoped_context(&invocation)?;
        let search_request = MemorySearchRequest::new(&request.query)
            .map_err(MemoryServiceError::input_from)?
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

    pub async fn write(
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
            let outcome = match request.expected_content_hash.as_deref() {
                Some(expected) => self
                    .backend
                    .compare_and_write_document_with_backend_options(
                        &context,
                        &path,
                        Some(expected),
                        b"",
                        &options,
                    )
                    .await
                    .map_err(MemoryServiceError::operation_from)?,
                None => {
                    self.backend
                        .write_document_with_backend_options(&context, &path, b"", &options)
                        .await
                        .map_err(MemoryServiceError::operation_from)?;
                    MemoryWriteOutcome::Written
                }
            };
            if outcome == MemoryWriteOutcome::Conflict {
                return Ok(write_conflict_response(&resolved_path));
            }
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
                    expected_content_hash: request.expected_content_hash.as_deref(),
                })
                .await;
        }

        if request.content.trim().is_empty() {
            return Err(MemoryServiceError::input());
        }
        let written_length = if request.append {
            // Appended entries are one self-contained line each — that is what
            // the memory protocol tells the model to write, and what the
            // curated standing-document lane assumes when it splits `MEMORY.md`
            // on line boundaries. The backend append is byte-exact, so without
            // this two correct guided saves ("likes tea", then "lives in
            // Berlin") persist as the single run-on line `likes tealives in
            // Berlin` and are surfaced to a later turn as one fact. Terminate
            // every appended entry with exactly one newline.
            let entry = format!("{}\n", request.content.trim_end());
            if let Some(expected) = request.expected_content_hash.as_deref() {
                let outcome = self
                    .backend
                    .compare_and_append_document_with_backend_options(
                        &context,
                        &path,
                        Some(expected),
                        entry.as_bytes(),
                        &options,
                    )
                    .await
                    .map_err(MemoryServiceError::operation_from)?;
                if outcome == MemoryAppendOutcome::Conflict {
                    return Ok(write_conflict_response(&resolved_path));
                }
            } else {
                self.backend
                    .append_document_with_backend_options(
                        &context,
                        &path,
                        entry.as_bytes(),
                        &options,
                    )
                    .await
                    .map_err(MemoryServiceError::operation_from)?;
            }
            entry.len()
        } else if let Some(expected) = request.expected_content_hash.as_deref() {
            let outcome = self
                .backend
                .compare_and_write_document_with_backend_options(
                    &context,
                    &path,
                    Some(expected),
                    request.content.as_bytes(),
                    &options,
                )
                .await
                .map_err(MemoryServiceError::operation_from)?;
            if outcome == MemoryWriteOutcome::Conflict {
                return Ok(write_conflict_response(&resolved_path));
            }
            request.content.len()
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
            request.content.len()
        };

        Ok(MemoryServiceWriteResponse {
            status: MemoryWriteStatus::Written,
            path: resolved_path,
            append: request.append,
            content_length: written_length,
            replacements: None,
            message: None,
        })
    }

    pub async fn read(
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
            content_hash: content_bytes_sha256(content.as_bytes()),
            content,
        })
    }

    pub async fn tree(
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

    pub async fn profile_set(
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
}

#[async_trait]
impl MemoryService for NativeMemoryService {
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

    /// Long-term lane: the standing memory document first, then the full-text
    /// hits for this turn's query.
    ///
    /// The curated prefix is query-independent by design — that is the whole
    /// point. Search can only surface a saved fact when the current message
    /// shares words with it, so a preference stated in one conversation is
    /// invisible in the next one that opens on an unrelated subject. Serving
    /// the document the write guidance tells the model to maintain closes that
    /// gap inside the lane the host already asks for, with no new host call and
    /// no lane-specific provider contract.
    async fn read_long_term(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        // Checked here as well as inside `ranked_in_scope_results`, because the
        // curated read must not happen either: a memory-disabled context
        // profile means no memory reaches the prompt, not "no search results".
        if request.max_snippets == 0 || memory_context_disabled(request.context_profile_id.as_str())
        {
            return Ok(Vec::new());
        }
        let mut snippets = self
            .curated_standing_snippets(&invocation, request.max_snippets)
            .await;
        let remaining = request.max_snippets.saturating_sub(snippets.len());
        if remaining == 0 {
            return Ok(snippets);
        }
        let Some(mut results) = self.ranked_in_scope_results(&invocation, &request).await? else {
            return Ok(snippets);
        };
        // Exclude every per-thread short-term scratch subtree, regardless of
        // whether the invocation carries an active thread, so the two lanes
        // stay disjoint when the host concatenates them into one memory block.
        //
        // `MEMORY_PATH` is excluded for a different reason: it is already at the
        // head of this lane. A query whose words happen to match the standing
        // document would otherwise spend a second snippet slot re-admitting
        // what the model can already see, displacing a different document that
        // matched.
        results.retain(|result| {
            let path = result.path.relative_path();
            !is_thread_scoped_path(path) && path != MEMORY_PATH
        });
        snippets.extend(rank_and_truncate(results, remaining));
        Ok(snippets)
    }

    async fn read_short_term(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        // Short-term ("run-local") lane: restrict to the active thread's memory
        // subtree. The `thread_id` is supplied by the trusted host run context
        // on the invocation scope, never by the model; with no active thread
        // there is nothing to retrieve, so degrade to empty.
        let Some(thread_id) = invocation.scope.thread_id.clone() else {
            tracing::debug!("read_short_term skipped: no thread_id on invocation scope");
            return Ok(Vec::new());
        };
        let Some(mut results) = self.ranked_in_scope_results(&invocation, &request).await? else {
            return Ok(Vec::new());
        };
        let prefix = thread_memory_prefix(&thread_id);
        results.retain(|result| path_has_thread_prefix(result.path.relative_path(), &prefix));
        Ok(rank_and_truncate(results, request.max_snippets))
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
        // convention the `read_short_term` lane filters on (reusing
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
    /// Shared body of the two retrieval lanes ([`MemoryService::read_long_term`]
    /// / [`MemoryService::read_short_term`]): guard the disabled/zero cases,
    /// over-fetch a wide candidate set, and drop cross-scope + non-finite
    /// results. Returns `None` when retrieval is disabled for this request.
    ///
    /// Over-fetch BEFORE the caller's lane filter. `backend.search` caps results
    /// to the search limit, so capping at `max_snippets` up front would let
    /// general (long-term) hits in the global top-N starve the thread-scoped
    /// (short-term) lane — a short-term call could come back short or empty
    /// under normal ranking pressure. Fetch a wider candidate set, apply the
    /// scope + lane retains, THEN truncate to `max_snippets` (in
    /// `rank_and_truncate`) so each lane keeps its own top results.
    async fn ranked_in_scope_results(
        &self,
        invocation: &MemoryInvocation,
        request: &MemoryServiceContextRequest,
    ) -> Result<Option<Vec<MemorySearchResult>>, MemoryServiceError> {
        if request.max_snippets == 0 || memory_context_disabled(request.context_profile_id.as_str())
        {
            return Ok(None);
        }
        let (_, context) = self.scoped_context(invocation)?;
        let fetch_limit = request.max_snippets.saturating_mul(8).max(64);
        let search_request = MemorySearchRequest::new(&request.query)
            .map_err(MemoryServiceError::input_from)?
            .with_limit(fetch_limit)
            .with_pre_fusion_limit(fetch_limit.max(20))
            // Full-text only: the native backend declares vector_search=false and
            // fails closed on a vector request (matches the `search` method).
            // FTS-only is intentional and correct for this provider — no
            // embeddings are wired, so a vector request would fail closed and
            // return nothing. A vector-capable provider sets this in its own
            // lane methods.
            .with_vector(false);
        let mut results = self
            .backend
            .search(&context, search_request)
            .await
            .map_err(MemoryServiceError::unavailable_from)?;
        results.retain(|result| result.path.scope() == context.scope() && result.score.is_finite());
        Ok(Some(results))
    }

    /// The standing [`MEMORY_PATH`] document, split into snippet-sized chunks
    /// for the head of the long-term lane. Empty when there is nothing saved.
    ///
    /// Absence is the NORMAL state — a user who has never saved anything — and
    /// [`NativeMemoryService::read`] reports it as an `Input` error, so this
    /// degrades to no curated prefix rather than failing the lane. A backend
    /// fault degrades the same way, with a `debug!`: memory is best-effort
    /// context and must never take a turn down with it.
    async fn curated_standing_snippets(
        &self,
        invocation: &MemoryInvocation,
        max_snippets: usize,
    ) -> Vec<MemoryServiceContextSnippet> {
        let budget = max_snippets.min(MAX_CURATED_SNIPPETS);
        if budget == 0 {
            return Vec::new();
        }
        let (scope, context) = match self.scoped_context(invocation) {
            Ok(resolved) => resolved,
            Err(_) => return Vec::new(),
        };
        let path = match document_path(&scope, MEMORY_PATH) {
            Ok(path) => path,
            Err(_) => return Vec::new(),
        };
        let document = match self.backend.read_document(&context, &path).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::debug!(
                    %error,
                    "standing memory document read failed; long-term lane degrades to search only"
                );
                return Vec::new();
            }
        };
        let Ok(text) = String::from_utf8(document) else {
            tracing::debug!("standing memory document is not valid UTF-8; skipping curated prefix");
            return Vec::new();
        };
        // Split to one past the budget: that extra chunk is what proves the
        // document did not fit, which is all the truncation marker needs. It
        // also bounds the WORK — the document is user-controlled and re-read on
        // every run, so an arbitrarily large one must not be fully chunked just
        // to be discarded.
        let mut chunks = split_curated_text(&text, CURATED_CHUNK_RAW_BYTES, budget + 1);
        let truncated = chunks.len() > budget;
        chunks.truncate(budget);
        if truncated && let Some(last) = chunks.last_mut() {
            mark_curated_truncation(last);
        }
        chunks
            .into_iter()
            .map(|text| MemoryServiceContextSnippet {
                tenant_id: scope.tenant_id().to_string(),
                user_id: scope.user_id().to_string(),
                agent_id: scope.agent_id().map(ToString::to_string),
                project_id: scope.project_id().map(ToString::to_string),
                relative_path: path.relative_path().to_string(),
                text,
            })
            .collect()
    }

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
            let current_hash = content_bytes_sha256(existing.as_bytes());
            if request
                .expected_content_hash
                .is_some_and(|expected| expected != current_hash)
            {
                return Ok(write_conflict_response(request.resolved_path));
            }
            let expected = current_hash;
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
    expected_content_hash: Option<&'a str>,
    old_string: &'a str,
    new_string: &'a str,
    replace_all: bool,
}

fn write_conflict_response(path: &str) -> MemoryServiceWriteResponse {
    MemoryServiceWriteResponse {
        status: MemoryWriteStatus::Conflict,
        path: path.to_string(),
        append: false,
        content_length: 0,
        replacements: None,
        message: Some(
            "Document changed since it was read. Re-read it and re-apply the change.".to_string(),
        ),
    }
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

/// Resolve a model-facing native-memory target alias to its canonical relative
/// document path. The host handler uses the same resolver for pre-dispatch
/// outcomes so conflict and success responses cannot disagree on `path`.
pub fn resolve_target_path(
    target: &str,
    timezone: Option<&str>,
) -> Result<String, MemoryServiceError> {
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

/// Order the lane-filtered candidates (score descending, path ascending on
/// ties), truncate to the requested count AFTER the lane filter so the
/// over-fetch never leaks extra candidates, and map to raw snippets. The host
/// sanitizes the text, wraps it in the untrusted-memory envelope, builds the
/// `memory-snippet:*` reference, and enforces the per-snippet + aggregate
/// model-visible byte budgets — see `ironclaw_host_runtime::memory_context`.
/// This provider only ranks and scopes; it never shapes model-visible content,
/// so a provider cannot bypass host prompt safety.
fn rank_and_truncate(
    mut results: Vec<MemorySearchResult>,
    max_snippets: usize,
) -> Vec<MemoryServiceContextSnippet> {
    results.sort_by(compare_memory_search_results);
    results.truncate(max_snippets);
    results
        .into_iter()
        .map(map_search_result_to_snippet)
        .collect()
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

/// Split the standing memory document into at most `max_chunks` chunks of at
/// most `max_raw_bytes` each, preferring line boundaries so a one-fact-per-line
/// `MEMORY.md` is never cut mid-fact. Blank lines are dropped.
///
/// A single line longer than the byte budget is clipped and marked HERE rather
/// than left for the host to clip: the host sanitizes a curated chunk with the
/// same code path it uses for a search hit, which truncates silently, so an
/// over-long line would reach the model shortened with nothing saying so.
/// Clipping it at a char boundary first keeps every emitted chunk inside the
/// documented limit and keeps the marker.
///
/// `max_chunks` bounds the work, not just the result: the caller passes its
/// admission budget plus one, so an arbitrarily large user-controlled document
/// is never fully materialized just to be discarded.
fn split_curated_text(text: &str, max_raw_bytes: usize, max_chunks: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    if max_chunks == 0 {
        return chunks;
    }
    let mut current = String::new();
    for line in text.lines() {
        if chunks.len() >= max_chunks {
            return chunks;
        }
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        // Clipped before the packing logic below, so everything after this
        // point can assume a line fits a chunk on its own.
        let clipped_line;
        let line = if line.len() > max_raw_bytes {
            clipped_line = clip_and_mark(line, max_raw_bytes);
            clipped_line.as_str()
        } else {
            line
        };
        if !current.is_empty()
            && current.len() + CURATED_LINE_SEPARATOR.len() + line.len() > max_raw_bytes
        {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str(CURATED_LINE_SEPARATOR);
        }
        current.push_str(line);
        if current.len() >= max_raw_bytes {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() && chunks.len() < max_chunks {
        chunks.push(current);
    }
    chunks
}

/// Append the truncation marker to the last admitted chunk, making room for it
/// first so the chunk still fits [`CURATED_CHUNK_RAW_BYTES`]. Without the
/// reservation a full chunk would grow past the size the host's per-snippet cap
/// was sized for, and the sanitizer would clip the marker back off — leaving a
/// clipped document indistinguishable from a complete one.
fn mark_curated_truncation(chunk: &mut String) {
    let marked = clip_and_mark(chunk, CURATED_CHUNK_RAW_BYTES);
    *chunk = marked;
}

/// Clip `text` at a char boundary and append the truncation marker, so the
/// result is at most `max_raw_bytes`. Text already inside the budget is only
/// marked. Never slices mid-character.
fn clip_and_mark(text: &str, max_raw_bytes: usize) -> String {
    let room = max_raw_bytes.saturating_sub(CURATED_TRUNCATION_MARKER.len());
    let mut end = text.len().min(room);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut clipped = text[..end].to_string();
    clipped.push_str(CURATED_TRUNCATION_MARKER);
    clipped
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
