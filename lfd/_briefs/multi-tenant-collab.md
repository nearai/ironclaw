# LFD Brief: multi-tenant-collab — Multi-tenant cross-agent collaboration

**State**: foundation built (identity, projects ACL, thread scoping,
spawn_subagent) — this LFD targets the product layer: agents collaborating
across agent identities within a tenant, isolation preserved. **Bar**: 0.95
holdout AND zero isolation violations (acceptance requires both).
**Profile**: `cross_agent`.

## Outcome

Within a tenant: agent A delegates work to agent B (spawn or message
routing), results return with correct attribution; project-scoped memory and
threads are visible per ACL role; permissions attenuate through delegation
(B never exceeds A's grants); every cross-TENANT interaction fails closed.

## Spec sources

- `crates/ironclaw_agent_loop/` (spawn_subagent), `crates/ironclaw_projects/`
  (Owner>Editor>Viewer ACL), `crates/ironclaw_threads/` (scope tuple),
  `crates/ironclaw_reborn_identity/CONTRACT.md`
- `tests/reborn_subagent_spawn_e2e.rs`, `tests/reborn_*_scope_isolation_parity.rs`
- Memory note: capability surfaces fail toward MORE exposure only for
  capability bugs; identity/tenancy boundaries fail CLOSED — this LFD is
  the fail-closed kind (stale-test alignment lesson, PR #5108).

## Stage 0 inner suite

ALL `tests/reborn_*_scope_isolation_parity.rs` + subagent spawn e2e +
`ironclaw_projects`/`ironclaw_threads` crate tests. Green every cycle —
these are the isolation floor; weakening any of them is a violation.

## Eval themes (dev ~35 / holdout ~12)

1. Delegation round-trip (8): A spawns/delegates to B with a task → B's
   run executes under B's agent identity (state_pred on scope tuple),
   result returns to A's thread with attribution (required events:
   spawn, child completion, parent resume).
2. ACL views (6): project members with Owner/Editor/Viewer see exactly
   their slice (state queries per role; forbidden: Viewer write success).
3. Cross-tenant fail-closed (8): delegation/message/memory-read attempts
   targeting another tenant → typed denial, nothing persisted across the
   boundary (forbidden: any cross-tenant state/egress; required: denial
   events). These forbidden hits are the "isolation violations" acceptance
   gate — one hit on holdout fails the run regardless of aggregate.
4. Concurrency (4): two collaborations in-flight in one tenant → no
   cross-thread bleed (state_eq on both threads' isolated records).
5. Attribution (5): results/memory writes from B are attributed to B's
   agent identity within the shared project (state_pred), never
   impersonating A (forbidden).
6. Attenuation (4): A grants B a tool subset → B's out-of-subset tool call
   denied (gate/decision contracts); B cannot re-grant itself (forbidden).

## Feature-specific cheats → fences

- **Collaborate via shared global state** (skip real delegation) → required
  spawn/completion events + scope-tuple state_preds; a shortcut through
  ungated shared storage shows wrong attribution and fails theme 5.
- **Weaken isolation to make collab pass** → Stage-0 isolation parity suite
  green every cycle + theme-3 forbidden matchers + the zero-violation
  acceptance gate. The cheap path must remain "build scoped delegation."
- **Pass fail-closed cases by failing EVERYTHING** (deny all delegation) →
  theme-1/2 required matchers price the missing-collaboration direction.
- **Hardcode tenant/agent ids** → probe renames all identity literals; caps
  pattern for dev tenant/agent id literals in diff = 0.

## caps.json extras

Dev tenant/agent/project id literals in `crates/**` diff: max 0.

## Live mode

3 live cases: real model as agent A instructed to delegate a subtask —
required: actual spawn tool call + child scope correctness (structural
contracts; live text free).
