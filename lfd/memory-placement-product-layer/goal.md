# Goal: harden memory placement at the product/provider boundary

This is the lane-16 Wave-1 LFD target for memory placement. It merges the
roadmap lane goal, `lfd/_briefs/memory-placement-product-layer.md`,
`lfd/_briefs/COMMON.md`, and the lane-16 addendum. The load-bearing claim is
that product-facing memory operations go through the provider-neutral
`ironclaw_memory::MemoryService` boundary with native memory as the default
provider, while host/admin policy owns authorization, constraints, audit,
storage placement, sandboxing, streaming, and network mediation.

## Stage 0 — Build to spec

Implement `spec.md`. Do not descend on the eval until the inner suite is
green, and keep it green every cycle.

Stage-0 command list:

1. `cargo fmt`
2. `cargo clippy --all --benches --tests --examples --all-features`
3. `cargo test -p ironclaw_memory`
4. `cargo test -p ironclaw_memory_native`
5. `cargo test --features integration --test integration group_memory`
6. Make `tests/integration/lfd/profiles/memory_placement.rs` execute every
   dev case with `status: "ran"` through the real Reborn/product call path.

The LFD profile is harness assembly only. Outcome extraction remains in pinned
runner code reading persisted state, recorder events, gates, egress, and
profile state queries.

## Target

Metric: product/provider boundary correctness and parity, both directions.
Missing required behavior starves the required numerator; forbidden native
shortcuts, wrong storage tier, dropped retention/version records, provider
direct egress, prompt-layer authority, and policy bypasses halve the case per
violation class. Harness errors and unsupported cases score 0.

Weighting intent:

- 35% provider boundary exists and native memory works through it by default.
- 25% host/admin policy can allow, deny, and constrain providers.
- 25% existing memory read, write, search, tree, profile, versioning, and
  prompt-context behavior remains structurally equivalent through the boundary.
- 15% audit, auth, sandboxing, storage, streams, and network stay
  host-mediated.

Bar: **0.95 on holdout**, architecture/dependency checks green, and zero
host-mediation bypasses. Acceptance is measured on holdout exclusively.
Holdout scoring is aggregate-only, max 3 calls per 24 h, audit-logged.

Small-eval warning: Per-feature evals are 30–60 dev + 10–15 holdout cases:
far below the ~200 enumerability threshold. The compensating controls are
(a) contract-style scoring (satisfying a behavioral contract usually requires
the machinery, unlike data-lookup evals), (b) probe gap as the memorization
gauge, (c) feedback capped to aggregate + ≤5 worst case ids, (d) holdout
answers off-repo.

## Visibility and surfaces

Eval inputs may be read; eval answers may not. Dev answers are sealed in
`lfd/memory-placement-product-layer/harness/answers.dev.json` and canary
scanned. Holdout answers live outside the repo under
`$LFD_STATE_ROOT/holdout/memory-placement-product-layer/`.

Read/write during optimization:

- `crates/**` for the memory contract, native provider, product workflow,
  Reborn composition, host runtime, authorization/policy/audit/network seams.
- `src/**` only where existing v1 memory behavior is being preserved, not for
  new Reborn feature construction.
- `tests/**` for focused coverage, plus exactly one writable LFD runner file:
  `tests/integration/lfd/profiles/memory_placement.rs`.
- `lfd/memory-placement-product-layer/LOG.md`.

Read-only during optimization:

- this `goal.md`, `spec.md`, `harness/**`, `eval/**`, `lfd/_shared/**`,
  `tests/integration/lfd/**` except the profile file above, and
  `tests/integration/support/**`.

Banned entirely:

- reading `lfd/memory-placement-product-layer/harness/answers.dev.json`;
- reading or writing `$LFD_STATE_ROOT/**` except through the harness;
- editing any other lane package.

Budgets: 14 h wall-clock, $10 LLM/API ceiling, no live external memory
provider. The deterministic suite should spend $0; the ceiling is a backstop.

## Instruments

- `harness/lint.sh` runs before scoring. Any violation prints exactly
  `VOID: constraint violation`.
- `harness/score.sh` scores dev by default and holdout with `--holdout`.
- `harness/probe.sh` deterministically perturbs visible dev inputs and reports
  the dev/probe gap once outcomes exist.
- `harness/status.sh` reports elapsed time, score history, holdout budget,
  spend, and gain trend.

## Capacity caps

`harness/caps.json` enforces the common caps plus lane-specific caps:

- provider-specific branches in product/provider routing diff: max 5;
- new `pub use ironclaw_memory_native::...` reexports from product crates: 0;
- direct `ironclaw_memory_native` references in product/composition/host policy
  routing diff: 0, except native-provider factory wiring;
- eval case-id branching: 0;
- dev-seed literal copying into product code: 0;
- `#[ignore]` and `#[cfg(never)]` test weakening additions: 0.

## Cycle protocol

1. Run `harness/status.sh`.
2. Run Stage-0 checks relevant to the changed surface.
3. Run the LFD runner over dev cases, then `harness/score.sh --outcomes <dir>`.
4. Run `harness/probe.sh`; if probe outcomes are available, score with
   `harness/score.sh --probe lfd/memory-placement-product-layer/eval/probe/map.json --outcomes <probe-out>`.
5. Write the LOG entry before the next code change: hypothesis, expected
   failure mode, diagnostic, and result.
6. Commit each cycle with a message containing the dev score and the changed
   boundary surface.

Each cycle must exercise at least one behavior-parity case and one
architecture/dependency case. If native passes but fake-provider parity fails,
the next cycle targets abstraction leakage, not native-specific behavior.

## Entropy rules

- Rotate between provider API, policy mediation, parity, host mediation, and
  dependency/static boundaries every 3 cycles.
- If the metric is flat for one cycle, the next attempt must be a structural
  change rather than the same tweak with more branches.
- Every fifth cycle must try a different surface or remove an eval-shaped
  artifact, even if the current approach is improving.
- A widening provider-specific branch list is a failed descent. Replace it
  with provider-neutral data/trait shape.

## Cheat audit

1. Special-case native provider in product code. Fence: fake-provider parity
   cases and provider-branch cap.
2. Put the fake provider in the writable lane profile. Fence: spec requires
   the fake provider to live in pinned shared support; state query asserts
   `fake_provider_in_support == true`.
3. Re-export native internals under a provider-neutral name. Fence: cap on
   `pub use ironclaw_memory_native::` and dependency state query.
4. Always allow policy. Fence: denial/constrained cases require gate denial or
   constrained operation state, and provider invocation is forbidden when
   denied.
5. Route denied operations to the provider and rely on provider errors. Fence:
   denied cases require host policy decision before provider invocation.
6. Preserve storage but drop audit. Fence: mutating cases require both
   persisted state and `memory.*` audit events.
7. Use the wrong storage tier. Fence: storage-placement cases require
   `memory_provider_repository` and forbid control-plane/prompt-only writes.
8. Drop retention/version records during boundary refactor. Fence:
   retention cases require old-version retrieval and retention log counts.
9. Hide embedding/network access inside the provider. Fence: host mediation
   cases require host network egress and forbid provider-direct egress.
10. Move authority into prompt assembly. Fence: prompt cases require context
    build without memory writes from prompt assembly.
11. Hardcode dev case ids or seeded literals. Fence: caps, answer-literal
    overlap lint, capped dev feedback, and probe gap.
12. Weaken tests to make Stage 0 pass. Fence: caps for `#[ignore]` and
    `#[cfg(never)]`, plus review of deleted tests.

## Stop conditions

Stop when holdout is at least 0.95 with Stage 0 green and zero mediation
bypasses, any budget is exhausted, marginal dev gain is < 0.01 for 4
consecutive cycles, a critical auth/secret/network/storage bypass is found,
or the scorer is invalid and cannot be repaired within budget. On stop, write
a final LOG entry with best dev and holdout scores, what generalized, what was
abandoned, and the highest-leverage next change.

## Human pre-flight

Use a disposable API key with a provider-side spend limit if live-model
experiments are added later. Babysit cycle 1 and confirm the optimizer uses
the profile, scorer, probe, and status instruments rather than editing or
reading sealed artifacts.
