# Combined Slack Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan. This session must use inline execution because the active agent instructions do not authorize subagent spawning.

**Goal:** Consolidate PRs #5957 and #5983 into PR #5957, reconcile their overlapping extension-removal designs, fix every validated audit blocker, and publish a current-main, tested, review-clean PR.

**Architecture:** Keep #5983's trusted, explicit cleanup-requirements registry as the authority for external uninstall effects. Layer #5957's durable retry/tombstone behavior on that contract so local package loss or process restart cannot erase cleanup obligations. Keep OAuth callback parsing at ingress, durable flow ownership in `ironclaw_auth`/product-auth, extension activation behind the lifecycle facade, Slack epoch fencing behind the Slack connection lifecycle, and frontend polling observationally consistent with explicit mutating reconciliation commands.

**Tech Stack:** Rust/Tokio/Axum, filesystem-backed CAS stores, React/TypeScript/Vitest, repository integration harness, GitHub CLI.

## Global Constraints

- Preserve private-install masked-denial ordering: unauthorized callers must continue to observe “not installed,” including concurrent waiters.
- Do not use `.unwrap()` or `.expect()` in production Rust.
- Do not infer Slack cleanup from extension surface kind; only trusted cleanup metadata may select an adapter.
- GET status endpoints remain read-only. Recovery/continuation dispatch must use an explicit command path or background owner and be deadline-bounded.
- Durable claims, credential generations, connection epochs, and installation ownership remain fenced; stale work must not revoke or complete newer state.
- Every behavior change starts with a failing caller-level or integration test.
- The final branch must include current `origin/main`, both original PR heads, updated contracts/changelog, zero unresolved review threads, and fresh verification evidence.

---

### Task 1: Form the combined branch and reconcile merge conflicts

**Files:**
- Modify only conflict paths selected by Git, especially `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Preserve `crates/ironclaw_reborn_composition/src/extension_host/extension_removal_cleanup.rs`

- [ ] Merge `origin/main` into `codex/slack-idempotent-remove` without rewriting published history.
- [ ] Merge exact PR #5983 head `origin/pr-5983` into the same branch.
- [ ] Resolve overlap by retaining explicit `ExtensionRemovalCleanupRequirements` and typed cleanup adapters while retaining #5957's OAuth, continuation, stale-tool, and durable state machinery.
- [ ] Confirm both exact original heads and current main are ancestors with `git merge-base --is-ancestor`.
- [ ] Run merge-baseline compilation/tests for `ironclaw_auth`, `ironclaw_reborn_composition`, and the WebUI frontend before authored fixes; record any merge-only failure separately.

### Task 2: Make explicit extension removal retryable across failure and restart

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_removal_cleanup.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Modify: lifecycle persistence/store modules selected by the live code owner
- Test: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle/tests/`
- Test: `tests/integration/group_extensions/`
- Test support: `tests/integration/support/harness/profiles/extension.rs`

- [ ] Add a failing test that removes the public Slack extension through production wiring and proves package, manifest, installation, credential, identity, pairing, DM-target, and personal connection state converge.
- [ ] Add a failing test that a registered Slack cleanup adapter with an empty late-bound facade slot fails closed and keeps cleanup retryable.
- [ ] Add a failing concurrent test proving the authenticated winner and unauthenticated waiter preserve masked “not installed” precedence.
- [ ] Add a failing true-restart test that creates a fresh lifecycle service after catalog/package loss and successfully retries cleanup from durable state.
- [ ] Introduce a durable, typed removal obligation/tombstone containing the trusted cleanup requirements and minimum manifest/ownership data needed for retry; validate all persisted fields on load.
- [ ] Normalize installed and orphan teardown into one idempotent pipeline: authorize, persist obligation, run required external adapters, revoke extension credentials, unpublish/remove local runtime state when present, delete installation/manifest, then clear the obligation only after read-back convergence.
- [ ] Avoid requiring an in-memory lifecycle package on restart; use persisted teardown data for orphan cleanup.
- [ ] Keep the operation lock around authorization and obligation transitions, but do not hoist the actor check ahead of masked authorization.
- [ ] Run focused lifecycle unit and integration scenarios until green.

### Task 3: Terminalize malformed OAuth callbacks and separate read from recovery

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/serve/oauth.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs`
- Modify: product-workflow/auth route descriptors if the command surface changes
- Test: route tests beside the above modules
- Test: `crates/ironclaw_reborn_composition/tests/auth_callbacks.rs`

- [ ] Add failing callback tests for known flows with missing code, missing PKCE, invalid scope, and provider denial; each must persist a terminal failure before one-shot material is discarded and expose `failed` on status read.
- [ ] Route known malformed callbacks through the typed `RebornOAuthCallbackOutcome::Malformed`/`fail_oauth_callback` path while preserving state-hash and scope checks.
- [ ] Add a failing test proving GET status performs no writes, continuation dispatch, activation, compensation, or terminal hooks.
- [ ] Add/retain an explicit bounded mutating reconciliation endpoint/command, or move recovery to the durable callback/worker owner; update frontend callers to invoke it before observational reads when recovery is required.
- [ ] Apply one overall backend deadline to reconciliation, including provider terminal hooks.
- [ ] Preserve/log backend causes before returning sanitized route failures.

### Task 4: Fix lifecycle continuation correctness and concurrency

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/lifecycle_auth_continuation.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: extension credential-readiness contracts in their owning module
- Test: `crates/ironclaw_reborn_composition/src/factory/auth_tests.rs`
- Test: `crates/ironclaw_reborn_composition/src/product_auth/durable/tests.rs`
- Test: `tests/integration/` production-factory OAuth lifecycle scenario

- [ ] Add a failing production-wrapper test proving successful lifecycle activation delegates to blocked-auth resume fanout and resumes a waiting run.
- [ ] Add a failing two-provider extension test proving the first credential completes durably without compensation, reports remaining credential blockers, and the second completion activates/publishes tools.
- [ ] Replace string/error inference with a typed activation readiness outcome that distinguishes active, incomplete credentials, and real activation failure.
- [ ] Add a failing concurrency test where one slow lifecycle continuation cannot block another flow/user.
- [ ] Remove the service-global mutex; rely on the durable per-flow claim/lease and flow-local fencing (or a garbage-collected keyed guard only if the live store requires local exclusion).
- [ ] Add filesystem-backed simultaneous claim, stale-lease reclamation, and stale-owner settlement tests.
- [ ] Add the already-absent compensation test and prove its fingerprint journal converges idempotently.

### Task 5: Fence Slack cleanup before fallible identity deletion and close OAuth/removal races

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_personal_oauth.rs`
- Modify: Slack connection lifecycle/store owner selected from live code
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs`
- Modify: narrow installed-extension/lifecycle operation port and factory wiring
- Test: Slack lifecycle tests and production-factory tests

- [ ] Add a failing test where identity deletion fails twice and prove the target epoch is immediately fenced from ingress while durable cleanup remains retryable.
- [ ] Add a failing reconnect test proving fencing a failed pending epoch does not disconnect a previous valid active epoch.
- [ ] Introduce a typed fence/begin-cleanup transition that preserves the target epoch/owner for retry, then perform identity deletion and final abandonment idempotently.
- [ ] Add a failing race test that removal cannot complete between OAuth installation validation and flow/epoch creation.
- [ ] Replace the optional production WebUI installation lookup with a required narrow lifecycle-owned port; wire an explicit fake in tests and serialize/start with the same installation mutation generation or guard used by removal.

### Task 6: Validate durable credential identities and update auth contracts

**Files:**
- Modify: `crates/ironclaw_auth/src/credential.rs`
- Modify: `crates/ironclaw_auth/src/cleanup.rs`
- Modify: `docs/reborn/contracts/auth-product.md`
- Modify: crate guidance/API docs affected by continuation semantics
- Test: `crates/ironclaw_auth/tests/auth_product_contract/`

- [ ] Add failing serde tests rejecting empty, non-hex, and non-64-character persisted credential fingerprints while accepting canonical lowercase SHA-256 hex.
- [ ] Deserialize through the validated constructor/newtype pattern.
- [ ] Document compensation outcome guarantees, durable continuation claim/settlement, incomplete multi-credential activation, acknowledgement retry, terminal activation failure, explicit reconciliation, and observational GET semantics.

### Task 7: Fix Extensions and chat OAuth frontend state machines

**Files:**
- Modify: `crates/ironclaw_webui/frontend/src/pages/extensions/components/configure-modal.tsx`
- Test: `crates/ironclaw_webui/frontend/src/pages/extensions/components/configure-modal.test.ts`
- Modify: `crates/ironclaw_webui/frontend/src/pages/extensions/hooks/useExtensions.ts`
- Test: `crates/ironclaw_webui/frontend/src/pages/extensions/hooks/useExtensions-oauth.test.mjs`
- Modify: `crates/ironclaw_webui/frontend/src/pages/chat/hooks/useChannelOnboarding.ts`
- Add/modify: caller-level chat onboarding tests

- [ ] Add a failing real-shape test proving public Slack (`wasm_tool`) emits the Extensions OAuth-connected event.
- [ ] Gate notification by the trusted Slack-tools identity/channel binding as well as connectable kinds.
- [ ] Add failing watcher tests proving `canceled` and `expired` terminate immediately and malformed durable failures do not wait ten minutes.
- [ ] Defer only callback stages with an explicit durable reconciliation path.
- [ ] Add a deferred-response race test: flow A's stale response arrives after flow B starts and cannot complete, clear, fail, or notify for flow B.
- [ ] Capture a flow generation/snapshot for every async operation and compare it before every shared-ref/UI mutation and after each await; abort obsolete requests on cleanup.
- [ ] Run targeted Vitest suites, frontend typecheck, and lint.

### Task 8: Add production-tier coverage and whole-path verification

**Files:**
- Modify/add: `tests/integration/group_extensions/` scenarios
- Modify/add: integration scenarios for OAuth callback -> activation -> blocked-run fanout
- Modify: `tests/integration/support/` only at production seams
- Modify: `CHANGELOG.md` and parity/spec documents if live behavior/status changes

- [ ] Exercise real composition/factory wiring with only hermetic external-service doubles for Slack removal and OAuth lifecycle activation.
- [ ] Assert external cleanup evidence and read-back, not only `removed: false/true` response fields.
- [ ] Run owning-crate tests and Clippy with all targets/features and `-D warnings`.
- [ ] Run `cargo test -p ironclaw_architecture`.
- [ ] Run the focused root integration binaries/scenarios, `bash scripts/reborn-e2e-rust.sh`, and `scripts/pre-commit-safety.sh`.
- [ ] Run the workspace-wide Clippy command from `.claude/rules/review-discipline.md`.
- [ ] Search changed production files for `.unwrap()`, `.expect()`, hardcoded temporary paths, suspicious slicing, swallowed causes, and sibling instances of fixed patterns.
- [ ] Review `git diff --check`, the complete diff against current main, docs/parity/changelog obligations, and exact staged files.

### Task 9: Publish one review-clean PR and perform acceptance QA

**Files:**
- Update PR #5957 title/body and review threads through GitHub
- Close PR #5983 only after the combined push succeeds

- [ ] Commit the scoped combined implementation and push normally to `codex/slack-idempotent-remove`.
- [ ] Reply to the #5983 actor-check review with the masked-denial concurrency invariant, resolve the thread, and confirm zero unresolved threads on the combined PR.
- [ ] Update PR #5957 title/body to describe both original scopes, compatibility, rollback, test evidence, and remaining risks; mark #5983 superseded and close it.
- [ ] Monitor all required checks for the exact pushed SHA; diagnose/fix any failure and re-run until green.
- [ ] Boot the exact combined SHA with the repository live-test workflow; verify the server cwd and commit.
- [ ] Use the in-app browser to test WebUI chat, Slack install, real OAuth connection approval, tool availability, uninstall, cleanup read-back, and reinstall/reconnect as applicable.
- [ ] Stop/clean the test stack and tunnel, re-check GitHub mergeability/review decision/check rollup, and report any human-approval or secret-gated check separately from code readiness.
