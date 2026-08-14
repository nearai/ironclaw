# New and Stop Product Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `/new`, `/stop`, and `/interrupt` through the shared WebUI and channel command system with non-destructive, idempotent continuous-channel reset.

**Architecture:** Extend the shared typed command registry and reuse the existing create-thread and cancel-run ProductSurface paths. Channel `/new` performs a caller-scoped run preflight and then compare-and-rotates the canonical external conversation binding under the existing conversation-store mutation/CAS boundary, preserving old thread data while fencing late delivery.

**Tech Stack:** Rust 2024, Tokio, serde, `ironclaw_assistant`, `ironclaw_conversations`, `ironclaw_turns`, React 19, TypeScript 6, Vitest.

## Global Constraints

- Keep WebUI command discovery and execution unavailable until a task is active.
- Do not delete old threads, transcript records, or accepted-message audit records.
- Do not add implicit command aliases; `stop` and `interrupt` are explicit tokens.
- Do not use `.unwrap()` or `.expect()` in production Rust.
- Preserve caller, installation, route, tenant, user, agent, project, thread, and idempotency scope.
- Use failing behavior tests before every production behavior change.
- Do not stage or commit without explicit user authorization.

---

### Task 1: Atomic conversation-binding rotation

**Files:**
- Modify: `crates/domains/ironclaw_conversations/src/types.rs`
- Modify: `crates/domains/ironclaw_conversations/src/traits.rs`
- Modify: `crates/domains/ironclaw_conversations/src/memory.rs`
- Modify: `crates/domains/ironclaw_conversations/src/conversation_state_store.rs`
- Modify: `crates/domains/ironclaw_conversations/src/lib.rs`
- Test: `crates/domains/ironclaw_conversations/tests/inbound_contract.rs`
- Test: `crates/domains/ironclaw_conversations/tests/conversation_state_store_contract.rs`

**Interfaces:**
- Produces: `ResetConversationRequest { resolve, expected_thread_id }` and `ResetConversationOutcome { previous_thread_id, resolution }`.
- Produces: `ConversationBindingService::reset_conversation_binding(request)`.

- [ ] Add an in-memory caller test that binds a route, resets it, observes a new thread, resolves the route to that same new thread, retains old accepted records, and rejects the old reply ref.
- [ ] Run the focused test and confirm failure because the reset API is absent.
- [ ] Add a stale-expected-thread and duplicate-external-event replay test; confirm both fail for the missing behavior.
- [ ] Implement the typed request/outcome and atomic mutation, including durable reset replay state and old delivery-ref revocation.
- [ ] Forward the method through `RebornFilesystemConversationServices` and export the new types.
- [ ] Add durable reload coverage to the existing state-store contract.
- [ ] Run `cargo test -p ironclaw_conversations --test inbound_contract` and `cargo test -p ironclaw_conversations --test conversation_state_store_contract`.

### Task 2: Shared product command vocabulary and operations

**Files:**
- Modify: `crates/product/ironclaw_assistant/src/commands.rs`
- Modify: `crates/product/ironclaw_assistant/src/binding.rs`
- Modify: `crates/product/ironclaw_assistant/src/conversation_binding.rs`
- Modify: `crates/product/ironclaw_assistant/src/reborn_services/types.rs`
- Modify: `crates/product/ironclaw_assistant/src/reborn_services/product_commands.rs`
- Modify: `crates/product/ironclaw_assistant/src/reborn_services/product_capability_handlers.rs`
- Modify: `crates/product/ironclaw_assistant/src/reborn_services.rs`
- Modify: `crates/product/ironclaw_assistant/src/lib.rs`
- Test: `crates/product/ironclaw_assistant/tests/product_commands_contract.rs`
- Test: `crates/product/ironclaw_assistant/tests/reborn_services_contract.rs`

**Interfaces:**
- Produces: `ProductCommand::New` and `ProductCommand::Stop { invocation: ProductStopInvocation }`.
- Produces: `PRODUCT_NEW_COMMAND_OPERATION_ID` and `PRODUCT_STOP_COMMAND_OPERATION_ID`.
- Produces: `ProductNewCommandOutput { can_reset, result }` and `RebornProductCommandEffect::OpenThread { thread_id }`.
- Consumes: conversation reset API from Task 1.

- [ ] Add registry/parser/audience tests for `new`, `stop`, and `interrupt`; run them red.
- [ ] Implement descriptors and typed parsing with explicit stop invocation tokens; run them green.
- [ ] Add RebornServices tests proving channel-new preflight refuses a nonterminal latest run, stop requests canonical cancellation, repeated/no-run stop is safe, and WebUI new creates a caller-owned thread with an open-thread effect; run them red.
- [ ] Implement the new/stop operation handlers by reusing thread history, `create_thread`, `get_run_state`, and `cancel_run`.
- [ ] Extend the product binding adapter with caller/retry-safe reset forwarding.
- [ ] Run `cargo test -p ironclaw_assistant --test product_commands_contract` and the focused `reborn_services_contract` tests.

### Task 3: Channel dispatch and non-destructive reset

**Files:**
- Modify: `crates/product/ironclaw_assistant/src/workflow.rs`
- Test: `crates/product/ironclaw_assistant/tests/product_command_surface_contract.rs`
- Test: `crates/extensions/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

**Interfaces:**
- Consumes: new/stop ProductSurface operations and reset-capable product binding service.
- Produces: a rendered `CommandResultView` ack after permitted reset and no reset after active-run refusal.

- [ ] Add a product caller test proving `/new` invokes the preflight, rotates exactly once on permission, replays the same outcome on duplicate delivery, and never submits an agent turn; run it red.
- [ ] Add active-run refusal and `/stop`/`/interrupt` dispatch tests; run them red.
- [ ] Route New through preflight then binding reset, unwrap only the typed result view for delivery, and route both stop spellings through the shared stop operation.
- [ ] Extend the extension-host caller harness for the bundled continuous channel path.
- [ ] Run the focused product and extension-host tests.

### Task 4: WebUI generic navigation effect

**Files:**
- Modify: `crates/product/ironclaw_webui/frontend/src/pages/chat/hooks/useChat.ts`
- Modify: `crates/product/ironclaw_webui/frontend/src/pages/chat/chat.tsx`
- Test: `crates/product/ironclaw_webui/frontend/src/pages/chat/lib/useChat-send.test.ts`
- Test: `crates/product/ironclaw_webui/frontend/src/pages/chat/lib/chat.test.ts`

**Interfaces:**
- Consumes: `effect: { type: "open_thread", thread_id: string }` from Task 2.
- Produces: generic navigation to a command-returned task without hardcoding `new` in the frontend.

- [ ] Add a hook test proving an open-thread effect is returned as the response destination while the result notice remains scoped to the task that executed it; run it red.
- [ ] Add a page test proving a command response whose destination differs from the active task calls `onSelectThread`; run it red.
- [ ] Implement generic effect extraction and destination selection.
- [ ] Re-run the homepage empty-inventory regression and command-menu suites.

### Task 5: Manifests, contracts, and final verification

**Files:**
- Modify: `crates/extensions/packages/slack/manifest.toml`
- Modify: `crates/extensions/packages/telegram/manifest.toml`
- Modify: `docs/internal/reborn/contracts/conversation-binding.md`
- Modify: `docs/internal/superpowers/specs/2026-07-29-product-command-train-design.md`

**Interfaces:**
- Declares: `commands = ["model", "status", "new", "stop", "interrupt"]` for both bundled continuous channels.

- [ ] Extend an existing manifest/host test to assert all declared tokens validate; run it red before editing manifests.
- [ ] Update both manifests and the command-train follow-up list.
- [ ] Add reset semantics and exact test commands to the conversation-binding contract.
- [ ] Run targeted crate tests, frontend tests/typecheck/conventions, first-party manifest tests, architecture tests, clippy for changed Rust crates, `scripts/reborn-e2e-rust.sh`, and `git diff --check`.
- [ ] Review the final diff for unrelated edits, production `.unwrap()`/`.expect()`, scope loss, raw backend error leaks, and old-path references.
