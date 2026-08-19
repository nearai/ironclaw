# Shape: complete subagent support (PR2–PR6), enable last

Decision record (2026-08-19, /shape gate). Companion recon:
`research-background-enable.md`. Canonical design (tiebreaker for every
mechanism below): `thread-harness-design.md` — this file only fixes slice
order, file placements, and signatures so the implementation plan is
transcription plus judgment.

## Decision

- **Scope:** ship the design's PR2 through PR6 — background delivery,
  inspect, extend, WebUI child-tree, cancel.
- **Extend-vs-fork verdict:** extend the landed PR1 path everywhere
  (`SubagentSpawnCapabilityPort`, `AwaitEdgeResolver`,
  `subagent/await_edge/`). Nothing forks; no new crate, no cargo feature,
  no stored counters (design standing rulings).
- **Deviation from the design's staging, with reason:** the design
  prod-enables (clears `builtin.spawn_subagent` from
  `disabled_capability_ids`) after PR2. We enable **after PR6 instead** —
  subagents are complex enough that the observe/steer/cancel surfaces should
  exist before real users get the tool. Strictly more conservative; costs
  one line to change our mind. Everything else follows the design's staging
  verbatim, including both hard prod-enable gates (drain safety scan, gate
  escalation walk) landing with PR2.
- **Testing is not deferred to the end:** the five `#[ignore]`d e2e tests
  come alive at PR2 via the harness's own capability enablement
  (`tests/support/reborn_parity_qa/binary_e2e.rs:861` clears
  `disabled_capability_ids`), independent of production enablement. Only
  the final slice touches production defaults.
- **What gets deleted:** `background_subagents_disabled()` and its codec
  rejections; the `drain_settled` stub body + dead `LoopBackgroundChildPort`
  doc reference; the hard-coded `SpawnSubagentMode::Blocking` in
  `finish_spawn`; at the end, the deny-filter entry and the
  "capability stays off" test assertions.
- **Deletion-first check:** ran — the change is mostly *unblocking* landed
  code; the only large additions the design allows are the ones it names
  (activate primitive, escalation walk, WebUI tree).

## Slices

Each slice = one reviewable PR. Labels: [B] behavioral, [S] structural.
Design task IDs (P2.x…) in parentheses; the design section is the spec for
each.

### Slice 1 [B] — activation provenance + `activate()` primitive (P2.4 prereq)

- `ActivationProvenance { Human, ParentAgent, System }` — new enum in
  `crates/contracts/ironclaw_host_api/src/turn.rs` (turn vocabulary is
  host_api-owned per `crates/kernel/ironclaw_turns/AGENTS.md`).
- `TurnRunRecord.subagent_activation_provenance: Option<ActivationProvenance>`
  — additive field, set once at run creation, immutable
  (`crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs`, beside
  `parent_run_id`/`subagent_depth` at `:181-185`; mirror on `request.rs`).
- `activate(thread, input, provenance)` — the single re-activation
  primitive (design §1). Home: `crates/kernel/ironclaw_turns/src/coordinator.rs`
  beside `submit_turn` (TODO exact signature: takes typed thread id +
  durable input submission + provenance; returns `ThreadBusy` when a run is
  live — reuse `TurnError::ThreadBusy`, `status.rs:269`). It must NOT mint
  `TrustedInboundTurnRequest` or touch trusted trigger submitters
  (root `AGENTS.md:49`; extend the scan roots of
  `reborn_dependency_boundaries.rs:1657` if any submitter-shaped type is
  added).
- Streak-cap admission inside `activate()` (design §8.3, pulled into PR2):
  `SYSTEM_WAKE_STREAK_CAP = 16`, derived `LIMIT K` query over run records
  with `ParentAgent` excluded from the fetch; `Human` resets. Constant
  independently named (never merged with the descendant cap 16 or iteration
  limit 16). §6's ParentAgent cap of 8 ships later (Slice 7) on the same
  field.
- Tests: crate-tier windowing rows (§8.3's four assertions a–d).

### Slice 2 [B] — background mode accepted + delivery (P2.4)

- Codec: delete both rejections in `TryFrom<SpawnSubagentWireArgs>`
  (`subagent_spawn_port.rs:218-227`) and `background_subagents_disabled()`
  (`:1500`); advertise `mode` in `build_spawn_subagent_parameters_schema`
  (`:62`, enum `["blocking","background"]`, default blocking); thread
  `args.mode` through `finish_spawn` instead of the hard-coded `Blocking`
  (`:908`); background spawns return an immediate spawn-result payload
  (`spawn_result.rs` `SubagentSpawnMode::Background`) instead of
  `await_dependent_run`.
- Update `prompts/spawn_subagent_description.md` (currently blocking-only
  wording).
- Delivery, live parent: implement `PostCapabilityStage::drain_settled`
  (`crates/loop/ironclaw_agent_loop/src/executor/post_capability.rs:34-39`)
  wired to `AwaitEdgeResolver` — TODO seam: a small trait in
  `ironclaw_agent_loop` (or reuse the existing loop-host port surface),
  implemented on the turn_runner side, same dependency-inversion category
  as `AwaitEdgeWriter`/`AwaitEdgeSettler` (`await_edge_port.rs`). No
  `LoopBackgroundChildPort`.
- Delivery, parked/completed parent: resolver's settle path calls
  `activate(parent_thread, input, System)`; `ThreadBusy` benign no-op;
  one attempt per settled edge (the `settled` state is the dedupe).
- Batched drain (§8): multi-edge drain = one thread-snapshot read + one CAS
  write across all settled `(result_ref, safe_summary)` pairs — extend
  `drain_settled_group`'s tail (`resolver.rs:673`, the `:715` P2.4 comment
  marks the spot); O(E+M).
- Three-trigger retry set (§8.2): settle-time activate, run-start sweep
  (drain_settled runs on every `Continue`), boot pass (drains at resolver
  layer, no activate, never streak-capped).
- Background edges: parent-run completion with edge open is normal
  delivery, never abandonment (§2 mode-scoping).
- Tests (integration-tier, `tests/integration/`):
  `settled_edge_threadbusy_is_healed_by_run_start_and_boot_pass` (both
  scenarios + parent-completed precondition, §8.2); drain idempotency
  crash-replay (§8.1); batched-drain write-count seam (§8 required test).

### Slice 3 [B] — drain safety scan (prod-enable gate 1, P2.4/P2.5)

- One synchronous scan call at the single drain write site
  `update_parent_result_reference` (`resolver.rs:449`) before commit:
  `SafetyLayer::sanitize_tool_output`
  (`crates/substrates/ironclaw_safety/src/lib.rs:98`) or
  `scan_inbound_for_secrets` (`:193`) — whichever the platform wiring
  settles on; dependency already present. Covers blocking and background
  (mode-agnostic call site).
- Test (integration-tier): crafted child drain content tripping the
  leak-detector/injection patterns is redacted/rejected, not passed
  verbatim (§7 required test).

### Slice 4 [B] — gate escalation walk (prod-enable gate 2, P2.5/P2.6)

- Any descendant `BlockedApproval`/`BlockedAuth` at any depth bubbles to
  the **tree root's** originating surface via
  `ironclaw_outbound::resolve_run_notification_context` (no second
  fallback) — design §9.
- Resolution accepted from any owner-authenticated **human** surface, never
  any LLM; surface layer checks `caller.user_id == root.owner_user_id`
  (§9.2).
- Fix the named integration gap: `deliver_triggered_run`
  (`crates/app/ironclaw_composition/src/slack_delivery.rs:2033`) watches
  only the root run's own status — extend to descendant gates.
- Tests (integration-tier): gate walk end-to-end + non-owner rejection.

### Slice 5 [B] — PR2 wrap-up: counters, operator command, e2e revival

- `ResolveReport` counters + `ironclaw subagent edges [--scope …]` operator
  command (§5.4; host-level trusted, cross-tenant like the rest of the CLI;
  reports open-reservation counts beside edge counts).
- Round-5 boot-recovery fairness (bounded pending queue + per-tenant
  in-flight cap ≤2 of 4) lands here with the `run_boot_recovery` process-
  start wiring (§4.3 round-8 staging split) + its P1.9-extension test.
- Un-ignore all of `tests/reborn_subagent_spawn_e2e.rs`; rewrite
  `background_spawn_is_rejected_before_child_run_or_auth_invocation` (:149)
  into the background-accept + delivery scenario; suite runs with
  harness-side capability enablement. Update `tests/CLAUDE.md` rows
  (subagent group scenario, proactive/background gap #6369).

### Slice 6 [B] — `subagent_inspect` + per-flavor config (PR3: P3.2, P3.3)

- New capability ids under one multi-verb `FirstPartyCapabilityHandler`,
  template `first_party_tools/trigger_management.rs:40-88,151-166,242`
  (metadata only: status, gate state, byte counts — never raw transcript;
  §7 PR3 scope note).
- Per-flavor budget plumbing (§10c, iteration limit per flavor) and
  per-flavor model override (§10d) through
  `flavors.rs`/`material_for_run`; crate-tier tests.

### Slice 7 [B] — `subagent_extend` + human priority (PR4: P4.2, P4.3, P4.4)

- `subagent_extend` = `activate(child_thread, input, ParentAgent)` +
  consent-to-wake (own direct live child only) + §6 budget (8 consecutive
  `ParentAgent`, derived `LIMIT 8` query, `System` excluded) — windowing
  logic only; the field landed in Slice 1.
- Reservation re-claim at admission (extend on a full tree → the existing
  capacity error; the one re-claim path, §5.1).
- `human_waiting` reservation marker (§6a): owner-gated CAS'd marker file
  `{ owner: UserId, expires_at }`, 15-min lease
  (`HUMAN_RESERVATION_LEASE_TTL`), lazy expiry, no reaper; new
  `ThreadReserved` admission outcome treated like `ThreadBusy`.
- Tests: §6a's three integration cases; P4.2 crate rows; P4.4 full-tree
  rejection (integration).

### Slice 8 [B] — WebUI child tree (PR5a + PR5b)

- `GET /api/webchat/v2/threads/{thread_id}/children` — lineage projection
  over `TurnRunRecord.{parent_run_id, spawn_tree_root_run_id,
  subagent_depth}`, no new store; ride the `nested_dispatch` re-parenting
  shape (`runtime_projection.rs:197`). Route in `webui_v2/router.rs`.
- `ThreadTree` sidebar in `frontend/` + raw-vs-framed display rule (§11:
  raw child transcript is human-only) + interrupt & take over (P5.4 —
  console compose of run-cancel + `activate(Human)`, no new state).
- Tests: integration-tier endpoint test; frontend per its own tier.

### Slice 9 [B] — `subagent_cancel` (PR6) — **needs security review**

- Model-facing tool wrapping the run-cancel mechanism; drives child to
  terminal → slot release via the §5.5 tri-state; explicit tree teardown is
  the one legitimate open→abandoned path besides rollback (§2b).

### Slice 10 [B] — production enable (the deviation point)

- Remove the `builtin.spawn_subagent` entry from
  `default_disabled_capability_ids()`
  (`crates/loop/ironclaw_turn_runner/src/runtime.rs:277-282`) and the
  `TEMP(disable-spawn-subagents)` composition note (`:801-806`,
  `composition/src/runtime.rs:3892`).
- Flip the "capability stays off" assertions:
  `tests/integration/tool_call.rs:756,797`,
  `tests/integration/tool_disclosure.rs:168`,
  `crates/app/ironclaw_composition/tests/service_factory.rs:386,413`.
- Update `crates/Architecture.md:873-879` status prose and any guidance
  citing the deny-filter as current.
- Precondition checklist (design §7 + our deviation): Slices 1–9 merged,
  both backends green, `cargo test -p ironclaw_architecture_tests` green,
  `scripts/reborn-e2e-rust.sh` green, e2e suite un-ignored and green,
  scan + escalation tests green, PR6 security review done.

## Standing constraints for every slice (from recon — plan must carry these)

- CAS discipline: single-record RMW via the shared bounded CAS path;
  fixed close order state → release → prune → `delete_if_version` with the
  token from the caller's own last successful CAS; dual-backend parity
  suites (Postgres + libSQL + in-memory).
- Edge deletion is the sanctioned carve-out from "LLM data is never
  deleted" — keep §2's reasoning in an implementation comment at the
  delete site.
- Traits in `ironclaw_loop_host`/`ironclaw_agent_loop`, impls in
  `ironclaw_turn_runner`; never append to `completion_observer.rs`; new
  files < 800 lines; `subagent_spawn_port.rs` test-support ratchet frozen
  at 3.
- Child authority: empty grant/lease set; allowlist is a ceiling; child
  re-acquires leases via its own gates.
- No `info!`/`warn!` from background tasks (REPL rule) — `debug!` +
  counters per §5.4.
- Run `cargo test -p ironclaw_architecture_tests` whenever edges, layer
  keys, or pinned guidance files move.
