# Generic Cross-Channel Attachments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver one production attachment contract across WebUI, Telegram, Slack, and future channel adapters: inbound batches land atomically under shared host limits, the agent can explicitly attach workspace files to its reply, finalized assistant messages durably own attachment references, and adapters transfer exactly those referenced files.

**Architecture:** Extend the existing `RootFilesystem` fabric with a generic atomic subtree-creation operation, then use it from `ironclaw_attachments` to commit one message attachment directory per batch. Add a run-scoped, CAS-backed reply-attachment intent store in `ironclaw_outbound`; expose it through a mediated first-party capability, seal its refs into the finalized assistant `MessageContent`, and make delivery consume only those refs. Keep Slack and Telegram responsible for provider fetch/upload only, with restricted egress and host-injected credentials.

**Tech Stack:** Rust 2024, Tokio, `async_trait`, `serde`, `reqwest`, Axum, IronClaw Reborn crates, LibSQL, PostgreSQL, React/TypeScript, Node test runner, Cargo nextest-compatible tests.

## Global Constraints

- Preserve the approved product decisions: explicit outbound attachment intent, all-or-nothing inbound batches, and shared host limits that adapters may only narrow.
- Keep `RootFilesystem` as the only filesystem dispatch trait. Do not introduce an attachment-shaped storage trait.
- Treat every provider payload and remote file as untrusted until host validation completes.
- Do not persist transient provider download URLs, bearer tokens, upload URLs, or attachment bytes in outbound semantic state.
- Do not use `.unwrap()` or `.expect()` in production code.
- Keep every new operation bounded by `DEFAULT_ATTACHMENT_BUDGETS`.
- Preserve read compatibility for existing flat `/workspace/attachments/<date>/<filename>` paths.
- Run both LibSQL and PostgreSQL contract tests for persistence changes.
- Verify production composition, not only test-only assembly.
- Use `apply_patch` for source edits and commit after each green task.

---

## Task 1: Add a generic atomic subtree-creation filesystem contract

**Files:**

- Modify: `crates/ironclaw_filesystem/src/root.rs`
- Modify: `crates/ironclaw_filesystem/src/types.rs`
- Modify: `crates/ironclaw_filesystem/src/lib.rs`
- Modify: `crates/ironclaw_filesystem/src/scoped.rs`
- Modify: `crates/ironclaw_filesystem/src/catalog.rs`
- Modify: `crates/ironclaw_filesystem/src/fault.rs`
- Modify: `crates/ironclaw_filesystem/src/in_memory.rs`
- Modify: `crates/ironclaw_filesystem/src/local.rs`
- Modify: `crates/ironclaw_filesystem/src/libsql.rs`
- Modify: `crates/ironclaw_filesystem/src/postgres.rs`
- Test: `crates/ironclaw_filesystem/tests/filesystem_contract.rs`
- Test: `crates/ironclaw_filesystem/tests/db_root_filesystem_contract.rs`
- Test: `crates/ironclaw_filesystem/src/scoped/tests.rs`

### Contract

Add a neutral entry type:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct AtomicSubtreeEntry {
    pub path: VirtualPath,
    pub entry: Entry,
}
```

Add this operation to `RootFilesystem`:

```rust
async fn create_subtree_atomic(
    &self,
    prefix: &VirtualPath,
    entries: Vec<AtomicSubtreeEntry>,
) -> Result<Vec<RecordVersion>, FilesystemError>;
```

The contract is:

- `prefix` must not already exist;
- every entry path must be a strict descendant of `prefix`;
- paths must be unique;
- an empty batch is rejected;
- either all entries become visible or none do;
- returned versions preserve input order;
- a retry against an existing prefix fails without overwriting anything;
- a composite or scoped call must resolve to one writable mount before dispatch;
- failures use a new `FilesystemOperation::CreateSubtreeAtomic` audit/error witness.

### Steps

- [ ] Add backend-agnostic contract tests for empty batches, duplicate paths, out-of-prefix paths, existing-prefix conflicts, success ordering, and injected failure with zero visible entries.
- [ ] Run `cargo test -p ironclaw_filesystem create_subtree_atomic --all-features` and confirm the tests fail because the API is missing.
- [ ] Add `AtomicSubtreeEntry`, `FilesystemOperation::CreateSubtreeAtomic`, and the `RootFilesystem` method with a fail-closed default.
- [ ] Implement validation once in the filesystem crate and call it before any backend side effect.
- [ ] Implement `InMemoryFilesystem` by validating, taking one write lock, checking the prefix and all paths, then inserting the full batch before releasing the lock.
- [ ] Implement `DiskFilesystem` by creating a sibling staging directory on the same filesystem, materializing every entry inside it, syncing files and the directory as supported, then atomically renaming the staging directory to the final prefix. Remove only the operation-owned staging directory after failure.
- [ ] Implement LibSQL and PostgreSQL with one database transaction per batch. Map busy/conflict outcomes to errors that guarantee no partial commit.
- [ ] Implement `CompositeRootFilesystem` by resolving the prefix and every entry to the same mount and forwarding only after the complete validation passes.
- [ ] Implement `ScopedFilesystem::create_subtree_atomic` by checking write permission for the prefix and every scoped entry, resolving through the fixed mount view, and forwarding one root call.
- [ ] Update `FaultInjectingFilesystem` so a configured `CreateSubtreeAtomic` failure occurs before the delegate call and therefore cannot partially write.
- [ ] Run:

```bash
cargo test -p ironclaw_filesystem create_subtree_atomic --all-features
cargo test -p ironclaw_filesystem --test filesystem_contract --all-features
cargo test -p ironclaw_filesystem --test db_root_filesystem_contract --all-features
```

- [ ] Run `cargo clippy -p ironclaw_filesystem --all-targets --all-features -- -D warnings`.
- [ ] Commit:

```bash
git add crates/ironclaw_filesystem
git commit -m "feat(filesystem): add atomic subtree creation"
```

---

## Task 2: Make inbound attachment landing atomic and fix production mount permissions

**Files:**

- Modify: `crates/ironclaw_attachments/src/landing.rs`
- Modify: `crates/ironclaw_attachments/src/inbound.rs`
- Modify: `crates/ironclaw_attachments/src/lib.rs`
- Modify: `crates/ironclaw_product/src/scoped_fs/attachment_landing.rs`
- Modify: `crates/ironclaw_product/src/inbound_turn.rs`
- Modify: `crates/ironclaw_product/src/inbound_turn/tests/attachments.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory/tests.rs`
- Test: `crates/ironclaw_attachments/src/inbound.rs`
- Test: `crates/ironclaw_product/tests/inbound_turn_contract.rs`
- Test: `crates/ironclaw_reborn_composition/src/runtime/tests/core.rs`

### Contract

Land a message batch under:

```text
/workspace/attachments/<yyyy-mm-dd>/<sanitized-message-id>/<index>-<sanitized-filename>
```

Validation and document extraction happen before the atomic write. A later validation, extraction, or storage failure leaves the message prefix absent. Existing flat attachment references remain readable.

### Steps

- [ ] Replace `later_item_failure_fails_the_batch_and_leaves_earlier_bytes_landed` with a red contract asserting the earlier file is also absent.
- [ ] Add tests for same-name files, duplicate message delivery, empty filenames, path sanitization, maximum count, per-file limit, aggregate limit, and deterministic ordered refs.
- [ ] Run `cargo test -p ironclaw_attachments inbound::tests` and confirm the atomicity test fails against sequential writes.
- [ ] Change `land_inbound_attachments` to validate and prepare every `AtomicSubtreeEntry` in memory, then call one scoped atomic subtree operation.
- [ ] Return `AttachmentRef`s only after the batch commit succeeds.
- [ ] Add compensation in inbound product orchestration: if thread/message acceptance fails after landing, delete only the newly created message batch prefix and preserve unrelated batches.
- [ ] Add a bounded stale-batch cleanup pass that deletes only old message directories which have no durable thread attachment reference; make cleanup best-effort and observable, never a prerequisite for message admission.
- [ ] Change production `start_channel_host_assembly` wiring to give `ProjectScopedAttachmentLander` a read-write project workspace mount while keeping the reader and agent-visible read surfaces at their existing permissions.
- [ ] Add a production-assembly regression test that lands through `start_channel_host_assembly`, not the test-only read-write shortcut.
- [ ] Run:

```bash
cargo test -p ironclaw_attachments
cargo test -p ironclaw_product inbound_turn --all-features
cargo test -p ironclaw_reborn_composition attachment --all-features
```

- [ ] Run targeted clippy for the three changed crates.
- [ ] Commit:

```bash
git add crates/ironclaw_attachments crates/ironclaw_product crates/ironclaw_reborn_composition
git commit -m "fix(attachments): land inbound batches atomically"
```

---

## Task 3: Persist bounded run-scoped reply attachment intents

**Files:**

- Add: `crates/ironclaw_outbound/src/reply_attachment_intents.rs`
- Modify: `crates/ironclaw_outbound/src/outbound_state_store.rs`
- Modify: `crates/ironclaw_outbound/src/lib.rs`
- Modify: `crates/ironclaw_outbound/src/error.rs`
- Modify: `crates/ironclaw_outbound/src/types.rs`
- Test: `crates/ironclaw_outbound/tests/outbound_state_store_contract.rs`

### Contract

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyAttachmentIntent {
    pub path: ScopedPath,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[async_trait]
pub trait ReplyAttachmentIntentPort: Send + Sync {
    async fn register(
        &self,
        scope: &ResourceScope,
        run_id: &RunId,
        intent: ReplyAttachmentIntent,
    ) -> Result<(), OutboundError>;

    async fn seal(
        &self,
        scope: &ResourceScope,
        run_id: &RunId,
    ) -> Result<Vec<ReplyAttachmentIntent>, OutboundError>;
}
```

Persist one CAS-managed record per scoped run with ordered unique intents and a `sealed` flag. Registration after sealing fails. Repeated identical registration is idempotent. Conflicting metadata for the same path fails closed. Count and total bytes use `DEFAULT_ATTACHMENT_BUDGETS`.

### Steps

- [ ] Add failing in-memory and durable store contract tests for registration, ordering, idempotency, conflicts, limits, sealing, registration-after-seal, concurrent registration, and repeated sealing.
- [ ] Run `cargo test -p ironclaw_outbound reply_attachment` and confirm the API is missing.
- [ ] Add the public intent type and port.
- [ ] Add a versioned persisted envelope with `#[serde(deny_unknown_fields)]` where compatible with existing state conventions.
- [ ] Implement register/seal with the shared bounded `cas_update` helper; never hold a process-local lock across backend I/O.
- [ ] Ensure stored data contains only stable scoped paths and metadata, never bytes or provider URLs.
- [ ] Run `cargo test -p ironclaw_outbound reply_attachment --all-features`.
- [ ] Run `cargo clippy -p ironclaw_outbound --all-targets --all-features -- -D warnings`.
- [ ] Commit:

```bash
git add crates/ironclaw_outbound
git commit -m "feat(outbound): persist reply attachment intents"
```

---

## Task 4: Add the explicit mediated attachment capability

**Files:**

- Add: `crates/ironclaw_host_runtime/src/first_party_tools/reply_attachment.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs`
- Modify: `crates/ironclaw_host_runtime/src/lib.rs`
- Modify: `crates/ironclaw_host_runtime/Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Test: `crates/ironclaw_host_runtime/src/services/tests/first_party_runtime_adapter.rs`
- Test: `crates/ironclaw_reborn_composition/src/runtime/tests/outbound_delivery.rs`

### Contract

Register `builtin.attach_workspace_file_to_reply` with input:

```json
{
  "path": "/workspace/reports/result.csv",
  "filename": "result.csv",
  "mime_type": "text/csv"
}
```

`filename` and `mime_type` are optional overrides. The handler resolves the request’s fixed mount view, stats and bounded-reads the file, derives safe defaults, validates MIME and shared budgets, and registers metadata against the authenticated run. It never returns file bytes to the model and never performs provider delivery.

### Steps

- [ ] Add a red first-party runtime test proving the builtin is discoverable but fails closed before composition injects a reply-intent port.
- [ ] Add handler tests for missing run ID, missing mount view, path outside workspace, directory path, missing file, oversized file, unsafe filename, invalid MIME, success, duplicate success, and sealed-run failure.
- [ ] Run `cargo test -p ironclaw_host_runtime reply_attachment` and confirm the builtin is absent.
- [ ] Add the schema, manifest, default fail-closed handler, and production registration function following the existing outbound-delivery first-party pattern.
- [ ] Declare read-filesystem and outbound side-effect metadata so authorization, approval, obligations, and auditing remain active.
- [ ] Use `ScopedFilesystem::with_fixed_view`; do not mint a trusted request or bypass `InvocationServices`.
- [ ] Inject the same `Arc<dyn ReplyAttachmentIntentPort>` backed by `OutboundStateStore` from production composition.
- [ ] Add a composition test that invokes the real registry wrapper chain and reads the registered intent back.
- [ ] Run:

```bash
cargo test -p ironclaw_host_runtime reply_attachment --all-features
cargo test -p ironclaw_reborn_composition reply_attachment --all-features
cargo test -p ironclaw_architecture
```

- [ ] Run targeted clippy and commit:

```bash
git add crates/ironclaw_host_runtime crates/ironclaw_reborn_composition
git commit -m "feat(runtime): add explicit reply attachment capability"
```

---

## Task 5: Seal attachment refs into the finalized assistant message

**Files:**

- Modify: `crates/ironclaw_loop_host/src/lib.rs`
- Modify: `crates/ironclaw_loop_host/Cargo.toml`
- Modify: `crates/ironclaw_runner/src/loop_driver_host.rs`
- Modify: `crates/ironclaw_runner/Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Test: `crates/ironclaw_loop_host/tests/thread_loop_host_contract.rs`
- Test: `crates/ironclaw_runner/tests/loop_driver_host.rs`

### Contract

`ThreadBackedLoopTranscriptPort::finalize_assistant_message` seals the run’s reply intents before appending the final transcript entry, converts them to `AttachmentRef`s, and writes:

```rust
MessageContent::with_attachments(reply_text, attachment_refs)
```

Drafts remain text-only. Idempotent finalization compares text and attachment refs. The finalized message is the sole durable source for presentation and delivery.

### Steps

- [ ] Add a red transcript contract test that registers two intents and expects both ordered refs on the finalized assistant message.
- [ ] Add tests for no intents, seal failure, retry after a successful transcript write, retry with mismatched refs, and duplicate finalization.
- [ ] Run `cargo test -p ironclaw_loop_host finalized_assistant_attachment` and confirm refs are missing.
- [ ] Add an optional reply-attachment intent port to `ThreadBackedLoopTranscriptPort` constructors/builders without weakening existing call sites.
- [ ] Convert `ThreadScope` plus the canonical run ID to the outbound store key without re-deriving identity from display strings or transport metadata.
- [ ] Seal immediately before final append; preserve the sealed record for idempotent retries.
- [ ] Compare complete `MessageContent`, not only `content.as_deref()`, when recovering an already-finalized message.
- [ ] Wire the shared port through `ironclaw_runner` and production composition.
- [ ] Run:

```bash
cargo test -p ironclaw_loop_host finalized_assistant --all-features
cargo test -p ironclaw_runner reborn_driver --all-features
cargo test -p ironclaw_architecture
```

- [ ] Run targeted clippy and commit:

```bash
git add crates/ironclaw_loop_host crates/ironclaw_runner crates/ironclaw_reborn_composition
git commit -m "feat(loop): finalize replies with attachment refs"
```

---

## Task 6: Cut delivery and WebUI presentation over to durable attachment refs

**Files:**

- Modify: `crates/ironclaw_product/src/run_delivery/observer.rs`
- Modify: `crates/ironclaw_product/src/delivery_coordinator.rs`
- Modify: `crates/ironclaw_product/src/outbound_delivery.rs`
- Modify: `crates/ironclaw_product/tests/run_delivery_contract.rs`
- Modify: `crates/ironclaw_product/tests/outbound_delivery_contract.rs`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/components/message-bubble.tsx`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/components/project-file-chips.tsx`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/components/message-bubble.test.ts`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/lib/project-file-paths.ts`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/lib/project-file-paths.test.ts`
- Modify: `crates/ironclaw_webui/src/webui_v2/static_assets/assets.rs`

### Contract

The run-delivery observer loads the complete finalized assistant `MessageContent`. `CoordinatedDeliveryRequest` carries ordered durable `AttachmentRef`s. The coordinator materializes `OutboundPart::File` only from those refs after scoped stat/read and budget validation. Text containing `/workspace/...` never creates a file delivery.

WebUI renders attachment chips from message attachment refs. Legacy text-path parsing may remain only as an explicitly labeled display-only compatibility fallback for old history; it must not trigger download, delivery, or semantic state.

### Steps

- [ ] Add red delivery tests proving a finalized attachment ref produces a file part and prose-only `/workspace/report.csv` does not.
- [ ] Add tests for ref ordering, missing files, oversized files, aggregate limit, MIME preservation, filename sanitization, and caller-supplied bytes rejection.
- [ ] Run `cargo test -p ironclaw_product delivery_attachment --all-features` and confirm current prose scanning violates the negative test.
- [ ] Change the observer to load the final message record and pass its attachment refs into the coordinator.
- [ ] Replace `extract_workspace_attachment_paths` in materialization with ref-driven scoped reads.
- [ ] Keep bytes transient inside a single delivery attempt; persist only semantic refs and provider evidence.
- [ ] Update React props so `ProjectFileChips` accepts attachment refs and does not infer current-message attachments from text.
- [ ] Add frontend tests proving structured refs render and path-looking prose alone does not render a current-message file chip.
- [ ] Update static-asset contract assertions to reflect structured attachment handling.
- [ ] Run:

```bash
cargo test -p ironclaw_product delivery --all-features
cd crates/ironclaw_webui/frontend && npm test -- --run
cargo test -p ironclaw_webui static_assets --all-features
```

- [ ] Run targeted clippy and frontend typecheck/lint.
- [ ] Commit:

```bash
git add crates/ironclaw_product crates/ironclaw_webui
git commit -m "feat(product): deliver finalized attachment refs"
```

---

## Task 7: Implement Slack inbound file fetch behind restricted egress

**Files:**

- Modify: `crates/ironclaw_slack_extension/src/channel.rs`
- Modify: `crates/ironclaw_slack_extension/src/payload.rs`
- Modify: `crates/ironclaw_slack_extension/src/lib.rs`
- Modify: `crates/ironclaw_slack_extension/Cargo.toml`
- Modify: `crates/ironclaw_slack_extension/tests/channel_conformance.rs`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`
- Modify: `crates/ironclaw_reborn_composition/tests/first_party_manifest_v3_parity.rs`
- Modify: `docs/channels/slack.mdx`
- Modify: `docs/reborn/setup-slack-for-reborn-binary.md`

### Contract

The parser stores only stable Slack file IDs plus safe descriptor metadata. `SlackChannelAdapter::fetch_attachment` uses host-injected bot credentials to:

1. call `files.info` for current metadata and a private download URL;
2. validate type and declared size against host limits;
3. fetch the private URL with bearer authorization through declared egress;
4. enforce the byte limit while reading;
5. return bytes to the shared lander without persisting the private URL.

### Steps

- [ ] Add mock-provider tests for one file, multiple files, missing `files:read`, `files.info` error, private download redirect, oversized declared size, oversized streamed body, MIME mismatch, timeout, and mixed-success batch.
- [ ] Run `cargo test -p ironclaw_slack_extension inbound_attachment` and confirm `fetch_attachment` is unsupported.
- [ ] Implement Slack API response DTOs with `deny_unknown_fields` only where Slack response evolution permits it; keep token and private URLs out of debug/display output.
- [ ] Use the shared HTTP/network mediation surface and exact allowlisted Slack hosts. Do not create an unrestricted `reqwest::Client`.
- [ ] Preserve the Slack file ID as `vendor_ref`; do not persist `url_private` or `url_private_download`.
- [ ] Add `files:read` and exact inbound-download egress requirements to manifest and setup documentation.
- [ ] Add conformance coverage proving the shared product lander receives complete bytes and all-or-nothing behavior remains in the shared layer.
- [ ] Run:

```bash
cargo test -p ironclaw_slack_extension --all-features
cargo test -p ironclaw_reborn_composition slack --all-features
```

- [ ] Run targeted clippy and commit:

```bash
git add crates/ironclaw_slack_extension crates/ironclaw_first_party_extensions crates/ironclaw_reborn_composition/tests/first_party_manifest_v3_parity.rs docs/channels/slack.mdx docs/reborn/setup-slack-for-reborn-binary.md
git commit -m "feat(slack): fetch inbound attachments"
```

---

## Task 8: Implement Slack external file upload and read-back evidence

**Files:**

- Modify: `crates/ironclaw_slack_extension/src/channel.rs`
- Modify: `crates/ironclaw_slack_extension/src/delivery.rs`
- Modify: `crates/ironclaw_slack_extension/tests/channel_conformance.rs`
- Modify: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`
- Modify: `crates/ironclaw_reborn_composition/tests/first_party_manifest_v3_parity.rs`
- Modify: `docs/channels/slack.mdx`
- Modify: `docs/reborn/setup-slack-for-reborn-binary.md`

### Contract

For each `OutboundPart::File`, Slack delivery uses the supported external upload flow:

1. `files.getUploadURLExternal`;
2. upload bytes to the returned provider URL;
3. `files.completeUploadExternal` with the destination conversation/thread;
4. `files.info` read-back, or equivalent authoritative response evidence, confirming the resulting file ID and destination.

An attempt is successful only when text and every file have authoritative provider evidence. Partial provider success remains a failed composite attempt with explicit evidence; retries reuse persisted semantic refs and provider idempotency/evidence where Slack permits.

### Steps

- [ ] Add red tests that reject the retired `files.upload` endpoint and expect the three-step external upload flow.
- [ ] Add tests for upload URL failure, binary upload failure, completion failure, read-back mismatch, multiple ordered files, thread replies, zero-byte files, MIME/filename forwarding, token redaction, and partial success evidence.
- [ ] Run `cargo test -p ironclaw_slack_extension outbound_attachment` and confirm file parts are currently rejected.
- [ ] Add bounded binary upload support through the mediated network surface for only the provider-issued upload host.
- [ ] Implement external upload request/response DTOs and evidence mapping.
- [ ] Update `SlackChannelAdapter::deliver` to render text and transfer files from `OutboundContent.parts` without changing semantic state.
- [ ] Add `files:write` and exact upload egress requirements to manifest and documentation.
- [ ] Run:

```bash
cargo test -p ironclaw_slack_extension --all-features
cargo test -p ironclaw_product outbound_delivery --all-features
```

- [ ] Run targeted clippy and commit:

```bash
git add crates/ironclaw_slack_extension crates/ironclaw_first_party_extensions crates/ironclaw_reborn_composition/tests/first_party_manifest_v3_parity.rs docs/channels/slack.mdx docs/reborn/setup-slack-for-reborn-binary.md
git commit -m "feat(slack): upload outbound attachments"
```

---

## Task 9: Lock Telegram and WebUI parity with deterministic end-to-end tests

**Files:**

- Modify: `crates/ironclaw_telegram_extension/tests/channel_conformance.rs`
- Modify: `crates/ironclaw_product/tests/inbound_turn_contract.rs`
- Modify: `crates/ironclaw_product/tests/run_delivery_contract.rs`
- Modify: `crates/ironclaw_webui/tests/webui_v2_handlers_contract.rs`
- Modify: `crates/ironclaw_webui/tests/session_round_trip.rs`
- Add: `crates/ironclaw_reborn_composition/tests/cross_channel_attachment_e2e.rs`

### Scenarios

Run the same fixture matrix for WebUI, Telegram, and Slack:

- one inbound text file becomes a durable attachment ref and readable workspace file;
- multiple inbound files commit together and preserve order;
- one invalid file leaves no file and no accepted message;
- the agent reads an inbound file and cites its stable workspace path;
- the agent writes a new workspace file, explicitly attaches it, and the finalized assistant message owns the ref;
- path-looking prose without explicit intent sends no file;
- a missing file intent fails before provider delivery;
- an outbound text-plus-file reply appears in WebUI history and reaches the external adapter;
- retry does not duplicate intent state or change finalized refs.

### Steps

- [ ] Add adapter-neutral product fixtures and a production composition harness using real registries, mount views, stores, runner, transcript finalizer, and delivery observer.
- [ ] Keep provider HTTP deterministic with local mock servers; assert exact API requests and evidence.
- [ ] Run the matrix against an in-memory filesystem, LibSQL, and PostgreSQL where the persistence contract differs.
- [ ] Verify Telegram still uses `sendDocument`, shared budgets, and production read-write landing after the filesystem change.
- [ ] Verify WebUI upload/history/file-read routes round-trip structured refs and enforce auth, ownership, body limits, and `nosniff`.
- [ ] Run:

```bash
cargo test -p ironclaw_telegram_extension --all-features
cargo test -p ironclaw_slack_extension --all-features
cargo test -p ironclaw_webui attachment --all-features
cargo test -p ironclaw_reborn_composition --test cross_channel_attachment_e2e --all-features
```

- [ ] Run `cargo test -p ironclaw_architecture`.
- [ ] Run clippy for every changed Rust package and frontend tests/typecheck.
- [ ] Commit:

```bash
git add crates/ironclaw_telegram_extension crates/ironclaw_product crates/ironclaw_webui crates/ironclaw_reborn_composition
git commit -m "test(attachments): cover cross-channel end to end flow"
```

---

## Task 10: Run production-stack verification, document rollout, and publish

**Files:**

- Modify: `docs/superpowers/specs/2026-07-29-generic-cross-channel-attachments-design.md`
- Modify: `docs/superpowers/plans/2026-07-29-generic-cross-channel-attachments.md`
- Modify: `.github/pull_request_template.md` only if the existing template cannot express the required evidence
- Modify: PR #6364 body `Test Strategy` section through GitHub after local verification

### Steps

- [ ] Search all changed production files for `.unwrap()`, `.expect()`, suspicious byte slicing, hardcoded temporary paths, tokens, provider URLs, and lost error causes.
- [ ] Search all `crates/` for sibling prose-path inference and unsupported attachment branches.
- [ ] Review the full diff for authorization, actor scope, limits, redaction, rollback, compatibility, and provider side-effect evidence.
- [ ] Run `git diff --check`.
- [ ] Run the repository-prescribed narrow test tiers from `docs/internal/testing-playbook.md`, including architecture, changed-package clippy, backend parity, frontend tests, and the cross-channel E2E.
- [ ] Boot the production stack with the existing secret-safe local access bundle. Verify health and authenticate without printing bearer tokens.
- [ ] WebUI live test: upload multiple files, read them, create a file, explicitly attach it, download the returned ref, and verify path-looking prose alone has no chip/file delivery.
- [ ] Telegram live test: send one and multiple files, verify the durable refs and agent reads, then explicitly attach a generated file and verify the received document.
- [ ] Before changing Slack scopes or reinstalling the Slack app, obtain explicit user confirmation for the external workspace mutation.
- [ ] Slack live test after confirmed scope update: receive one and multiple files, verify atomic landing and reads, then explicitly attach a generated file and verify provider read-back and received file.
- [ ] Re-run the negative live tests for oversize, missing file, mixed-validity batch, and prose-only path.
- [ ] Mark this plan’s completed checkboxes and update the design status to `Implemented and verified` only after all three surfaces pass.
- [ ] Fetch the PR branch and confirm the remote head is still an ancestor of the local commit stack. Do not force-push.
- [ ] Push `HEAD` to `origin/codex/telegram-slack-attachments`.
- [ ] Update PR #6364’s title/body so every changed layer, compatibility behavior, rollback plan, risks, and complete `Test Strategy` evidence are represented.
- [ ] Re-query CI, review decision, unresolved threads, and merge state. Do not report merge-ready while any gate is unresolved.
- [ ] Commit final documentation changes before the push:

```bash
git add docs/superpowers/specs/2026-07-29-generic-cross-channel-attachments-design.md docs/superpowers/plans/2026-07-29-generic-cross-channel-attachments.md
git commit -m "docs: record cross-channel attachment verification"
```

## Completion Evidence

The implementation is complete only when all of the following are true:

- every supported inbound surface produces the same durable `AttachmentRef` semantics;
- no invalid multi-file batch leaves any landed bytes or accepted message;
- the production channel host uses a writable landing mount without broadening unrelated permissions;
- the only outbound file trigger is `builtin.attach_workspace_file_to_reply`;
- finalized assistant messages durably contain ordered attachment refs;
- delivery never scans prose for workspace paths;
- WebUI renders/downloads structured refs;
- Telegram and Slack transfer exactly the finalized refs;
- Slack uses the external upload flow, not retired `files.upload`;
- all provider credentials and transient URLs remain host-side and redacted;
- in-memory, local, LibSQL, and PostgreSQL filesystem behavior is covered;
- production composition and live WebUI/Telegram/Slack E2E are verified;
- the pushed PR head, CI, review, and unresolved-thread state are reported exactly.
