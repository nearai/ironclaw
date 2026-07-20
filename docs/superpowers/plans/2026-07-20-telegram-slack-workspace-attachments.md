# Telegram and Slack Workspace Attachments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Telegram and Slack inbound files land through the existing WebUI workspace attachment pipeline and deliver referenced workspace files back as native channel attachments.

**Architecture:** `DefaultInboundTurnService` gains a provider-neutral descriptor materialization port that feeds the existing `InboundAttachmentLander`. Shared channel delivery resolves assistant `/workspace/...` references through a scoped reader and passes transient bytes to a defaulted `ProductAdapter` attachment-render method; only native Telegram and Slack adapters override the method.

**Tech Stack:** Rust, async-trait, Reborn product workflow, `RootFilesystem`/`ScopedFilesystem`, mediated HTTP egress, Telegram Bot API, Slack Web API, Axum integration harnesses.

## Global Constraints

- Preserve the WebUI contract: 10 files maximum, 5 MiB per file, 10 MiB total.
- Store inbound bytes only through `InboundAttachmentLander` under `/workspace/attachments/...`.
- Do not persist raw attachment bytes, provider URLs, upload tickets, tokens, or host paths in DTOs, events, projections, logs, or delivery records.
- Do not submit an attachment-less turn when descriptor fetch, validation, or landing fails.
- Resolve outbound paths only through project-scoped filesystem authority.
- Mark an outbound attempt delivered only after every required text/file operation succeeds; partial visible delivery is permanent.
- Keep text-only behavior and all existing adapters backward compatible.
- Use `files.getUploadURLExternal` plus `files.completeUploadExternal`; do not use retired `files.upload`.
- Production code contains no `.unwrap()` or `.expect()` and clippy must have zero warnings.

---

### Task 1: Shared attachment budgets and workspace-reference extraction

**Files:**
- Create: `crates/ironclaw_attachments/src/budgets.rs`
- Create: `crates/ironclaw_attachments/src/workspace_refs.rs`
- Modify: `crates/ironclaw_attachments/src/lib.rs`
- Modify: `crates/ironclaw_product_workflow/src/webui_inbound.rs`
- Test: `crates/ironclaw_attachments/src/budgets.rs`
- Test: `crates/ironclaw_attachments/src/workspace_refs.rs`
- Test: `crates/ironclaw_product_workflow/tests/webui_inbound_contract.rs`

**Interfaces:**
- Produces: `AttachmentBudgets`, `DEFAULT_ATTACHMENT_BUDGETS`, `validate_attachment_metadata`, and `extract_workspace_attachment_paths(&str) -> Vec<String>`.
- Consumes: `ironclaw_common::{is_supported_mime, normalize_mime_type}`.

- [ ] **Step 1: Write failing budget and extractor tests**

```rust
#[test]
fn default_budgets_match_webui_contract() {
    assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_count, 10);
    assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes, 5 * 1024 * 1024);
    assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes, 10 * 1024 * 1024);
}

#[test]
fn workspace_refs_ignore_code_and_deduplicate() {
    let text = "Use /workspace/report.pdf, not `/workspace/secret.pdf`.\n```\n/workspace/code.csv\n```\n/workspace/report.pdf /workspace/chart.png";
    assert_eq!(
        extract_workspace_attachment_paths(text),
        vec!["/workspace/report.pdf", "/workspace/chart.png"]
    );
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p ironclaw_attachments workspace_refs -- --nocapture`

Expected: compile failure because the new modules and functions do not exist.

- [ ] **Step 3: Implement the shared value contract and extractor**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentBudgets {
    pub max_count: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
}

pub const DEFAULT_ATTACHMENT_BUDGETS: AttachmentBudgets = AttachmentBudgets {
    max_count: 10,
    max_file_bytes: 5 * 1024 * 1024,
    max_total_bytes: 10 * 1024 * 1024,
};
```

Implement a single-pass Markdown code-span masker, recognize absolute
`/workspace/` tokens, trim sentence punctuation, reject `..` and unsupported
extensions/MIME guesses, and deduplicate with first-seen ordering. Replace the
private WebUI constants with `DEFAULT_ATTACHMENT_BUDGETS`.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p ironclaw_attachments && cargo test -p ironclaw_product_workflow --test webui_inbound_contract`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_attachments crates/ironclaw_product_workflow/src/webui_inbound.rs crates/ironclaw_product_workflow/tests/webui_inbound_contract.rs
git commit -m "feat: share attachment limits and workspace refs"
```

### Task 2: Descriptor materialization in the canonical inbound workflow

**Files:**
- Modify: `crates/ironclaw_product_workflow/src/inbound_turn.rs`
- Modify: `crates/ironclaw_product_workflow/src/lib.rs`
- Modify: `crates/ironclaw_product_workflow/src/fakes.rs`
- Test: `crates/ironclaw_product_workflow/tests/inbound_turn_contract.rs`

**Interfaces:**
- Produces: `InboundAttachmentMaterializer`, `AttachmentMaterializationError`, `AttachmentMaterializationFailureKind`, and `DefaultInboundTurnService::with_inbound_attachment_materializer`.
- Consumes: `ProductInboundEnvelope`, `ProductAttachmentDescriptor`, and `InboundAttachment`.

- [ ] **Step 1: Add red caller-level tests**

```rust
#[tokio::test]
async fn descriptor_attachments_materialize_then_land_before_acceptance() {
    // Arrange one descriptor, a recording materializer, and recording lander.
    // Assert materialize == 1, land == 1, and accepted content has one AttachmentRef.
}

#[tokio::test]
async fn duplicate_descriptor_event_does_not_materialize_again() {
    // Submit the same external_event_id twice and assert one materialize/land call.
}

#[tokio::test]
async fn descriptor_fetch_failure_never_accepts_attachmentless_message() {
    // Materializer returns Permanent; assert no land, accept, or turn submit.
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `cargo test -p ironclaw_product_workflow --test inbound_turn_contract descriptor_ -- --nocapture`

Expected: compile failure because the materializer contract is absent.

- [ ] **Step 3: Add the port and workflow ordering**

```rust
#[async_trait]
pub trait InboundAttachmentMaterializer: Send + Sync {
    async fn materialize(
        &self,
        envelope: &ProductInboundEnvelope,
        descriptors: &[ProductAttachmentDescriptor],
    ) -> Result<Vec<InboundAttachment>, AttachmentMaterializationError>;
}
```

Store the optional port on `DefaultInboundTurnService`. In
`accept_with_before_policy_inner`, preserve the existing order: prepare -> replay
-> policy -> materialize -> land/accept. Reject mixed inline bytes plus
descriptors and descriptors without a configured materializer. Map `Retryable`
to `ProductWorkflowError::Transient` and `Permanent` to a sanitized
`TurnSubmissionRejected`.

- [ ] **Step 4: Run workflow tests**

Run: `cargo test -p ironclaw_product_workflow`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_product_workflow
git commit -m "feat: materialize channel attachments in workflow"
```

### Task 3: Transient outbound attachment contract and bounded protocol egress

**Files:**
- Modify: `crates/ironclaw_product_adapters/src/adapter.rs`
- Modify: `crates/ironclaw_product_adapters/src/egress.rs`
- Modify: `crates/ironclaw_product_adapters/src/lib.rs`
- Modify: `crates/ironclaw_product_workflow/src/outbound_delivery.rs`
- Test: `crates/ironclaw_product_adapters/tests/product_adapter_contract.rs`
- Test: `crates/ironclaw_product_workflow/tests/outbound_delivery_contract.rs`

**Interfaces:**
- Produces: non-serializable `ProductOutboundAttachment`, default
  `ProductAdapter::render_outbound_with_attachments`,
  `EgressRequest::with_response_body_limit`, and
  `ProductOutboundDeliveryRequest.attachments`.
- Consumes: existing `ProductOutboundEnvelope`, `ProtocolHttpEgress`, and
  `OutboundDeliverySink`.

- [ ] **Step 1: Write red compatibility and fail-closed tests**

```rust
#[tokio::test]
async fn default_attachment_renderer_delegates_empty_and_rejects_nonempty() {
    // Empty list reaches render_outbound; one file returns InvalidPayload.
}

#[test]
fn egress_request_carries_bounded_response_limit() {
    let request = request().with_response_body_limit(Some(5 * 1024 * 1024));
    assert_eq!(request.response_body_limit(), Some(5 * 1024 * 1024));
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_product_adapters --test product_adapter_contract attachment_renderer`

Expected: compile failure on missing contract.

- [ ] **Step 3: Implement transient values and orchestration**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductOutboundAttachment {
    pub workspace_path: String,
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
```

Do not derive `Serialize`/`Deserialize`. Add the default trait method and pass
attachments from `prepare_and_render_product_outbound` to it. Existing callers
provide `Vec::new()`. Add an optional requested response limit to
`EgressRequest`; each host egress implementation must clamp it to the shared
attachment maximum rather than trusting an adapter-supplied value.

- [ ] **Step 4: Run adapter/workflow tests**

Run: `cargo test -p ironclaw_product_adapters && cargo test -p ironclaw_product_workflow --test outbound_delivery_contract`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_product_adapters crates/ironclaw_product_workflow
git commit -m "feat: add transient outbound attachment rendering"
```

### Task 4: Scoped outbound workspace-file resolution in shared channel delivery

**Files:**
- Modify: `crates/ironclaw_channel_host/src/delivery_protocol.rs`
- Create: `crates/ironclaw_channel_delivery/src/attachments.rs`
- Modify: `crates/ironclaw_channel_delivery/src/lib.rs`
- Modify: `crates/ironclaw_channel_delivery/src/services.rs`
- Modify: `crates/ironclaw_channel_delivery/src/observer.rs`
- Modify: `crates/ironclaw_channel_delivery/src/triggered.rs`
- Test: `crates/ironclaw_channel_delivery/src/tests.rs`
- Modify: `crates/ironclaw_reborn_composition/src/support/fs/project_filesystem_reader.rs`

**Interfaces:**
- Produces: `OutboundWorkspaceAttachmentReader::read_workspace_attachment`,
  optional `FinalReplyDeliveryServices.workspace_attachments`, and
  `resolve_final_reply_attachments`.
- Consumes: `ThreadScope`, shared path extractor/budgets, and
  `ProductOutboundAttachment`.

- [ ] **Step 1: Write red shared delivery tests**

```rust
#[tokio::test]
async fn final_reply_workspace_ref_is_read_and_given_to_adapter() {
    // Final text includes /workspace/report.pdf; assert reader scope/path and bytes.
}

#[tokio::test]
async fn missing_workspace_ref_prevents_text_only_delivery() {
    // Reader returns NotFound; assert adapter and egress were not called.
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_channel_delivery workspace_ref -- --nocapture`

Expected: compile failure on missing reader/service field.

- [ ] **Step 3: Implement bounded resolution and wire both drivers**

```rust
#[async_trait]
pub trait OutboundWorkspaceAttachmentReader: Send + Sync {
    async fn read_workspace_attachment(
        &self,
        scope: &ThreadScope,
        workspace_path: &str,
    ) -> Result<ProductOutboundAttachment, FinalReplyDeliveryError>;
}
```

Resolve only `FinalReply` payloads, before outbound policy/rendering. Enforce
count/per-file/total limits again after reads. Pass the same attachment vector
through live observer and triggered driver. Leave prompt/status payloads empty.
Implement the reader over the existing project-scoped mount resolver.

- [ ] **Step 4: Run channel-delivery and architecture tests**

Run: `cargo test -p ironclaw_channel_delivery && cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_channel_host crates/ironclaw_channel_delivery crates/ironclaw_reborn_composition/src/support/fs
git commit -m "feat: resolve outbound workspace attachments"
```

### Task 5: Telegram inbound materialization

**Files:**
- Create: `crates/ironclaw_telegram_extension/src/attachments/inbound.rs`
- Create: `crates/ironclaw_telegram_extension/src/attachments/mod.rs`
- Modify: `crates/ironclaw_telegram_extension/src/lib.rs`
- Modify: `crates/ironclaw_telegram_extension/src/egress.rs`
- Modify: `crates/ironclaw_telegram_extension/src/host/revision.rs`
- Test: `crates/ironclaw_telegram_extension/src/attachments/inbound.rs`

**Interfaces:**
- Produces: `TelegramInboundAttachmentMaterializer` implementing the Task 2 port.
- Consumes: `TelegramProtocolHttpEgress`, `getFile`, descriptor metadata, and shared budgets.

- [ ] **Step 1: Write red provider-seam tests**

```rust
#[tokio::test]
async fn telegram_document_get_file_downloads_bounded_bytes() {
    // Fake mediated egress returns getFile then bytes; assert path, MIME, name, bytes.
}

#[tokio::test]
async fn telegram_declared_oversize_makes_no_request() {
    // 5 MiB + 1 descriptor; assert zero egress calls and Permanent error.
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_telegram_extension attachments::inbound -- --nocapture`

Expected: compile failure because the materializer is absent.

- [ ] **Step 3: Implement `getFile` plus bounded `/file/` transfer**

Validate `file_path` as a relative Telegram path without traversal/control
characters. Extend Telegram host egress URL construction to distinguish Bot API
methods from `/file/` downloads while retaining token placeholder substitution.
Request at most 5 MiB + 1 byte and reject overflow. Wire the materializer and
existing project lander into every revision workflow.

- [ ] **Step 4: Run Telegram tests**

Run: `cargo test -p ironclaw_telegram_extension`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_telegram_extension
git commit -m "feat: land Telegram attachments in workspace"
```

### Task 6: Telegram native outbound attachments

**Files:**
- Create: `crates/ironclaw_telegram_v2_adapter/src/multipart.rs`
- Modify: `crates/ironclaw_telegram_v2_adapter/src/adapter.rs`
- Modify: `crates/ironclaw_telegram_v2_adapter/src/lib.rs`
- Modify: `crates/ironclaw_telegram_extension/src/egress.rs`
- Test: `crates/ironclaw_telegram_v2_adapter/src/adapter.rs`

**Interfaces:**
- Produces: Telegram override of `render_outbound_with_attachments`.
- Consumes: `ProductOutboundAttachment`, `sendPhoto`, `sendDocument`, current target parser, and delivery sink.

- [ ] **Step 1: Write red multipart and honesty tests**

```rust
#[tokio::test]
async fn workspace_png_renders_send_photo_multipart_then_delivered() {
    // Assert multipart body contains chat id, filename, content-type, exact bytes.
}

#[tokio::test]
async fn second_attachment_failure_after_first_send_is_permanent() {
    // First 2xx, second 500; assert FailedPermanent and no Delivered.
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_telegram_v2_adapter outbound_attachment -- --nocapture`

Expected: default renderer rejects non-empty attachments.

- [ ] **Step 3: Implement native aggregate sequence**

Create a random multipart boundary without user-controlled bytes; escape quoted
filenames and reject CR/LF. Send text chunks followed by ordered attachments,
using `sendPhoto` only for supported image MIME types and `sendDocument`
otherwise. Reuse existing response classification; after any successful visible
part, map later failures to `FailedPermanent`.

- [ ] **Step 4: Run Telegram adapter/extension tests**

Run: `cargo test -p ironclaw_telegram_v2_adapter && cargo test -p ironclaw_telegram_extension`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_telegram_v2_adapter crates/ironclaw_telegram_extension/src/egress.rs
git commit -m "feat: send workspace files through Telegram"
```

### Task 7: Slack inbound materialization

**Files:**
- Create: `crates/ironclaw_reborn_composition/src/slack/slack_attachments.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/mod.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_egress.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs`
- Test: `crates/ironclaw_reborn_composition/src/slack/slack_serve/e2e_tests.rs`

**Interfaces:**
- Produces: `SlackInboundAttachmentMaterializer` implementing the Task 2 port.
- Consumes: `files.info`, validated `files.slack.com` download URLs, mediated bearer egress, and shared budgets.

- [ ] **Step 1: Write red Slack fetch tests**

```rust
#[tokio::test]
async fn slack_file_id_is_resolved_then_downloaded_with_host_auth() {
    // Capture files.info and files.slack.com calls; assert exact InboundAttachment.
}

#[tokio::test]
async fn slack_download_url_on_foreign_host_fails_before_download() {
    // files.info returns https://evil.example/x; assert Permanent and one API call.
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_reborn_composition --lib slack_attachment -- --nocapture`

Expected: compile failure because the materializer is absent.

- [ ] **Step 3: Implement lookup, validation, download, and wiring**

Add `files.slack.com` to the explicit egress policy for authenticated downloads
and unauthenticated upload tickets. Parse URLs with `url`, require HTTPS and the
exact host, preserve only origin-form path/query, and bound responses at 5 MiB +
1 byte. Compare returned id/name/MIME/size to the descriptor and fail on
mismatch. Wire the materializer and shared lander into Slack's inbound service.

- [ ] **Step 4: Run Slack composition tests**

Run: `cargo test -p ironclaw_reborn_composition --features test-support,libsql --lib slack`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn_composition/src/slack
git commit -m "feat: land Slack attachments in workspace"
```

### Task 8: Slack native outbound attachments and scopes

**Files:**
- Create: `crates/ironclaw_slack_v2_adapter/src/files.rs`
- Modify: `crates/ironclaw_slack_v2_adapter/src/adapter.rs`
- Modify: `crates/ironclaw_slack_v2_adapter/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_egress.rs`
- Modify: `docs/reborn/setup-slack-for-reborn-binary.md`
- Test: `crates/ironclaw_slack_v2_adapter/src/adapter.rs`
- Test: `crates/ironclaw_reborn_composition/src/slack/slack_serve/e2e_tests.rs`

**Interfaces:**
- Produces: Slack override of `render_outbound_with_attachments` and additive `files:write` setup scope.
- Consumes: Task 3 transient files, upload ticket egress, channel/topic target metadata, and delivery sink.

- [ ] **Step 1: Write red three-stage upload tests**

```rust
#[tokio::test]
async fn slack_attachment_gets_ticket_uploads_bytes_and_completes_with_reply() {
    // Assert getUploadURLExternal -> files.slack.com POST -> completeUploadExternal.
}

#[tokio::test]
async fn slack_partial_upload_is_failed_permanent_without_completion_retry() {
    // First file uploaded, second fails; assert FailedPermanent and no Delivered.
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p ironclaw_slack_v2_adapter outbound_attachment -- --nocapture`

Expected: default renderer rejects non-empty attachments.

- [ ] **Step 3: Implement Slack upload V2 and scope projection**

For each file call `files.getUploadURLExternal` with filename and exact length,
validate the returned HTTPS `files.slack.com` URL, and POST raw bytes. Finalize
the collected ids once with `files.completeUploadExternal`, `channel_id`,
optional `thread_ts`, and final text as `initial_comment`. Record one aggregate
status and treat any failure after a successful byte upload as permanent. Add
`files:write` everywhere setup manifests and copy declare Slack bot scopes.

- [ ] **Step 4: Run Slack adapter/composition tests**

Run: `cargo test -p ironclaw_slack_v2_adapter && cargo test -p ironclaw_reborn_composition --features test-support,libsql --lib slack`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_slack_v2_adapter crates/ironclaw_reborn_composition/src/slack crates/ironclaw_first_party_extensions docs
git commit -m "feat: send workspace files through Slack"
```

### Task 9: Whole-journey integration coverage

**Files:**
- Create: `tests/integration/telegram_journeys/scenario_attachments.rs`
- Modify: `tests/integration/telegram_journeys/main.rs`
- Modify: `tests/integration/telegram_journeys/harness.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_serve/e2e_tests.rs`
- Modify: `tests/integration/attach.rs`

**Interfaces:**
- Consumes: the production Telegram and Slack host builders, real workflow/lander/readers, and hermetic provider doubles.
- Produces: caller-level proof of inbound/outbound parity, replay, failure honesty, durability, and isolation.

- [ ] **Step 1: Add Telegram red journey cases**

Add provider-capture cases for photo/document/attachment-only/multiple, replay,
oversize/fetch failure, outbound image/document, missing outbound path, restart,
and cross-project denial. Assertions must inspect the workspace bytes, durable
message refs, captured model request, provider calls, and delivery status.

- [ ] **Step 2: Add Slack red journey cases**

Drive the production Slack webhook/host seam with `files.info`, private download,
upload ticket, upload, and completion doubles. Assert the same storage/model and
delivery properties as Telegram.

- [ ] **Step 3: Run the journeys and verify they fail before final wiring**

Run: `cargo test --test reborn_integration_telegram_journey attachments -- --nocapture`

Run: `cargo test -p ironclaw_reborn_composition --features test-support,libsql slack_attachment -- --nocapture`

Expected: failures identify any missing production reader/materializer wiring.

- [ ] **Step 4: Complete only the production wiring exposed by failures**

Wire `ProjectScopedAttachmentLander`, the channel materializers, and the
project-scoped outbound reader into `TelegramRevisionWorkflowBuilder` and Slack
host assembly. Do not add test-only bypasses.

- [ ] **Step 5: Run shared and channel integration suites**

Run: `cargo test --test reborn_integration_attach`

Run: `cargo test --test reborn_integration_telegram_journey`

Run: `cargo test -p ironclaw_reborn_composition --features test-support,libsql slack`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tests/integration crates/ironclaw_telegram_extension crates/ironclaw_reborn_composition/src/slack
git commit -m "test: cover channel attachment journeys"
```

### Task 10: Contracts, parity, verification, and PR

**Files:**
- Modify: `docs/reborn/contracts/telegram-v2.md`
- Modify: `docs/reborn/setup-slack-for-reborn-binary.md`
- Modify: `FEATURE_PARITY.md`
- Modify: `CHANGELOG.md`
- Review: every file changed on the branch

**Interfaces:**
- Consumes: all prior task behavior and tests.
- Produces: documented shipped contract and a reviewable PR.

- [ ] **Step 1: Update contracts and parity rows**

Document the shared landing path, path-triggered native outbound behavior,
limits, Slack scopes, Telegram/Slack provider flows, honest failure behavior,
and exact integration commands. Add a concise unreleased changelog entry.

- [ ] **Step 2: Run formatting and focused clippy**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy -p ironclaw_attachments -p ironclaw_product_adapters -p ironclaw_product_workflow -p ironclaw_channel_host -p ironclaw_channel_delivery -p ironclaw_telegram_v2_adapter -p ironclaw_telegram_extension -p ironclaw_slack_v2_adapter -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings`

Expected: PASS with zero warnings.

- [ ] **Step 3: Run architecture and whole-path checks**

Run: `cargo test -p ironclaw_architecture`

Run: `bash scripts/reborn-e2e-rust.sh`

Run: `scripts/pre-commit-safety.sh`

Expected: PASS.

- [ ] **Step 4: Audit the final diff**

Run: `git diff --check $(git merge-base HEAD origin/main)..HEAD`

Run: `git diff --stat $(git merge-base HEAD origin/main)..HEAD`

Run: `rg -n '\.unwrap\(|\.expect\(|/tmp/|source_url|url_private_download' $(git diff --name-only $(git merge-base HEAD origin/main)..HEAD -- 'crates/**/*.rs')`

Expected: no production unwrap/expect, hardcoded temp paths, persisted provider
URLs, or whitespace errors; intentional provider response parsing is reviewed in
place.

- [ ] **Step 5: Commit documentation**

```bash
git add docs FEATURE_PARITY.md CHANGELOG.md
git commit -m "docs: document channel attachment parity"
```

- [ ] **Step 6: Push and open the PR**

```bash
git push -u origin codex/telegram-slack-attachments
gh pr create --base main --head codex/telegram-slack-attachments \
  --title "Add Telegram and Slack workspace attachments" \
  --body "Adds shared inbound workspace landing and native outbound workspace-file delivery for Telegram and Slack. Includes provider-seam and whole-journey coverage, Slack scope compatibility, rollback notes, and the exact verification results below."
```

The PR body must list the inbound/outbound user journeys, the shared architecture,
Slack scope compatibility note, rollback behavior, exact tests run, and any suite
not run with a reason.
