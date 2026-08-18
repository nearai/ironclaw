//! Reborn integration-test framework — storage-backend matrix.
//!
//! Covers: backend parity (one golden scenario through `StorageMode::InMemory`
//! and `StorageMode::LibSql`, asserting an identical outcome — the canonical
//! `rstest` matrix exemplar for this tier) and LibSql persistence correctness
//! (write-then-reopen through a fresh database handle, design §3.8 guardrail).
//!
//! Runs under default features, no services, no keys, no Docker, no
//! `integration` feature — libSQL is an embedded SQLite file in a `TempDir`
//! dropped at test end.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_filesystem::{Filter, Page, RootFilesystem};
use ironclaw_host_api::{
    ids::{CapabilityId, ProviderToolName},
    path::VirtualPath,
};
use ironclaw_threads::{
    AppendToolResultReferenceRequest, FinalizedAssistantMessageByRunRequest,
    ListThreadsForScopeRequest, ProviderToolCallReferenceEnvelope, SessionThreadService,
    ToolResultSafeSummary, UpdateToolResultReferenceRequest,
};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;

/// Backend-parity self-test (design §7): the same golden turn must produce the
/// same finalized reply on every storage backend. The canonical matrix
/// exemplar — add a backend by adding one `#[case]`.
#[rstest]
#[case(StorageMode::InMemory)]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn backend_parity_replies_to_greeting(#[case] storage: StorageMode) {
    let harness = RebornIntegrationHarness::test_default()
        .storage(storage)
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("hi there")
        .await
        .expect("turn completes");
    harness
        .assert_reply_contains("Hello! How can I help?")
        .await
        .expect("reply finalized in thread history");
}

/// Message hot-path parity: exact lookups resolve the same source message on
/// each production database backend without materializing sibling entry rows.
#[rstest]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn message_lookup_indexes_share_the_message_row(#[case] storage: StorageMode) {
    let harness = RebornIntegrationHarness::test_default()
        .storage(storage)
        .script([RebornScriptedReply::text("indexed reply")])
        .build()
        .await
        .expect("harness builds");
    let run_id = harness
        .submit_turn("first user message")
        .await
        .expect("turn completes");
    let service = harness
        .thread_service_for_test()
        .expect("thread service is available");
    let scope = harness.thread_harness.scope.clone();
    let thread_id = harness.binding.thread_id.clone();

    let assistant = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: run_id.to_string(),
        })
        .await
        .expect("assistant lookup by run succeeds")
        .expect("assistant lookup finds the finalized row");
    assert_eq!(assistant.content.as_deref(), Some("indexed reply"));

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("first-user lookup derives the title");
    assert_eq!(
        listed.threads[0].title.as_deref(),
        Some("first user message")
    );

    let generic = service
        .append_tool_result_reference(tool_result_request(
            &scope,
            &thread_id,
            run_id.to_string(),
            "result:generic",
            None,
        ))
        .await
        .expect("generic tool result is appended");
    let generic_replay = service
        .append_tool_result_reference(tool_result_request(
            &scope,
            &thread_id,
            run_id.to_string(),
            "result:generic",
            None,
        ))
        .await
        .expect("generic tool-result lookup deduplicates the replay");
    assert_eq!(generic_replay.message_id, generic.message_id);

    let provider_call = provider_call_reference("call-1");
    let provider = service
        .append_tool_result_reference(tool_result_request(
            &scope,
            &thread_id,
            run_id.to_string(),
            "result:provider",
            Some(provider_call.clone()),
        ))
        .await
        .expect("provider-call tool result is appended");
    let provider_replay = service
        .append_tool_result_reference(tool_result_request(
            &scope,
            &thread_id,
            run_id.to_string(),
            "result:provider",
            Some(provider_call),
        ))
        .await
        .expect("provider-call lookup deduplicates the replay");
    assert_eq!(provider_replay.message_id, provider.message_id);
    let updated = service
        .update_tool_result_reference(UpdateToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: run_id.to_string(),
            result_ref: "result:provider".to_string(),
            provider_call_id: Some("call-1".to_string()),
            safe_summary: ToolResultSafeSummary::new("provider result updated")
                .expect("valid summary"),
        })
        .await
        .expect("provider-call lookup targets the exact row");
    assert_eq!(updated.message_id, provider.message_id);

    let owner = scope
        .owner_user_id
        .as_ref()
        .expect("integration thread scope has an owner");
    let rows = harness
        ._shared
        .composite
        .query(
            &VirtualPath::new(format!(
                "/tenants/{}/users/{}/threads",
                scope.tenant_id.as_str(),
                owner.as_str()
            ))
            .expect("valid threads root"),
            &Filter::All,
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .expect("database entry rows can be inspected");
    assert!(
        rows.iter()
            .all(|row| !row.path.as_str().contains("/indexes/")),
        "message lookup must not write sibling entry rows"
    );
}

fn tool_result_request(
    scope: &ironclaw_threads::ThreadScope,
    thread_id: &ironclaw_host_api::ids::ThreadId,
    turn_run_id: String,
    result_ref: &str,
    provider_call: Option<ProviderToolCallReferenceEnvelope>,
) -> AppendToolResultReferenceRequest {
    AppendToolResultReferenceRequest {
        scope: scope.clone(),
        thread_id: thread_id.clone(),
        turn_run_id,
        result_ref: result_ref.to_string(),
        safe_summary: ToolResultSafeSummary::new("tool result").expect("valid summary"),
        provider_call,
        model_observation: None,
        intrinsic_outcome: None,
    }
}

fn provider_call_reference(call_id: &str) -> ProviderToolCallReferenceEnvelope {
    ProviderToolCallReferenceEnvelope {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        provider_turn_id: "provider-turn".to_string(),
        provider_call_id: call_id.to_string(),
        provider_tool_name: ProviderToolName::new("builtin__result_read")
            .expect("valid provider tool name"),
        capability_id: CapabilityId::new("builtin.result_read").expect("valid capability id"),
        arguments: serde_json::json!({"offset": 0}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

/// Persistence correctness (design §3.8): the reply must survive to the
/// SQLite file and read back through a fresh database handle, not an
/// in-process cache. InMemory cannot make this assertion (nothing reaches disk).
#[tokio::test]
async fn libsql_persists_reply_across_reopen() {
    let harness = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([RebornScriptedReply::text("durable answer")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("remember this")
        .await
        .expect("turn completes");
    harness
        .assert_reply_persists_after_reopen("durable answer")
        .await
        .expect("reply durable in reopened SQLite");
}

/// Guard: `assert_reply_persists_after_reopen` must return `Err` when the
/// expected text is absent, proving the reopen assertion isn't vacuously
/// green — it inspects real on-disk history.
#[tokio::test]
async fn persistence_assertion_fails_on_mismatch_after_reopen() {
    let harness = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([RebornScriptedReply::text("durable answer")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("remember this")
        .await
        .expect("turn completes");
    assert!(
        harness
            .assert_reply_persists_after_reopen("a reply that was never produced")
            .await
            .is_err(),
        "reopen assertion must fail when the expected text is absent from persisted history"
    );
}

/// Inventory gate for persistence backends (#6524 workstream 4: "apply the
/// same pattern to ... persistence backends and other closed product
/// surfaces").
///
/// The capability inventory works because the denominator comes from
/// production, not from a hand-kept list: adding a capability without
/// classifying it fails CI. Storage backends had the coverage but not the
/// gate — all three variants happen to be exercised today, and nothing would
/// have said so if a fourth arrived uncovered.
mod backend_inventory {
    /// Every backend the harness can run, read from the enum's own source.
    ///
    /// Deliberately not a hand-kept array. A list restating the enum can be
    /// left behind: a new `StorageMode` variant only has to satisfy the
    /// compiler, and extending a `|` pattern is easier to remember than an
    /// array three screens away. Parsing the declaration means the
    /// denominator cannot disagree with the type it describes.
    fn declared_storage_modes() -> Vec<String> {
        let source = include_str!("support/builder.rs");
        let body = source
            .split_once("pub enum StorageMode {")
            .expect("StorageMode is declared in support/builder.rs")
            .1
            .split_once("\n}")
            .expect("the enum declaration is brace-terminated")
            .0;
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("#["))
            // Take the variant NAME, whatever follows it. A payload-carrying
            // variant (`Custom(String),`) or a discriminant (`Legacy = 1,`)
            // must still be required to have a parity case -- dropping those
            // lines as unparseable would let exactly the backend this gate
            // exists to catch slip through unnamed.
            .map(|line| {
                line.split(['(', '{', ',', '='])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .filter(|name| {
                !name.is_empty()
                    && name.starts_with(|c: char| c.is_ascii_uppercase())
                    && name
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
            })
            .collect()
    }

    /// The attribute block attached to the parity test, and nothing else.
    ///
    /// Scoped to that one item on purpose: scanning the whole file would let
    /// an unrelated `#[case(StorageMode::Postgres)]` elsewhere stand in for
    /// the parity coverage this gate is about. Returning the block as one
    /// string also makes multiline attributes work without parsing them.
    fn parity_attribute_block() -> String {
        let source = include_str!("backend_matrix.rs");
        let lines: Vec<&str> = source.lines().collect();
        let signature = lines
            .iter()
            .position(|line| line.contains("async fn backend_parity_replies_to_greeting"))
            .expect("the parity matrix test is declared in this file");
        // Walk back to the blank line separating this item from the previous
        // one; everything between is this item's own doc and attributes.
        let mut first = signature;
        while first > 0 && !lines[first - 1].trim().is_empty() {
            first -= 1;
        }
        lines[first..signature].join("\n")
    }

    /// The attribute text alone, with doc prose removed.
    ///
    /// The block above deliberately includes doc comments, and a raw substring
    /// search over that blob would let a passing mention of a backend in prose
    /// -- a historical note, an explanation of why some case exists -- stand in
    /// for the real `#[case(...)]`. That is the same false positive this gate
    /// already rejects across tests, just reachable through prose.
    ///
    /// Bracket depth is tracked rather than matching line-by-line so an
    /// attribute split across lines is still read whole.
    fn parity_case_attributes() -> String {
        let mut attributes = String::new();
        let mut depth = 0usize;
        for line in parity_attribute_block().lines() {
            let trimmed = line.trim();
            if depth == 0 && !trimmed.starts_with("#[") {
                continue;
            }
            attributes.push_str(trimmed);
            attributes.push('\n');
            depth += trimmed.matches('[').count();
            depth = depth.saturating_sub(trimmed.matches(']').count());
        }
        attributes
    }

    /// Every declared backend is exercised by the parity matrix.
    #[test]
    fn every_storage_backend_is_exercised_by_the_parity_matrix() {
        let modes = declared_storage_modes();
        // Guard against a silent parse failure: an empty or truncated list
        // would make every assertion below vacuous.
        assert!(
            modes.len() >= 3,
            "parsed only {modes:?} from the StorageMode declaration; the \
             parser has drifted from the enum's shape and this gate would \
             pass vacuously"
        );
        for expected in ["InMemory", "LibSql", "Postgres"] {
            assert!(
                modes.iter().any(|mode| mode == expected),
                "the StorageMode parser stopped finding `{expected}`; it reads \
                 `pub enum StorageMode` from support/builder.rs -- check \
                 whether the declaration moved or changed shape. Until this is \
                 fixed the gate cannot see new backends either. Parsed: {modes:?}"
            );
        }

        let attributes = parity_case_attributes();
        assert!(
            attributes.contains("#[case"),
            "found no #[case] attributes on the parity matrix; the gate would \
             pass vacuously. Block was:\n{attributes}"
        );

        for mode in &modes {
            let needle = format!("StorageMode::{mode}");
            assert!(
                attributes.contains(&needle),
                "storage backend `{mode}` is declared in StorageMode but never \
                 exercised by the parity matrix; add `#[case({needle})]` to \
                 `backend_parity_replies_to_greeting`, or remove the variant. \
                 A backend with no case is the shape this gate exists to catch."
            );
        }
    }
}
