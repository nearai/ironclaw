# Loop worker three-lane experiment

Status: experiment design for the same host membrane running three loop
implementations. Companion to `2026-09-pi-loop-worker-plan.md` (Pi worker
behind the #7908 membrane) and `2026-08-harness-v0-findings.md` (Claude Code
over ACP, #7648).

## Lanes

| Lane | Loop implementation | Placement | Wire |
| --- | --- | --- | --- |
| A | Canonical Rust loop (`ironclaw_agent_loop`) | In-process host | none (same process) |
| B | Rust worker (`ironclaw-loop-worker`) | Sandbox container | v2 stdio, content-`Blind` |
| C | Pi worker (`ironclaw-pi-worker`, `@earendil-works/pi-agent-core`) | Sandbox container | v2 stdio, content-`Resolved` |

B versus A tests sandbox placement with the same Rust loop. C versus B
compares the complete loop variants, including prompt construction,
compaction, and tool handling. A shared host does not make their prompts or
tool decisions identical. Record those differences before assigning a cause.

A fourth comparator exists from #7648: Claude Code over ACP
(`Dockerfile.claude-code-acp`, harness executor, per-profile routing). It is
not part of this batch's wire surface but is recorded here because the
experiment should read all three families together: A/B/C differ only in loop
implementation; the ACP lane differs in *who owns the whole loop*.

## Task set

Reuse the six audited tasks from `2026-08-harness-v0-findings.md` unchanged:

1. Trace WebUI submission through profile-level executor routing (turn-path Q&A).
2. Explain `credential_name` versus `extension_name` and its setup-UI effect.
3. Repair a seeded 1.5-second live-text coalescing regression, add focused
   coverage, make a local commit, and draft the PR handoff.
4. Diagnose and repair UTF-8 truncation across two turns.
5. Diagnose and repair an incorrect profile-routing lookup across two turns.
6. Retain an architecture map across four turns, create a continuity note,
   and review the note against the original facts.

Scoring stays the findings doc's 0/1/2 strict semantic points; every turn
names its allowed workspace root and requires `pwd` +
`git rev-parse --show-toplevel` preflight. Same model identity in all lanes.

## Metrics per task

Recorded per task, per lane, from the run artifact (host-side, never
worker-reported):

- strict success (2/1/0) and the semantic observation note;
- model calls (`StreamModel` count) and capability calls
  (`InvokeCapability`/batch count);
- prompt tokens by bundle section: identity, instruction snippets, memory
  snippets, transcript window, tool surface (requires the gateway usage log;
  the Rust loop exposes usage, Pi must surface it through `LoopModelUsage`);
- gate pauses: count of `Blocked` exits and total time in gates per task;
- compactions: count and wall time of `Compact` host calls;
- wall time per turn and per task;
- cost: USD from provider usage where the model identity exposes pricing.

The harness-v0 finding that native usage was not symmetrically exposed is a
blocker to fix first (`model_usage` must flow in all three lanes) or token
comparisons stay inconclusive.

## Reading the result

- **B ≈ C ≫ A** — the sandbox membrane itself costs the quality; the loop
  implementation does not matter. Invest in placement (cheaper exec path),
  not loop rewrites.
- **B ≈ A, C ≫ both** — the host membrane is not the problem; Pi's loop
  strategy is worse. Keep the canonical loop; treat Pi as a portability
  experiment only.
- **C ≈ B ≫ A** — both sandboxed loops underperform the in-process loop
  equally: suspicion falls on the wire (latency, ref-resolution round trips,
  `Blind`/`Resolved` content handling). Profile which host call dominates
  before changing loops.
- **C < A** — Pi's loop strategy is genuinely better behind the same host;
  promotion conversation (checkpoint schema parity, budget enforcement
  parity) opens.
- Between the poles: split the deltas per metric — e.g. if C ≈ A on tool
  calls but ≫ A on wall time, look at per-call overhead (framing, resolve
  round trips) rather than loop strategy.

## How to run each lane

Prerequisites for every lane: built sandbox image
(`docker build -f Dockerfile.sandbox-worker .`), a configured model backend,
`IRONCLAW_REBORN_SANDBOX_LOOP_WORKER=true`.

- **Lane A** (canonical in-process loop): unset
  `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER`.
- **Lane B** (Rust worker in sandbox):
  `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER=true` (worker kind defaults to
  `rust`), or explicitly
  `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=rust`.
- **Lane C** (Pi worker in sandbox):
  `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER=true` plus
  `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND=pi` (accepted values `rust`|`pi`,
  case-insensitive; invalid values fail startup).
- **ACP comparator**: `[harness]` configuration + per-profile routing from
  #7648 (see `2026-08-harness-v0-findings.md` for the exact pins).

Wire-level conformance for lanes B and C (no Docker) is pinned by
`cargo test -p ironclaw_turn_runner --test loop_worker_conformance`; the
real-container Pi lane is `cargo test -p ironclaw_integration_tests --test
reborn_integration_sandbox_shell_turn` with
`IRONCLAW_REQUIRE_DOCKER_TESTS=1`.
