# Discord Gateway DM Pairing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Require pairing for Discord Gateway direct messages while keeping guild Gateway traffic and webhook behavior unchanged.

**Architecture:** The Discord WASM channel continues to own pairing decisions. Gateway DM events should follow the existing pairing policy instead of bypassing it, but their rejection response must use the Gateway-safe channel message route rather than interaction webhooks. The host runtime remains unchanged except for any existing metadata delivery already in place.

**Tech Stack:** Rust, WASM channel component, Discord REST API, existing pairing store APIs

---

### Task 1: Add a failing regression test for Gateway DM pairing

**Files:**
- Modify: `channels-src/discord/src/lib.rs`

**Step 1: Write the failing test**

Add a unit test proving Gateway DM traffic still applies pairing checks and does not use the current bypass behavior.

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path channels-src/discord/Cargo.toml tests::test_gateway_dm_pairing_behavior_matches_webhook_dm -- --exact`

Expected: FAIL because Gateway DM currently bypasses pairing.

**Step 3: Write minimal implementation**

Change the permission/pairing decision helper so Gateway DMs are treated like webhook DMs for pairing purposes, while non-DM Gateway traffic stays unchanged.

**Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path channels-src/discord/Cargo.toml tests::test_gateway_dm_pairing_behavior_matches_webhook_dm -- --exact`

Expected: PASS.

---

### Task 2: Add a Gateway-safe pairing reply path

**Files:**
- Modify: `channels-src/discord/src/lib.rs`

**Step 1: Write the failing test**

Add a unit test for pairing reply routing so webhook interactions still use followup webhooks, but Gateway DM metadata routes pairing replies to `POST /channels/{channel_id}/messages`.

**Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path channels-src/discord/Cargo.toml tests::test_pairing_reply_route_uses_channel_messages_for_gateway_metadata -- --exact`

Expected: FAIL because pairing replies are currently interaction-webhook-only.

**Step 3: Write minimal implementation**

Refactor pairing reply sending to share the existing response route logic so Gateway DM pairing rejections can reply in-channel.

**Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path channels-src/discord/Cargo.toml tests::test_pairing_reply_route_uses_channel_messages_for_gateway_metadata -- --exact`

Expected: PASS.

---

### Task 3: Update docs and verify focused coverage

**Files:**
- Modify: `channels-src/discord/README.md`
- Modify: `FEATURE_PARITY.md`

**Step 1: Update behavior docs**

Document that Gateway DMs now follow pairing policy just like webhook DMs, while guild Gateway traffic remains unaffected.

**Step 2: Run focused verification**

Run: `cargo test --manifest-path channels-src/discord/Cargo.toml`

Expected: PASS.
