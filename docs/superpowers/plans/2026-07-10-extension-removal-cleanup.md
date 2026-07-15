# Explicit Extension Removal Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to execute this plan task-by-task, with task-compliance review before moving to the next task.

**Goal:** Make every channel extension removable while running only cleanup explicitly owned by that extension, with extensive red-green TDD coverage for lifecycle, Slack, WebUI, and model-capability callers.

**Architecture:** Trusted `AvailableExtensionPackage` metadata carries typed removal-cleanup requirements. A small adapter registry executes those requirements from authenticated caller scope. `RebornLocalExtensionManagementPort::remove` remains the single convergence point and invokes cleanup before the existing local deletion and credential behavior. Surface kinds and facade capability probing play no role.

**Tech Stack:** Rust 2024, Tokio, `async-trait`, existing Reborn lifecycle/catalog and channel facade, Cargo tests and Clippy.

## Global Constraints

- Work only in Reborn `crates/`; do not modify v1 `src/`.
- Do not alter installation, manifest, auth-account, or database schemas.
- Preserve existing credential cleanup semantics; issue #5953 is channel cleanup.
- Cleanup requirements come only from trusted package/catalog projection.
- Never infer cleanup from `ExternalChannel`, credentials, package-id checks inside lifecycle removal, connection maps, or facade support probing.
- Missing or failed declared cleanup stops removal before local deletion.
- Generic channels with no declared host-owned cleanup remove normally.
- Both WebUI and `builtin.extension_remove` must use the same management-port path.
- Tests must be written and observed failing before production implementation.

---

## Task 1: Capture the current behavior RED regressions

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle_capabilities.rs`

### Steps

- [ ] Replace `extension_remove_fails_required_cleanup_when_channel_facade_is_unset` with a test expecting a generic external channel to remove successfully without a facade, including package-file, manifest, and installation assertions.
- [ ] Update the Slack caller double to count `caller_channel_connections` calls, then assert Slack removal calls disconnect but never status discovery.
- [ ] Add a lifecycle regression showing that a registered Slack facade must not receive a disconnect for an unrelated generic channel.
- [ ] Run each new focused test against current production code and record the expected failures in the task report. Keep this task compileable and do not modify production behavior.
- [ ] Commit the red tests as `test(reborn): specify explicit extension removal cleanup`.

## Task 2: Build the typed cleanup contract and trusted catalog metadata

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Create: `crates/ironclaw_reborn_composition/src/extension_host/extension_removal_cleanup.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/mod.rs`

### Steps

- [ ] Before implementing each production seam below, add and run the corresponding test so it fails for the expected reason. Record every RED and GREEN command/result in the task report.
- [ ] Add catalog tests proving `slack` has one explicit personal cleanup requirement while `slack_bot`, ordinary packages, and a generic `ExternalChannel` package have none.
- [ ] Define validated typed adapter/channel ids, `ExtensionRemovalCleanupBinding`, and `ExtensionRemovalCleanupRequirement`.
- [ ] Add cleanup requirements to `AvailableExtensionPackage`, defaulting to empty for filesystem and ordinary bundled packages.
- [ ] Attach Slack personal cleanup only in `slack_package()`; do not infer it inside removal.
- [ ] Implement an adapter trait/registry with unit tests for deterministic dispatch, duplicate-adapter rejection, unknown required adapters, wrong binding rejection, and sanitized adapter errors. Its cleanup call receives trusted `ResourceScope` plus the authenticated actor.
- [ ] Implement the Slack adapter over the existing late-bound channel facade. Call `disconnect_channel_for_caller` directly; never call `caller_channel_connections`.
- [ ] Run the new cleanup-module and catalog tests green, then commit as `feat(reborn): declare extension removal cleanup`.

## Task 3: Wire removal callers and delete obsolete channel logic

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle_capabilities.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`

### Steps

- [ ] Before production wiring, add and run lifecycle tests for exact adapter dispatch, unrelated-adapter non-invocation, missing adapter fail-closed behavior, adapter-error fail-closed behavior, authenticated-actor scoping, and cleanup-before-file-deletion ordering. Record each RED result.
- [ ] Extend the production-shaped caller test before wiring the final path so WebUI and `builtin.extension_remove` each remove a generic channel and never call Slack cleanup.
- [ ] Rewire `RebornLocalExtensionManagementPort::remove` to resolve explicit requirements, execute them in deterministic order, then run the existing local and credential cleanup behavior.
- [ ] Delete every legacy channel-removal item named in the design spec, including credential-based channel probing and the management-port channel-facade field/builder.
- [ ] Run all Task 1 tests and confirm they turn green.
- [ ] Run existing extension removal and credential-sharing tests to prove preserved behavior.
- [ ] Commit as `fix(reborn): use explicit extension removal cleanup`.

## Task 4: Verify callers, regressions, and code quality

**Files:**

- Modify tests only if verification exposes a real missing assertion.
- Check: `FEATURE_PARITY.md`
- Check: `CHANGELOG.md`

### Steps

- [ ] Run focused lifecycle, catalog, Slack, WebUI, and model-capability removal tests with required feature flags.
- [ ] Run `cargo test -p ironclaw_extensions`.
- [ ] Run `cargo test -p ironclaw_reborn_composition --lib`.
- [ ] Run `cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings` and report any verified unrelated baseline failures separately.
- [ ] Run `cargo fmt --check` and `git diff --check`.
- [ ] Require no production hits for `RemovableChannelCleanup`, `removable_channel_cleanup_for_summary`, `disconnect_channel_for_cleanup`, `cleanup_channel_before_remove`, or `IfConnectionFacadeSupportsChannel`.
- [ ] Check `FEATURE_PARITY.md` and `CHANGELOG.md`; change them only if current repository conventions make the bug fix status inaccurate.
- [ ] Run final task and whole-branch code reviews; fix all Critical/Important findings and reverify.

## Completion Criteria

- Every undeclared/generic channel uninstalls without Slack coupling.
- Every declared cleanup invokes exactly its registered adapter before deletion.
- Missing/failing declared cleanup cannot leave a falsely successful uninstall.
- Slack remains actor-scoped and ordered before local deletion without status probing.
- Existing credential behavior and persisted schemas are unchanged.
- Both public removal doors have caller-level regression coverage.
- Red and green test evidence is recorded.
