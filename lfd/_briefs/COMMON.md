# LFD Portfolio — Common Design (applies to every feature brief)

Every per-feature brief in this directory inherits this file. Briefs only
state what differs. Generators: read this + SCHEMA.md + the feature brief,
then emit the file set in "Generator deliverables" below.

## Budgets and bars (from the owner, 2026-07-07)

- Wall-clock: **12 h** per feature loop. Spend ceiling: **$100** LLM (live
  mode), tracked in `$LFD_STATE_ROOT/spend/<feature>.jsonl`.
- Acceptance bar on **holdout only**: 0.95 for built-features-in-hardening,
  0.90 for partial features, 0.85 for greenfield (each brief states its bar).
- Holdout: aggregate-only, **max 3 calls per 24 h window**, audit-logged.
- Stop conditions: bar hit on holdout · any budget exhausted · marginal dev
  gain < 0.01 for 4 consecutive cycles (then write final LOG.md report).

## Eval sizing — explicit warning

Per-feature evals are 30–60 dev + 10–15 holdout cases: far below the ~200
enumerability threshold. The compensating controls are (a) contract-style
scoring (satisfying a behavioral contract usually requires the machinery,
unlike data-lookup evals), (b) probe gap as the memorization gauge,
(c) feedback capped to aggregate + ≤5 worst case ids, (d) holdout answers
off-repo. Every goal.md states this warning verbatim in its Target section.

## Stage 0 (inner loop) — invariant across features

1. Implement `spec.md`. `cargo fmt` clean; `cargo clippy --all --benches
   --tests --examples --all-features` zero warnings; feature-relevant test
   suite (named in each goal.md) green.
2. The FULL existing suite named in the goal stays green **every cycle**.
   Weakening tests to make room is a violation (see caps: `#[ignore]`
   additions = 0, test deletions = 0 in diff).
3. Make `tests/integration/lfd/profiles/<feature>.rs` execute every dev
   case with `status: "ran"` (skeleton ships as `unsupported`).
Only then begin descending on the eval.

## Surface (allowlist; everything else denied)

- Read/write: `crates/**`, `src/**` (only where the brief says v1 is in
  scope), `tests/**`, `lfd/<feature>/LOG.md`,
  `tests/integration/lfd/profiles/<feature>.rs` (the ONE writable runner file).
- Read-only: `goal.md`, `lfd/<feature>/harness/**`, `lfd/<feature>/eval/**`
  (inputs are readable; that is intended), `lfd/_shared/**`,
  `tests/integration/lfd/**` (except your profile), `tests/integration/support/**`.
- BANNED entirely: `lfd/<feature>/harness/answers.dev.json` (sealed dev
  answers — reading it is detected and VOIDs), `$LFD_STATE_ROOT/**`
  (holdout, audit, lint reports), any other feature's `lfd/` dir.
- Live LLM: only via `harness/live_run.sh` (wrapper appends spend ledger);
  disposable `ANTHROPIC_API_KEY` from env; never hardcode keys.

## Common cheat inventory (fences already built into the harness)

Each brief adds feature-specific cheats; these 12 apply everywhere:

1. **Read sealed answers** → canary tokens + lint scan → VOID.
2. **Special-case branching on eval literals** (case ids, distinctive
   input strings) → caps.json capacity caps (eval-distinctive patterns in
   `src/**`+`crates/**` diff = 0) + answer-literal overlap lint + probe gap.
3. **Edit scorer/goal/eval** → read-only surface + pins.json hash check;
   holdout accepted only from pristine pins.
4. **Game the writable profile file** (fabricate outcomes) → outcome
   extraction lives in pinned code reading recorders + persisted state;
   profile API only assembles the harness. Residual risk accepted and
   audited: babysit cycle 1 + holdout runs on pristine runner.
5. **Always-succeed capability stubs** → contracts assert persisted state
   (`state_queries` hit real storage) and egress order, not just replies;
   failure-path cases REQUIRE errors/denials (status/gate matchers).
6. **Dev overfit / memorization** → probe.sh gap gauge; acceptance holdout-only.
7. **Dev-set victory lap** → stop conditions name holdout exclusively.
8. **Miss-list mining** → dev feedback = aggregate + ≤5 worst case ids,
   never matcher details. Leak budget: ~5 ids/cycle over visible inputs —
   reveals which scenario fails, not what the contract expects.
9. **Weakening Stage-0 tests** → caps: `#[ignore]`/`#[cfg(never)]`
   additions = 0; deleted test functions = 0 (lint counts in diff).
10. **Secret laundering** (encode/split secrets past the leak scan) →
    scrubber runs over ALL outcome text incl. events and tool outputs;
    forbidden `leak` matcher zeroes-halves the case; goal.md bans encoding
    tricks explicitly (human-audited).
11. **Same-knob descent** → stall rule: flat cycle ⇒ structural change
    required next; exploration quota every 5 cycles.
12. **Budget amnesia** → status.sh every cycle (elapsed, spend, gain/cycle
    trend); stop conditions include exhaustion.

## Live-model mode

Cases with `"live": true` run the real provider (disposable key, capped).
They live in `eval/dev/cases-live/` and holdout equivalents; they are the
LAST descent stage (goal.md stages them after scripted-dev bar is near),
because they cost money and add variance. `harness/live_run.sh` wraps the
runner, estimates usd from token counts, appends the spend ledger, refuses
to start if ledger ≥ ceiling.

## Generator deliverables (per feature)

```
lfd/<feature>/
  goal.md                     # from references/goal-template.md structure, all invariants kept
  spec.md                     # the inner-loop build spec (sources in brief)
  eval/dev/cases/*.json       # visible inputs (SCHEMA.md §1)
  eval/dev/cases-live/*.json  # live-mode acceptance cases (where brief says)
  harness/score.sh|lint.sh|probe.sh|status.sh   # from templates, FEATURE filled
  harness/answers.dev.json    # sealed contracts + canary_token LFDC-<feature>-<8hex>
  harness/caps.json           # capacity caps incl. feature-specific ones from brief
  harness/pins.json           # filled by portfolio finalization pass, ship {"files":{}}
  LOG.md                      # template instantiated, RUN START unset
tests/integration/lfd/profiles/<feature>.rs     # compiling skeleton (all queries → unsupported)
$LFD_STATE_ROOT/holdout/<feature>/
  cases/*.json + answers.holdout.json           # off-repo; canary_token distinct from dev
```

Case/contract quality rules: every case has ≥1 required state/egress/event
matcher (not reply-only); failure-direction cases are ≥25% of the set
(forbidden matchers, fail-closed status contracts); no entity, date range,
or template dominates >20% of the set; holdout cases are structurally
different (new entities, one unseen sub-scenario class per feature), never
copies of dev; dev and holdout canary tokens differ.
