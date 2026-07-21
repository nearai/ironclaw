# Telegram and Slack Attachment Review Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve the approved Telegram and Slack attachment journeys while removing the mirror file DTO, test-optional production dependency, generic implicit ACK policy, local/WebUI-specific filesystem wiring, and Slack provider behavior in composition identified during architecture and PR review.

**Architecture:** `ironclaw_attachments` owns one non-serializable generic `MaterializedFile<P>` value; project workspace reads specialize it with `ScopedPath`, and the product adapters consume that same value. Composition builds the canonical attachment lander and project reader once over the selected `RootFilesystem`, channel delivery requires the reader, Slack transfer behavior lives in `ironclaw_slack_extension`, and Slack/Telegram explicitly opt into bounded pre-ACK attachment intake.

**Tech Stack:** Rust 2024, async-trait, `RootFilesystem`/`ScopedFilesystem`, Reborn product workflow and channel delivery, Telegram Bot API, Slack Web API, Axum integration harnesses, GitHub review threads.

## Global Constraints

- Preserve the WebUI contract: 10 files maximum, 5 MiB per file, 10 MiB total.
- Store inbound bytes only through `InboundAttachmentLander` under `/workspace/attachments/...`.
- Do not add a mirror DTO, a product-specific filesystem carrier, a `Local*`/`Hosted*` implementation family, or a trait used only by tests.
- Keep `InboundAttachmentMaterializer` as the one provider-transfer port; Slack and Telegram remain its two production implementations.
- Keep provider wire DTOs and transfer policy with the provider host owner; composition only constructs and wires concrete values.
- Use one required `Arc<dyn ProjectFilesystemReader>` in `FinalReplyDeliveryServices`; do not initialize it as `None` and patch it later with a builder.
- Product hosts select pre-ACK attachment intake explicitly; the generic runner must not impose a universal attachment timeout on every adapter.
- Resolve outbound paths through `ScopedPath` and project-scoped filesystem authority; do not reconstruct authority from a string in the adapter layer.
- Do not persist raw bytes, provider URLs, upload tickets, tokens, or host paths.
- Production code contains no `.unwrap()` or `.expect()` and clippy must have zero warnings.

---

### Task 1: Canonical materialized file and required delivery reader

**Files:**
- Create: `crates/ironclaw_attachments/src/materialized_file.rs`
- Modify: `crates/ironclaw_attachments/src/lib.rs`
- Modify: `crates/ironclaw_product_adapters/Cargo.toml`
- Modify: `crates/ironclaw_product_adapters/src/adapter.rs`
- Modify: `crates/ironclaw_product_adapters/src/lib.rs`
- Modify: `crates/ironclaw_product_workflow/src/reborn_services/project_fs.rs`
- Modify: `crates/ironclaw_product_workflow/src/reborn_services/fs_browse.rs`
- Modify: `crates/ironclaw_product_workflow/src/reborn_services.rs`
- Modify: `crates/ironclaw_product_workflow/src/lib.rs`
- Modify: `crates/ironclaw_channel_delivery/src/services.rs`
- Modify: `crates/ironclaw_channel_delivery/src/observer.rs`
- Modify: `crates/ironclaw_channel_delivery/src/triggered.rs`
- Modify: `crates/ironclaw_channel_delivery/src/workspace_attachments.rs`
- Modify: all caller and test fixtures that construct `FinalReplyDeliveryServices` or project file results
- Test: `crates/ironclaw_channel_delivery/src/tests.rs`
- Test: `crates/ironclaw_product_adapters/tests/product_adapter_contract.rs`

**Interfaces:**
- Produces: `MaterializedFile<P>`, `WorkspaceFile = MaterializedFile<ScopedPath>`, and `ProjectFsFile = MaterializedFile<String>`.
- Produces: required `FinalReplyDeliveryServices.project_filesystem_reader: Arc<dyn ProjectFilesystemReader>`.
- Consumes: the existing `ProjectFilesystemReader` dependency-inversion port and `ScopedPath` validation.

- [ ] **Step 1: Write failing contract tests**

Add a product-adapter contract test that constructs a `WorkspaceFile` with a real `ScopedPath`, passes it through `render_outbound_with_attachments`, and proves the adapter receives the same path and bytes. Change the channel-delivery resolver tests to pass a required reader and delete the `None` case; add a compile-time source ratchet in `ironclaw_architecture` rejecting `pub struct ProductOutboundAttachment` and `with_project_filesystem_reader` in channel delivery.

- [ ] **Step 2: Run the red tests**

Run: `cargo test -p ironclaw_architecture attachment -- --nocapture && cargo test -p ironclaw_channel_delivery workspace_attachment -- --nocapture`

Expected: FAIL because the mirror `ProductOutboundAttachment` and optional reader builder still exist.

- [ ] **Step 3: Implement the canonical carrier**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedFile<P> {
    pub path: P,
    pub filename: Option<String>,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl<P> MaterializedFile<P> {
    pub fn size_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
}

pub type WorkspaceFile = MaterializedFile<ScopedPath>;
pub type ProjectFsFile = MaterializedFile<String>;
```

Move `ProjectFsFile` to the attachment owner as the alias above and re-export it from product workflow for compatibility. Change `ProjectFilesystemReader::read_file` to return `WorkspaceFile`; keep the standalone multi-mount browse reader on `ProjectFsFile`. Remove `ProductOutboundAttachment` and make `ProductAdapter::render_outbound_with_attachments` accept `Vec<WorkspaceFile>`. The shared resolver passes the reader-returned values directly after count/per-file/total checks.

- [ ] **Step 4: Make the delivery dependency required**

Add `project_filesystem_reader` directly to `FinalReplyDeliveryServices`; remove the observer/triggered `Option<Arc<...>>` fields and `with_project_filesystem_reader` methods. Update every production and test constructor. Test fixtures may use one test-only denying reader, but production must never omit the dependency.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p ironclaw_attachments && cargo test -p ironclaw_product_adapters && cargo test -p ironclaw_product_workflow && cargo test -p ironclaw_channel_delivery`

Expected: PASS.

### Task 2: Explicit product ACK policy and Slack provider ownership

**Files:**
- Create: `crates/ironclaw_slack_extension/Cargo.toml`
- Create: `crates/ironclaw_slack_extension/src/lib.rs`
- Create: `crates/ironclaw_slack_extension/src/attachment_materializer.rs`
- Modify: `Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/src/slack/mod.rs`
- Delete: `crates/ironclaw_reborn_composition/src/slack/slack_attachment_materializer.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs`
- Modify: `crates/ironclaw_telegram_extension/src/ingress/resolver.rs`
- Modify: `crates/ironclaw_wasm_product_adapters/src/runner.rs`
- Modify: `crates/ironclaw_wasm_product_adapters/src/runner_immediate_ack.rs`
- Test: `crates/ironclaw_wasm_product_adapters/src/runner_immediate_ack.rs`
- Test: `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`

**Interfaces:**
- Produces: public concrete `ironclaw_slack_extension::SlackAttachmentMaterializer`.
- Produces: `NativeProductAdapterRunnerConfig::with_pre_ack_attachment_workflow_timeout(Duration)` with no universal default.
- Consumes: the existing generic runner mechanism and existing `InboundAttachmentMaterializer` port.

- [ ] **Step 1: Write failing policy and ownership tests**

Add a runner test proving that an attachment-bearing envelope stays on immediate ACK when the host does not opt in. Keep the retry-before-ACK test, but configure the explicit policy there. Add an architecture test rejecting Slack provider API strings such as `/api/files.info` under `ironclaw_reborn_composition/src` and asserting `ironclaw_slack_extension` does not depend on composition.

- [ ] **Step 2: Run the red tests**

Run: `cargo test -p ironclaw_wasm_product_adapters immediate_ack -- --nocapture && cargo test -p ironclaw_architecture slack -- --nocapture`

Expected: FAIL because the generic default is pre-ACK and Slack transfer behavior remains in composition.

- [ ] **Step 3: Make ACK timing explicit**

Represent pre-ACK attachment timeout as `Option<Duration>` in `NativeProductAdapterRunnerConfig`, defaulting to `None`. The builder sets `Some(timeout)`. The runner uses synchronous bounded intake only when the configured value is present and the parsed envelope has attachment descriptors. Configure exactly 15 seconds in the Telegram extension and Slack host construction sites.

- [ ] **Step 4: Move Slack transfer behavior**

Move the materializer implementation and Slack `files.info` wire shapes unchanged into `ironclaw_slack_extension`. Expose only the concrete constructor and type. Composition imports it, injects mediated egress/credential handles, and wires it into the existing workflow port. Add the new crate to the workspace and dependency-boundary inventories.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p ironclaw_slack_extension && cargo test -p ironclaw_wasm_product_adapters && cargo test -p ironclaw_telegram_extension && cargo test -p ironclaw_reborn_composition --features test-support,libsql --lib slack`

Expected: PASS.

### Task 3: Deployment-neutral runtime attachment ports

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Modify: `crates/ironclaw_reborn_composition/src/webui/facade.rs`
- Modify: `crates/ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs`
- Test: `crates/ironclaw_reborn_composition/src/runtime/tests/core.rs`

**Interfaces:**
- Produces: required runtime-level `Arc<dyn InboundAttachmentLander>` and `Arc<dyn ProjectFilesystemReader>` values constructed once over the active scoped root.
- Consumes: `ProjectScopedAttachmentLander<F>` and `ProjectScopedFilesystemReader<F>` generic over `RootFilesystem`.

- [ ] **Step 1: Write a failing identity test**

Add a runtime composition test that obtains the attachment ports used by WebUI, Slack, and Telegram and proves they are clones of the same `Arc` values. Add a source ratchet rejecting `webui_workspace_filesystem()` calls from concrete channel host modules.

- [ ] **Step 2: Run the red test**

Run: `cargo test -p ironclaw_reborn_composition --features test-support,libsql runtime_attachment_ports -- --nocapture`

Expected: FAIL because every surface currently reconstructs its own concrete lander/reader from a WebUI/local-runtime filesystem accessor.

- [ ] **Step 3: Construct the ports once**

Build the two trait-object ports in each `RebornServices` factory path from that deployment's scoped `RootFilesystem`. Store them as required service dependencies. Expose clone-only crate-private accessors on `RebornRuntime`. Remove the attachment use of `webui_workspace_filesystem()` and remove duplicated `ProjectScopedAttachmentLander::new` / `ProjectScopedFilesystemReader::with_max_read_bytes` recipes from WebUI, Slack, Telegram, and OpenAI-compatible composition.

- [ ] **Step 4: Run composition tests**

Run: `cargo test -p ironclaw_reborn_composition --features test-support,libsql runtime_attachment_ports -- --nocapture && cargo test -p ironclaw_reborn_composition --features test-support,libsql --lib slack`

Expected: PASS.

### Task 4: Whole-path attachment journeys and review reconciliation

**Files:**
- Modify: `tests/integration/telegram_journeys/harness.rs`
- Modify: `tests/integration/telegram_journeys/scenario_attachments.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_host_beta.rs`
- Modify: `docs/reborn/contracts/telegram-v2.md`
- Modify: `scripts/reborn-e2e-rust.sh`
- Modify: `docs/superpowers/specs/2026-07-20-telegram-slack-workspace-attachments-design.md`
- Modify: `docs/superpowers/plans/2026-07-20-telegram-slack-workspace-attachments.md`

**Interfaces:**
- Produces: caller-level proof of exact inbound bytes, durable attachment refs, one accepted retry result, and native outbound provider uploads from assistant workspace references.
- Consumes: the real composed webhook, workflow, filesystem, runner, delivery observer, and mediated provider egress seams.

- [ ] **Step 1: Strengthen inbound journey assertions**

For Telegram and Slack, drive the real signed webhook, locate the one durable user message, assert one `AttachmentRef` below `/workspace/attachments/`, and read exact downloaded bytes back through the production project-scoped path. In the Telegram retry test assert exactly one accepted message, one attachment ref, one completed run/final reply, one byte download, and one delivered provider reply after the retry.

- [ ] **Step 2: Add outbound whole-path journeys**

Telegram: create `/workspace/report.txt` through the real `builtin.write_file` capability after enabling the test runtime's operator auto-approve setting, return final text containing that path, and assert one `sendDocument` multipart request containing the filename, exact bytes, target chat, and text. Slack: seed a file through the scoped workspace authority, configure a final reply containing its path, and assert `files.getUploadURLExternal` -> exact byte upload -> one `files.completeUploadExternal` with the destination/thread and reply.

- [ ] **Step 3: Map the contract to the executable suite**

Name `reborn_integration_telegram_journey` in `docs/reborn/contracts/telegram-v2.md` and add `run_test ironclaw reborn_integration_telegram_journey` to `run_architecture` in `scripts/reborn-e2e-rust.sh`.

Reconcile the original implementation plan and design spec with the final architecture: canonical `MaterializedFile<ScopedPath>` values rather than a mirror product DTO, provider-owned Slack transfer behavior, product-selected pre-ACK intake, and deployment-neutral attachment ports. Historical red-test references to removed shapes may remain only where they are explicitly described as rejected/removed; normative instructions must not contradict the shipped design.

- [ ] **Step 4: Run the whole-path tests**

Run: `cargo test --test reborn_integration_telegram_journey scenario_attachments -- --nocapture && cargo test -p ironclaw_reborn_composition --features test-support,libsql --lib slack -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Verify, publish, and reconcile review**

Run the focused crates, `cargo test -p ironclaw_architecture`, workspace-wide clippy from `.claude/rules/review-discipline.md`, `bash scripts/reborn-e2e-rust.sh`, and `scripts/pre-commit-safety.sh`. Inspect `git diff --check`, the final staged file list, and the exact PR head. Commit and push. Update PR #6364's body with exact test evidence, reply to all active review threads with the fixing commit/evidence, resolve addressed threads, and leave any genuinely unresolved thread open with a precise blocker.
