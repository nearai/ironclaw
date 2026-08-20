# Recon: completing subagent support with background subagents enabled

/shape recon output (2026-08-19, branch `subagent`). Three read-only lanes
(trace / patterns / constraints); all file:line claims spot-checked against
live code. The canonical design this work executes is
`thread-harness-design.md` in this directory — this file is a map of what has
landed vs. what remains, not a new design.

## Map

### What has landed (PR1 of the design's 6-PR staging — blocking mode, switched off)

- Spawn entry: `builtin.spawn_subagent` manifest in
  `crates/kernel/ironclaw_host_runtime/src/first_party_tools/spawn_subagent.rs:4`
  (registered `first_party_tools/mod.rs:248`); real behavior lives in the
  loop-host decorator `SubagentSpawnCapabilityPort`
  (`crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs:381`, ~1600
  lines): schema `:62` (only `subagent_type`/`task`/`handoff` — no mode field
  advertised), admission caps (fanout ≤4 `:669`, depth ≤1 `:44`, descendant
  cap, actor/scope checks `:727-774`), `finish_spawn` `:889` (placeholder
  result → child thread with `SubagentThreadMetadata` → durable goal input →
  `AwaitedChildSetRecord` → `submit_child_run` with process dependency →
  parent parks on an await-dependent-run gate `:1101`), compensation rollback
  `:1112`. Model-facing description:
  `crates/loop/ironclaw_loop_host/prompts/spawn_subagent_description.md`
  (blocking-only wording).
- Flavors: four static kinds (general/explorer/coder/planner), all
  `allow_nesting: false` —
  `crates/loop/ironclaw_turn_runner/src/subagent/flavors.rs:106-134`;
  per-flavor directions in `subagent/directions/*.md`.
- Result return (blocking): `AwaitEdgeResolver`
  (`crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs`) is
  a `TurnCommittedEventObserver` (`:1881`); child terminal →
  `settle_and_maybe_drain` `:603` → `drain_settled_group` `:673` (waits for
  all siblings under the shared gate ref) → `update_parent_result_reference`
  `:449` (framed, byte-capped, untrusted-wrapped via
  `subagent/untrusted_text.rs`) → `resume_parent` `:489`
  (`ResumeTurnPrecondition::BlockedDependentRunGate`) exactly once → close
  edges. Edge store is a CAS'd projection over the kernel process-dependency
  journal (`await_edge/store.rs`), states Open → Settled → Drained/Abandoned
  (`await_edge/mod.rs:18`). Boot/lazy recovery:
  `await_edge/boot_recovery.rs` (`ScopeRecoveryDriver`), edge reconstruction
  from child-thread metadata `resolver.rs:285`.
- Production wiring: `crates/app/ironclaw_composition/src/runtime.rs:104-106,
  268-269, 3881`; DI seam (loop_host cannot depend on turn_runner):
  `crates/loop/ironclaw_loop_host/src/await_edge_port.rs`.

### The two off-switches

1. `default_disabled_capability_ids()`
   (`crates/loop/ironclaw_turn_runner/src/runtime.rs:277-282`, applied
   `:801-806`, `TEMP(disable-spawn-subagents)`) deny-filters
   `builtin.spawn_subagent` from every shipped surface. This is the sole
   on/off gate by standing ruling (thread-harness-design.md §Terminology —
   no `subagent.v2_enabled` flag, no cargo feature).
2. `background_subagents_disabled()` (`subagent_spawn_port.rs:1500`),
   enforced in `TryFrom<SpawnSubagentWireArgs>` `:222-227` for both
   `mode: "background"` and legacy `run_in_background: true` — and
   `finish_spawn` hard-codes `SpawnSubagentMode::Blocking` at `:908`.
   `SpawnSubagentMode::Background` exists as a type (`:183`) and is carried
   durably on `AwaitedChildSetRecord`/`SubagentThreadMetadata`, but is never
   constructed in production.

### What is missing (= the design's PR2, plus staged follow-ons)

- **Background completion delivery.** Blocking works because the parent is
  parked; background has no consumer. `PostCapabilityStage::drain_settled()`
  (`crates/loop/ironclaw_agent_loop/src/executor/post_capability.rs:34-39`)
  is a stub returning `Vec::new()`; the `LoopBackgroundChildPort` it names
  was never built and the design supersedes it — delivery is
  `activate(parent_thread, input, provenance=System)` with three healing
  triggers (settle-time activate, run-start sweep on every Continue, boot
  pass; §8.2, incl. required test
  `settled_edge_threadbusy_is_healed_by_run_start_and_boot_pass`).
- **`ActivationProvenance` / `subagent_activation_provenance`** on
  `TurnRunRecord` — zero hits in the tree. Type belongs in
  `ironclaw_host_api::turn` per `ironclaw_turns/AGENTS.md:20`.
- **System-wake streak cap** (16 consecutive System activations, derived by
  `LIMIT K` query, no stored counter; §8.3) — not present.
- **Batched multi-edge drain** (one snapshot read + one CAS write, O(E+M)) —
  the shipped per-member loop is documented as blocking-only-adequate
  (`resolver.rs:709-717`).
- **Gate-propagation escalation walk** (§9, pulled into PR2 as a prod-enable
  gate): descendant `BlockedApproval`/`BlockedAuth` must bubble to the tree
  root's originating surface via
  `ironclaw_outbound::resolve_run_notification_context`; resolution accepted
  only from an owner-authenticated human surface (`caller.user_id ==
  root.owner_user_id`), never any LLM. Named integration gap:
  `deliver_triggered_run` (`ironclaw_composition/src/slack_delivery.rs:2033`)
  watches only the root run's status.
- **Safety scan on the drain write** (second hard prod-enable gate,
  design §7 round-5): one synchronous `SafetyLayer::sanitize_tool_output`
  (`crates/substrates/ironclaw_safety/src/lib.rs:98`) or
  `scan_inbound_for_secrets` (`:193`) call at
  `update_parent_result_reference` before commit; `ironclaw_turn_runner`
  already depends on `ironclaw_safety`. One seam covers both modes.
- **Observe/control surfaces** (staged after prod-enable): PR3
  `subagent_inspect` (metadata-only), PR4 `subagent_extend` (+ ParentAgent
  streak cap of 8, `ThreadReserved`/human-priority reservation), PR5 WebUI
  (`GET .../threads/{id}/children` + `ThreadTree` sidebar + raw-vs-framed
  display rule), PR6 `subagent_cancel` (security review gate). WebUI has
  zero subagent awareness today (i18n strings only); the one existing
  "child work under parent" projection rule to ride is `nested_dispatch`
  re-parenting in
  `crates/events/ironclaw_event_projections/src/runtime_projection.rs:197`.
- **No cascade teardown**: `abandon_awaited_child` is rollback-only; for
  background edges, parent completion with the edge open is the normal
  delivery case, not abandonment (§2).

### Patterns to copy (nearest existing implementations)

- New-turn-from-background-worker (the shape `activate()` generalizes):
  `crates/app/ironclaw_composition/src/automation/trigger_poller_trusted_submit.rs:148`
  + `conversation_turn_submitter.rs:28,63` (incl. `ThreadBusy` → retryable
  classification at `:118-121`). `TurnCoordinator::submit_turn` +
  `TurnError::ThreadBusy` (`crates/kernel/ironclaw_turns/src/status.rs:269`)
  is the real seam; the design's `activate()` does not exist yet.
- Wake-notifier (secondary nudge only, cannot create work):
  `TurnRunWakeNotifier` (`crates/kernel/ironclaw_turns/src/coordinator.rs:102`,
  best-effort notify `:475`; scheduler impl
  `crates/loop/ironclaw_turn_runner/src/turn_scheduler.rs:383-388`).
- In-flight injection into a running parent (distinct third mechanism —
  use at most one of the three per job): `HostInputQueue` /
  `HostInputEnqueuePort`
  (`crates/loop/ironclaw_loop_host/src/input_queue.rs:45,144`).
- Multi-verb built-in tool family (template for inspect/extend/cancel):
  `crates/kernel/ironclaw_host_runtime/src/first_party_tools/trigger_management.rs:40-88,151-166,242,282,340`.

### Binding constraints (beyond the design doc itself)

- Root `AGENTS.md:47` — spawn creates/wires child runs only; everything else
  goes through the existing runner/driver/executor path. `AGENTS.md:49` +
  arch test `reborn_dependency_boundaries.rs:1400,1657` — background wake
  must not mint `TrustedInboundTurnRequest` or touch trusted trigger
  submitters (note: the `:1657` scan roots do not currently cover
  `subagent_spawn_port.rs` — extend the scan-root list if a submitter-shaped
  path is added).
- No new crate, no cargo feature, no stored counters, no new tables
  (design §12 non-goals + `.claude/rules/cargo-features.md`).
- Closed await-edges are deleted — the one carved-out exception to
  "LLM data is never deleted" (§2; preserve the carve-out reasoning in an
  implementation comment). Fixed close order: state CAS → reservation
  release → prune → `delete_if_version` with the token from the Released CAS.
- Dual-backend parity (Postgres + libSQL) via shared conformance suites;
  every RMW through the shared bounded CAS helper
  (`.claude/rules/database.md:54-82`).
- Traits in `ironclaw_loop_host`, impls in `ironclaw_turn_runner`
  (dependency-inversion category of `type-placement.md`); never append to
  the 4,758-line `completion_observer.rs`; new files aim <800 lines.
- `subagent_spawn_port.rs` ratchets: frozen at exactly 3 `test-support`
  methods (`reborn_struct_test_support_ratchet.rs:378`); on the
  provider-name allowlist (`reborn_dependency_boundaries.rs:2179`).
- Child authority: empty grant/lease set at start; surface allowlist is a
  ceiling, not authority; child re-acquires leases via its own approval gate.

### Tests

- Today: one integration file
  (`tests/integration/subagent_await_edge.rs:22,106`); five `#[ignore]`d e2e
  cases (`tests/reborn_subagent_spawn_e2e.rs:25,90,149,203,313` — one of
  which pins the background *rejection* and flips meaning);
  active tests asserting the capability stays off
  (`tests/integration/tool_call.rs:756,797`, `tool_disclosure.rs:168`,
  `crates/app/ironclaw_composition/tests/service_factory.rs:386,413`);
  zero Python E2E. Declared gaps at `tests/CLAUDE.md:519` (no subagent group
  scenario) and the proactive/background row (#6369).
- Prod enable = clear the deny filter after PR2 + un-ignore e2e + matrix
  green + the two hard gates (escalation walk, drain safety scan). Design
  names integration-tier for drain idempotency/batching, ThreadBusy healing,
  the scan gate, the gate walk + non-owner rejection; crate-tier only for
  the streak-cap windowing, release idempotency/ordering, flavor overrides,
  `delete_if_version` parity. Always run
  `cargo test -p ironclaw_architecture_tests` and `scripts/reborn-e2e-rust.sh`.

## Briefing

A subagent here is not a separate engine: it is an ordinary child turn-run on
its own thread, spawned by the built-in `spawn_subagent` tool. The spawn
path, the durable parent↔child "await edge" bookkeeping, child-output
framing, crash recovery, and blocking-mode delivery (parent parks on a gate,
resolver wakes it when all children finish) are all fully built — and then
deliberately switched off in two places: the tool is deny-filtered out of
every production surface, and any request for background mode is rejected at
the argument decoder.

"Complete support with background fully enabled" is therefore not a design
problem — an accepted, round-8-hardened design (`thread-harness-design.md`)
already specifies it, and PR1 of its six-PR staging has landed on this
branch. The remaining work is the design's PR2: let a child run *without*
parking the parent, and when the child finishes, durably hand the result to
a parent that may be mid-run, idle, or finished — by writing the framed
result into the parent transcript and "activating" the parent thread as a
System-provenance turn, with a run-start sweep and a boot pass healing any
missed wake, and a derived 16-wake streak cap stopping a parent from looping
autonomously forever. Two safety gates must land before the deny filter is
cleared: a blocked child's approval must escalate to the tree root's human
(otherwise a background child stuck on approval is invisible), and the
child→parent result write must pass the safety scan layer. After PR2 the
design stages observe/control surfaces: an inspect tool, an extend-runtime
tool, a WebUI child-thread tree, and cancel.
