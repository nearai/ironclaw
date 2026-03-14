# Discord Gateway Presence Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Show Discord bot presence as `dnd` before pairing approval and `online` after pairing approval in Gateway mode.

**Architecture:** Presence is a host-managed websocket concern because the wrapper owns the active Gateway socket. The wrapper should derive a Discord presence payload from persisted pairing/owner state and send `OP 3` updates when the websocket connects and whenever relevant state changes after a poll cycle.

**Tech Stack:** Rust, tokio, tokio-tungstenite, Discord Gateway protocol, existing channel workspace + pairing store APIs

---

### Task 1: Add failing tests for presence derivation

**Files:**
- Modify: `src/channels/wasm/wrapper.rs`

**Step 1: Write the failing tests**

Add focused unit tests that verify:
- unpaired Gateway Discord channel maps to `dnd`
- paired/approved Gateway Discord channel maps to `online`
- `owner_id` counts as approved access and maps to `online`

**Step 2: Run test to verify it fails**

Run: `cargo test --lib channels::wasm::wrapper::tests::test_discord_gateway_presence_ -- --nocapture`

Expected: FAIL because presence derivation helpers do not exist yet.

**Step 3: Write minimal implementation**

Add helper(s) that compute Discord Gateway presence JSON from existing persisted state.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib channels::wasm::wrapper::tests::test_discord_gateway_presence_ -- --nocapture`

Expected: PASS.

---

### Task 2: Send presence updates on connect and after relevant state changes

**Files:**
- Modify: `src/channels/wasm/wrapper.rs`

**Step 1: Write the failing test**

Add a targeted helper test showing the wrapper can build an `OP 3` presence update payload with the expected `status` field.

**Step 2: Run test to verify it fails**

Run: `cargo test --lib channels::wasm::wrapper::tests::test_build_discord_gateway_presence_update_ -- --nocapture`

Expected: FAIL because the payload builder/send path does not exist yet.

**Step 3: Write minimal implementation**

Send a presence update after websocket identify/connect, and send another one after websocket-triggered poll processing so pairing-state changes can flip `dnd -> online`.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib channels::wasm::wrapper::tests::test_build_discord_gateway_presence_update_ -- --nocapture`

Expected: PASS.

---

### Task 3: Verify wrapper coverage and document behavior

**Files:**
- Modify: `channels-src/discord/README.md`

**Step 1: Update behavior docs**

Document the Gateway presence behavior: `dnd` before pairing approval, `online` after approval or `owner_id` gating.

**Step 2: Run focused verification**

Run: `cargo test --lib channels::wasm::wrapper::tests`

Expected: PASS.
