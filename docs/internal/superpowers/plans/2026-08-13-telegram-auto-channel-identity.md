# Telegram Automatic Linked Channel Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a successful linked-device login automatically establish the caller's channel identity, so Telegram requires one connection ceremony while retaining Bot API ingress/replies and MTProto personal messaging tools.

**Architecture:** Add `device_link` as explicit manifest/product channel strategy data. At device-link completion, a vendor-blind extension-host service validates the active installation and strategy, creates an installation-scoped identity binding with an exact conditional rollback receipt, then lets the existing credential service durably mint the linked account. Existing channel resolution, first-DM target capture, delivery, and disconnect cleanup remain the owners of their current stages.

**Tech Stack:** Rust/Tokio, serde/TOML manifest contracts, `RootFilesystem` CAS persistence, React/TypeScript frontend, repository integration harness, live WebUI and Telegram.

**Spec:** `docs/internal/design/telegram-linked-device/AUTO-CHANNEL-IDENTITY.md`

## Global constraints

- Test first: observe every new regression test fail for the intended missing behavior before changing production code.
- Keep the host vendor-blind; provider and strategy come from the active manifest.
- Never expose or log phone numbers, login codes, session blobs, or administrator secrets.
- Preserve Telegram Bot API ingress/reply/delivery and MTProto tool execution as separate paths.
- Live write tests may target only `@ironclawqa_bot`; all product prompts must read like normal user requests.
- Preserve the three pre-existing Telegram documentation edits as a distinct change.

---

### Task 1: Add the linked-device channel strategy to contracts and projections

**Files:**

- Modify: `crates/contracts/ironclaw_extension_contracts/src/channel.rs`
- Modify: `crates/contracts/ironclaw_product_contracts/src/package_lifecycle.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/available_extensions.rs`
- Modify exhaustive consumers found by `rg -n "ChannelConnectStrategy|ChannelConnectionStrategy" crates`
- Test: existing unit tests in those files and `crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs`

- [ ] Add failing serialization/parser/projection tests proving `device_link` survives manifest parsing and product projection.
- [ ] Run the focused tests and capture the expected enum/match failures.
- [ ] Add `DeviceLink` to both strategy enums, its stable `device_link` wire value, projection mapping, and exhaustive user-facing fallback matches.
- [ ] Keep deep links and inbound code prefixes restricted to `WebGeneratedCode` and prove a `DeviceLink` descriptor rejects them.
- [ ] Run focused contract, registry, host, assistant, and architecture tests.

### Task 2: Make Telegram declare one connection ceremony

**Files:**

- Modify: `crates/extensions/packages/telegram/manifest.toml`
- Modify: `crates/app/ironclaw_architecture_tests/tests/telegram_extension_gates.rs`
- Modify: Telegram package/manifest tests that pin channel setup copy
- Modify: `crates/product/ironclaw_webui/frontend/src/pages/extensions/lib/extensions-schema.test.ts`
- Modify: `crates/product/ironclaw_webui/frontend/src/pages/extensions/components/configure-modal.test.ts`
- Modify static asset contract assertions only if their current WebGeneratedCode expectation is Telegram-specific

- [ ] Add failing tests proving Telegram projects `device_link`, exposes no generated proof code/deep link, and routes Configure to the existing device-link panel.
- [ ] Run Rust and frontend tests to observe the old `web_generated_code` behavior fail.
- [ ] Change Telegram's manifest strategy and connection copy; remove `deep_link_template` and `inbound_code_prefixes`.
- [ ] Keep the existing device-link secret/card as the single setup UI; do not create a duplicate frontend flow.
- [ ] Run package, manifest, WebUI schema/modal, static asset, and architecture tests.

### Task 3: Add exact conditional identity binding and rollback

**Files:**

- Modify: `crates/contracts/ironclaw_host_api/src/user_identity.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/channel_identity_store.rs`
- Modify all implementations/test doubles found by `rg -n "impl RebornUserIdentity.*Store" crates tests`
- Test: `crates/extensions/ironclaw_extension_host/src/channel_identity_store.rs`

- [ ] Add failing store tests for a created-vs-existing result, exact owner-scoped conditional deletion, and a stale receipt that cannot delete a newer version.
- [ ] Run the focused store tests and observe the missing API failures.
- [ ] Add neutral binding outcome/receipt vocabulary and a filesystem CAS implementation that returns the written version for new bindings.
- [ ] Implement rollback as exact provider + provider-user + IronClaw-user + expected-version deletion; never use prefix deletion for compensation.
- [ ] Preserve existing generic bind/delete APIs for pairing/OAuth/disconnect callers and update test doubles without weakening behavior.
- [ ] Run host-api, extension-host, OAuth journey, composition test-support, and architecture tests.

### Task 4: Bind channel identity inside device-link completion

**Files:**

- Add: `crates/extensions/ironclaw_extension_host/src/device_link_channel_identity.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/lib.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/active.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/device_link_driver.rs`
- Modify: `crates/extensions/ironclaw_extension_host/src/device_link_driver/tests.rs`
- Modify test manifest helper in `crates/extensions/ironclaw_extension_host/src/test_support.rs`

- [ ] Extend the caller-level driver tests first: new identity, same identity relink, different identity for caller, identity owned by another user, provider/strategy mismatch, custody rollback, and stale rollback protection.
- [ ] Run the focused tests and observe failures before adding production behavior.
- [ ] Carry the active installation ID in `ResolvedDeviceLinkBinding` and validate channel provider/strategy against the device-link auth vendor.
- [ ] Build the installation-scoped external identity from the authenticated `vendor_user_ref`; do not accept a browser/model value.
- [ ] Begin the binding transaction before credential completion; commit it after account mint and await exact rollback on mint/custody failure.
- [ ] Map conflicts to stable sanitized non-restartable device-link failures and keep PII out of logs.
- [ ] Run all extension-host tests plus clippy for the crate.

### Task 5: Wire production composition and converge unlink/disconnect

**Files:**

- Modify: `crates/app/ironclaw_composition/src/factory/production_backend_assembly.rs`
- Modify: `crates/app/ironclaw_composition/src/extension_host_assembly.rs` if constructor input ownership requires it
- Modify: `crates/extensions/ironclaw_extension_host/src/product_lifecycle.rs`
- Modify: `crates/product/ironclaw_assistant/src/reborn_services/extensions.rs`
- Test: existing channel connection/product lifecycle tests and composition factory tests

- [ ] Add failing caller-level tests proving no pairing service is built for `DeviceLink` and unlink routes through one channel disconnect coordinator.
- [ ] Run focused tests and observe old routing/exhaustive behavior fail.
- [ ] Inject the existing filesystem identity store into the device-link driver/binder.
- [ ] Ensure `DeviceLink` is never registered in `ChannelPairingRegistry` and connection state reads the normal identity store.
- [ ] Route device-link-channel unlink through credential/session revocation, DM target cleanup, then installation-scoped identity cleanup; keep non-channel device links on the ordinary credential path.
- [ ] Prove extension removal uses the same cleanup authority and revokes/logs out the linked device before deleting credential material, DM target, identity binding, and installation state.
- [ ] Run composition, assistant, product lifecycle, auth, channel connection, and architecture tests.

### Task 6: Prove the production-wired behavior at the integration seam

**Files:**

- Extend: `tests/integration/group_device_link/` and/or the closest existing production-wired channel delivery scenario
- Modify: `tests/integration/support/harness/profiles/device_link.rs` only for a production-shaped inspection seam
- Modify: `tests/integration/CLAUDE.md` coverage map if required

- [ ] Add the failing integration scenario: complete link with literal actor ID, connected/no-code projection, admit first verified DM as linked user, record DM target, disconnect all three durable resources, reject later actor message.
- [ ] Add failure legs for collision, custody rollback, same-account relink, and different-account rejection.
- [ ] Run the exact integration test and observe the pre-implementation failure at the production caller seam.
- [ ] Add only the minimum harness seam needed to inspect durable identity, credential, and target outcomes.
- [ ] Run the integration group and its sibling channel delivery tests.

### Task 7: Documentation, verification, and live Telegram acceptance

**Files:**

- Modify: `docs/internal/design/telegram-linked-device/AUTO-CHANNEL-IDENTITY.md` status
- Finalize the pre-existing edits in `crates/extensions/AGENTS.md`, `crates/extensions/packages/telegram/AGENTS.md`, and `crates/extensions/packages/telegram/README.md`
- Update owning contracts/README where behavior changed

- [ ] Run `cargo fmt` and scan changed production files for forbidden unwrap/expect, lost error causes, unsafe byte slicing, and hardcoded temporary paths.
- [ ] Run focused crate suites, frontend tests/build, integration scenario, `cargo test -p ironclaw_architecture_tests`, and focused zero-warning clippy.
- [ ] Start the full local stack with Telegram enabled from this checkout and confirm the configured administrator credentials remain accepted without exposing them.
- [ ] Unlink/relink the local Telegram account through the WebUI; confirm connected state and absence of a pairing code.
- [ ] Send ordinary natural-language prompts through the bot; prove same-user ingress, group/DM reads, user resolution, and Bot API reply/delivery.
- [ ] Exercise send/edit/react/unreact/delete only with `@ironclawqa_bot`; do not write to any person or other chat.
- [ ] Unlink and prove both channel admission and personal linked-account tools are removed, then relink if leaving a usable development stack is desired.
- [ ] Update the design status, record exact test/live evidence, review the diff, and keep commits scoped.
