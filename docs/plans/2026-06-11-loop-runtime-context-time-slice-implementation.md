# Loop Runtime Context (Time Slice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render loop-start date/time (UTC + optional user timezone) as a distinct fingerprinted `runtime` section in the Reborn loop prompt bundle, through the seam the full #4149 plan extends later.

**Architecture:** New `LoopRuntimeContext` type with its own render function; new optional field on `InstructionBundleRequest` rendered by `InstructionBundleBuilder` after identity, before snippets/safety/surface; `HostManagedLoopPromptPort` attaches it; `loop_driver_host.rs` stamps wall-clock once at loop spawn. Spec: `docs/plans/2026-06-11-loop-runtime-context-time-slice.md`.

**Tech Stack:** Rust, chrono + chrono-tz (workspace deps, see `crates/ironclaw_host_runtime/src/first_party_tools/time.rs` for the existing tz pattern).

**Wave structure for parallel execution:**

- Wave 1 (parallel): Task 1 (core module, self-contained) ∥ Task 2 (read-only recon)
- Wave 2: Task 3 (builder section) then Task 4 (prompt port) — same agent, sequential
- Wave 3: Task 5 (Reborn wiring + caller-path test, consumes Task 2 recon)
- Wave 4: Task 6 (quality gate)

---

## Task 1: `LoopRuntimeContext` type + renderer

**Files:**
- Create: `crates/ironclaw_turns/src/run_profile/runtime_context.rs`
- Modify: `crates/ironclaw_turns/src/run_profile/mod.rs` (add `pub mod runtime_context;` + re-export, matching how sibling modules are declared)
- Modify: `crates/ironclaw_turns/Cargo.toml` (add `chrono = { workspace = true }` and `chrono-tz = { workspace = true }` if not already present)

- [ ] **Step 1: Write failing tests** (in `runtime_context.rs` `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn stamp() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 6, 11, 21, 32, 47).unwrap()
    }

    #[test]
    fn renders_utc_and_local_when_timezone_known() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: Some(chrono_tz::America::Los_Angeles),
        };
        let text = ctx.render_model_content();
        assert!(text.contains("2026-06-11T21:32Z"), "minute-truncated UTC: {text}");
        assert!(text.contains("14:32 Thu"), "local time + weekday: {text}");
        assert!(text.contains("America/Los_Angeles"), "{text}");
        assert!(text.contains("time capability"), "{text}");
        assert!(!text.contains(":47"), "seconds must be truncated: {text}");
    }

    #[test]
    fn renders_unknown_timezone_fallback() {
        let ctx = LoopRuntimeContext {
            loop_started_at_utc: stamp(),
            user_timezone: None,
        };
        let text = ctx.render_model_content();
        assert!(text.contains("2026-06-11T21:32Z"), "{text}");
        assert!(text.contains("timezone is unknown"), "{text}");
        assert!(text.contains("ask the user"), "{text}");
    }

    // [Amendment: `invalid_timezone_falls_back_to_unknown` was removed when
    // `user_timezone` became `Option<chrono_tz::Tz>`; invalid IANA names are
    // unrepresentable by construction and are rejected at the producer boundary.]
}
```

Note: 2026-06-11 is a Thursday; 21:32 UTC = 14:32 PDT.

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_turns runtime_context`
Expected: compile error (`LoopRuntimeContext` not defined).

- [ ] **Step 3: Implement**

```rust
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Model-visible runtime context for one loop execution.
///
/// First slice carries only time. The #4149 plan adds capability posture,
/// scoped-path semantics, and subagent narrowing as additional fields
/// rendered into the same prompt section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRuntimeContext {
    /// Instant this loop execution started. Rendered at minute precision.
    pub loop_started_at_utc: DateTime<Utc>,
    /// Validated IANA timezone for the user (e.g. `chrono_tz::America::Los_Angeles`),
    /// when known. Never a guessed host timezone.
    pub user_timezone: Option<chrono_tz::Tz>,
}

impl LoopRuntimeContext {
    pub fn render_model_content(&self) -> String {
        let utc = self.loop_started_at_utc.format("%Y-%m-%dT%H:%MZ");
        let local = self
            .user_timezone
            .map(|tz| {
                let local = self.loop_started_at_utc.with_timezone(&tz);
                format!("{} ({}, {})", utc, local.format("%H:%M %a"), tz.name())
            });
        match local {
            Some(stamped) => format!(
                "Current date/time at loop start: {stamped}. This was captured when \
                 this loop started; for the precise current time use the time \
                 capability if it is visible."
            ),
            None => format!(
                "Current date/time at loop start: {utc}. The user's timezone is \
                 unknown - if local time matters, ask the user or use the time \
                 capability if it is visible."
            ),
        }
    }
}
```

Adjust `mod.rs` declaration/re-export to match sibling style (look at how `instruction_bundle` is declared there).

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p ironclaw_turns runtime_context`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_turns
git commit -m "feat(turns): add LoopRuntimeContext type with time rendering"
```

---

### Task 2: Recon (read-only, parallel with Task 1)

**Files:** none modified. Output = findings notes returned as text.

- [ ] **Step 1:** In `crates/ironclaw_reborn/src/loop_driver_host.rs`, locate where the loop/prompt port is constructed for a run (near `spawn` ~line 626, `run` ~line 700, `run_context` ~line 1712). Report exact file:line where `HostManagedLoopPromptPort` (or its wrapper) is built and which builder methods (`with_safety_context`, `with_current_surface…`) are chained, so `with_runtime_context` can be added at the same site.
- [ ] **Step 2:** Report whether any existing user-timezone source is reachable from that construction site (search `timezone`, `Tz`, `user_tz` in `ironclaw_reborn`, `ironclaw_reborn_composition`, run profile types). Expected answer: none → wire `None`.
- [ ] **Step 3:** In `crates/ironclaw_reborn/src/model_gateway.rs` tests (and `crates/ironclaw_turns/tests/agent_loop_host_contract.rs`), identify the existing test that proves prompt-bundle content reaches the final model request through the caller path; report its name and file:line so Task 5 can mirror it.

---

### Task 3: Builder section in `instruction_bundle.rs`

**Files:**
- Modify: `crates/ironclaw_turns/src/run_profile/instruction_bundle.rs` (request struct ~line 101, `build` ~line 202, section helpers ~line 507)
- Test: same file or `crates/ironclaw_turns/tests/agent_loop_host_contract.rs` (follow where existing bundle tests live — contract tests are at `agent_loop_host_contract.rs:341`)

- [ ] **Step 1: Write failing tests** (mirror the style of `instruction_bundle_builder_orders_sections_and_rebuilds_deterministically`, `agent_loop_host_contract.rs:341`)

Test 1 — section renders, ordered after identity and before instruction snippets, deterministic:

```rust
// Pseudostructure — reuse the existing test helpers in that file for
// building a request; the assertions are the substance:
let mut request = /* existing helper that builds a populated request */;
request.runtime_context = Some(LoopRuntimeContext {
    loop_started_at_utc: chrono::Utc.with_ymd_and_hms(2026, 6, 11, 21, 32, 0).unwrap(),
    user_timezone: None,
});
let bundle_a = builder.build(request.clone()).unwrap();
let bundle_b = builder.build(request).unwrap();
assert_eq!(bundle_a.fingerprint, bundle_b.fingerprint);
let runtime_idx = bundle_a.materialized_messages.iter()
    .position(|m| m.content_ref.as_str().starts_with("msg:runtime."))
    .expect("runtime section present");
let identity_idx = /* index of last identity message */;
let snippet_idx = /* index of first instruction-snippet message */;
assert!(identity_idx < runtime_idx && runtime_idx < snippet_idx);
assert!(bundle_a.materialized_messages[runtime_idx].model_content
    .contains("Current date/time at loop start: 2026-06-11T21:32Z"));
assert_eq!(bundle_a.materialized_messages[runtime_idx].role, "system");
```

Test 2 — `None` is byte-identical to today:

```rust
let request_without = /* request with runtime_context: None */;
let bundle = builder.build(request_without).unwrap();
assert!(bundle.materialized_messages.iter()
    .all(|m| !m.content_ref.as_str().starts_with("msg:runtime.")));
// and fingerprint equals a build of the same request before the field existed
// (i.e. adding the None field must not feed any fingerprint fields)
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_turns --test agent_loop_host_contract runtime`
Expected: compile error (no `runtime_context` field).

- [ ] **Step 3: Implement**

Add to `InstructionBundleRequest`:

```rust
pub runtime_context: Option<LoopRuntimeContext>,
```

(import from the `runtime_context` module; update every existing literal construction of `InstructionBundleRequest` in src + tests with `runtime_context: None`).

Add section helper, following `push_safety_context` (line 507) exactly:

```rust
fn push_runtime_context(
    messages: &mut Vec<LoopModelMessage>,
    materialized_messages: &mut Vec<InstructionBundleMaterializedMessage>,
    fingerprint: &mut Sha256,
    runtime_context: LoopRuntimeContext,
    synthetic_refs: &mut SyntheticMessageRefRegistry,
) -> Result<(), AgentLoopHostError> {
    let model_content = validate_model_safe_text(
        runtime_context.render_model_content(),
        "runtime context",
    )?;
    let content_ref =
        synthetic_message_ref("runtime", "loop-start", &model_content, 0, synthetic_refs)?;
    feed_field(fingerprint, b"section", b"runtime");
    feed_field(fingerprint, b"ref", content_ref.as_str().as_bytes());
    feed_field(fingerprint, b"content", model_content.as_bytes());
    materialized_messages.push(InstructionBundleMaterializedMessage {
        role: "system".to_string(),
        content_ref: content_ref.clone(),
        model_content,
    });
    messages.push(LoopModelMessage {
        role: "system".to_string(),
        content_ref,
    });
    Ok(())
}
```

In `InstructionBundleBuilder::build`: call `push_runtime_context` for `Some(runtime_context)` immediately after the identity messages are pushed and before instruction snippets. Read `build` first to find those two points; insert between them. `None` → no call, no fingerprint fields.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p ironclaw_turns`
Expected: all pass, including new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_turns
git commit -m "feat(turns): render runtime context as distinct instruction bundle section"
```

---

### Task 4: `HostManagedLoopPromptPort::with_runtime_context`

**Files:**
- Modify: `crates/ironclaw_turns/src/run_profile/prompt.rs` (struct ~line 35, builder methods ~lines 70–137, request construction inside `build_prompt_bundle`/`instruction_builder` ~lines 220–410)
- Test: wherever existing `HostManagedLoopPromptPort` tests live (search `HostManagedLoopPromptPort` in `crates/ironclaw_turns/tests/` and `src/run_profile/prompt.rs` tests module)

- [ ] **Step 1: Write failing test** — construct the port with `.with_runtime_context(ctx)` using the existing port test harness, build a prompt bundle, assert one materialized message's `content_ref` starts with `msg:runtime.` and content contains `Current date/time at loop start:`. Also assert a port built without the call produces no such message.

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_turns prompt`
Expected: compile error (no `with_runtime_context`).

- [ ] **Step 3: Implement**

```rust
// field on HostManagedLoopPromptPort:
runtime_context: Option<LoopRuntimeContext>,

// initialize to None in new(); builder method next to with_safety_context:
pub fn with_runtime_context(mut self, runtime_context: LoopRuntimeContext) -> Self {
    self.runtime_context = Some(runtime_context);
    self
}
```

Where the port constructs `InstructionBundleRequest`, set `runtime_context: self.runtime_context.clone()` instead of `None`.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p ironclaw_turns`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_turns
git commit -m "feat(turns): attach runtime context through host prompt port"
```

---

### Task 5: Reborn wiring + caller-path test

**Files (exact lines come from Task 2 recon):**
- Modify: `crates/ironclaw_reborn/src/loop_driver_host.rs` (prompt-port construction site)
- Test: `crates/ironclaw_reborn/src/model_gateway.rs` tests (mirror the recon-identified caller-path test)

- [ ] **Step 1: Write failing caller-path test** — drive a real loop turn through the model gateway test harness and assert the final model request's messages include a system message containing `Current date/time at loop start:`. This is the test-through-the-caller requirement (`.claude/rules/testing.md`).

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_reborn model_gateway`
Expected: new test FAILS (no runtime section yet — wiring absent).

- [ ] **Step 3: Implement** — at the prompt-port construction site, stamp once per loop spawn:

```rust
.with_runtime_context(LoopRuntimeContext {
    loop_started_at_utc: chrono::Utc::now(),
    user_timezone: None, // no user-tz source yet; follow-up wires settings
})
```

This is the only place wall-clock is read. Resume-after-pause re-stamps (loop start = this execution). If `chrono` is not yet a direct dep of `ironclaw_reborn`, add `chrono = { workspace = true }`.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p ironclaw_reborn`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_reborn
git commit -m "feat(reborn): stamp loop-start runtime context into prompt bundles"
```

---

### Task 6: Quality gate

- [ ] **Step 1:** `cargo fmt`
- [ ] **Step 2:** `cargo clippy --all --benches --tests --examples --all-features` — zero warnings
- [ ] **Step 3:** `cargo test -p ironclaw_turns -p ironclaw_reborn -p ironclaw_loop_support -p ironclaw_agent_loop`
- [ ] **Step 4:** Commit any fmt/clippy fixups:

```bash
git add -A && git commit -m "chore: fmt and clippy fixups for runtime context slice"
```
