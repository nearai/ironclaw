# INSTRUMENTS — shared harness binding for all blue lanes

This file amends COMMON.md for every lane. Where COMMON.md says the
assigned agent creates the harness and evals, THIS FILE OVERRIDES IT:
instruments are designer-owned and pre-built. A lane may tighten these
rules; nothing may weaken them. (COMMON.md's scoring physics — VOID,
holdout-only acceptance, caps, entropy — are unchanged; this file supplies
the mechanism that makes them real.)

## Roles

- **Designer** (LFD-design session / human owner): authors eval cases,
  sealed contracts, holdout placement, caps.json, pins.json; verifies the
  scorer (calibration + deliberate canary trip); red-teams the lane;
  patches the loss function mid-run if the loop cheats. The implementer's
  misses are diagnosed by the designer, never by exposing answers.
- **Implementation agent** (one per lane): builds the feature, writes ONE
  runner profile file, reads eval inputs, runs the instruments, writes
  LOG.md and code. It never authors or edits scorers, contracts, answers,
  caps, or pins.

## What already exists (verified in-tree)

| Artifact | Path | Status |
| --- | --- | --- |
| Schema (case / outcome / contract / scoring / trust boundary) | `lfd/_shared/SCHEMA.md` | authoritative |
| Scorer core (dev/holdout/probe/self-test, VOID, audit, budget) | `lfd/_shared/scorer/score_core.py` | self-test 10/10 |
| Lint (canaries, capacity caps, pins, answer-literal overlap) | `lfd/_shared/scorer/lint_core.py` | verified via self-test |
| Probe (deterministic perturbation + blinded map application) | `lfd/_shared/scorer/probe_core.py` | verified |
| Status (elapsed, score history, holdout budget, spend) | `lfd/_shared/scorer/status_core.py` | built |
| Wrapper templates (`score.sh` etc., LOG.md, caps.example.json) | `lfd/_shared/templates/` | built |
| Data-driven eval runner over `RebornIntegrationHarness` | `tests/integration/lfd/` | built this session; see its README/report for the exact `cargo test` invocation |
| Designer feature briefs (loss-function design per lane) | `lfd/_briefs/*.md` | 12 features + COMMON |
| Off-repo state root (holdout, audit, lint reports, spend) | `$LFD_STATE_ROOT/` | created |

## Per-lane binding

Each lane gets an `lfd/<lane-slug>/` package produced by the designer
(generator pass driven by the lane's brief + goal.md):

```
lfd/<lane-slug>/
  goal.md          # the lane goal.md from this directory + ADDENDA applied
  spec.md          # inner-loop build spec (designer-seeded; implementer may
                   # propose amendments, designer applies them)
  eval/dev/cases/*.json         # visible inputs (SCHEMA.md §1)
  eval/dev/cases-live/*.json    # live-model acceptance cases, where the lane uses them
  harness/{score,lint,probe,status}.sh   # thin wrappers over lfd/_shared
  harness/answers.dev.json      # SEALED — canary-tokened, read = VOID
  harness/caps.json             # capacity caps incl. lane-specific entries
  harness/pins.json             # sha256 pins over runner + scorer sources
  LOG.md
tests/integration/lfd/profiles/<lane-slug>.rs   # the ONE implementer-writable runner file
$LFD_STATE_ROOT/holdout/<lane-slug>/ # cases + answers, off-repo
```

Read-only for the implementer: `goal.md`, `harness/**`, `eval/**` (inputs
readable by design), `lfd/_shared/**`, all of `tests/integration/lfd/**`
except the lane's own profile, `tests/integration/support/**`.
BANNED (detected, VOIDs the score): reading `answers.dev.json`, anything
under `$LFD_STATE_ROOT/`, any other lane's `lfd/` package.

## Runner trust boundary

Outcome extraction (tool invocations, replies, egress, events, gates,
state queries against persisted storage, secret-leak scan) lives in pinned
shared code. The lane profile only assembles the harness for a case. Pins
mismatch is recorded on dev scoring and is a hard violation on holdout —
holdout is only accepted from a pristine runner. Residual risk (a profile
assembling a deliberately-lying harness) is accepted and controlled by:
babysitting cycle 1, designer diff review at acceptance, and holdout
re-run from pristine `git show` sources.

## Feedback resolution (enforced by the scorer, per COMMON.md)

- Dev: aggregate + PASS/FAIL for at most **5** worst case ids. Never
  matcher details or expected values.
- Holdout: one aggregate number; **max 3 calls per 24 h**, audit-logged to
  the off-repo state root; 4th call refused.
- Any lint violation: stdout is exactly `VOID: constraint violation`;
  details go to `$LFD_STATE_ROOT/lint-reports/` for the
  designer only.

(The scorer was patched to these values from its first build — miss cap
10→5, holdout window 12 h→24 h — and its self-test re-run; COMMON.md is
the binding constant set.)

## Eval authorship and growth

- Initial sets are designer-authored: 30–60 dev / 10–15 holdout per lane,
  contract-style (behavioral matchers over persisted state, egress,
  events, gates — not text-only). This is below the ~200 enumerability
  bar: every lane goal.md carries the small-eval warning, probe gap is the
  memorization gauge, and feedback is capped as above.
- The larger counts written in lane goal.md eval sections are GROWTH
  TARGETS owned by the designer role (mid-run widening, patch mode) — not
  implementer deliverables. An implementer that generates eval cases or
  answers has left its surface: VOID.
- Live-model cases (`live: true`) run through `harness/live_run.sh` with a
  disposable `ANTHROPIC_API_KEY`, append a usd estimate per call to the
  off-repo spend ledger, and refuse to start at the lane's ceiling. They
  are the last descent stage of a lane, never the inner loop.

## Lane launch checklist (designer runs this; Phase 6 per lane)

1. `harness/score.sh` on dev returns a number (0.0 with all-`unsupported`
   outcomes is valid for a skeleton profile).
2. Calibration: known-good synthetic outcomes ≥ 0.95, known-bad ≤ 0.30 on
   the lane's own contracts (scorer `--self-test` covers the engine; the
   lane check uses 2–3 lane cases).
3. Deliberate canary plant → exactly `VOID: constraint violation`, no
   detail on stdout → plant removed.
4. Blinding: from the repo root, the holdout answers are not reachable via
   any in-repo path; dev answers exist only in the sealed file.
5. `probe.sh` emits perturbed cases + map; `score.sh --probe` applies it.
6. pins.json filled; `status.sh` renders.
7. Wave/conflict check against LANE-ADDENDA.md before assignment.
