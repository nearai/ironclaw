# Shared LFD contract for IronClaw core roadmap lanes

This file is part of every lane's `goal.md`. The lane-specific file may add stricter rules; it must not weaken any rule here.

## Required launch artifacts

Before touching product code, the assigned agent must create these artifacts in its isolated working directory:

- `spec.md`: feature-specific system design, user-visible behavior, explicit non-goals, caller-level test plan, rollback concerns, and source links.
- `harness/score.sh`: runs `harness/lint.sh` first; emits a numeric dev score by default; for `--holdout`, emits one aggregate only.
- `harness/lint.sh`: enforces surface boundaries, capacity caps, forbidden leakage, and eval-literal overlap. If it touches eval answers, detailed findings go only to a human-only path outside the optimizer surface.
- `harness/probe.sh`: perturbs dev inputs with paraphrases, entity swaps, date shifts, tenant/user swaps, and channel/provider variants.
- `harness/status.sh`: reports elapsed wall-clock, score history, probe gap, token use where observable, spend so far, and projected spend before the next paid call.
- `eval/dev/`: visible inputs and scorer-owned answers for fast iteration.
- `eval/holdout/`: visible inputs only when needed; answers outside the repo and outside every optimizer-readable mount.
- `LOG.md`: instantiated from the LFD log template. Each cycle records hypothesis, expected failure mode, and diagnostic before code changes.

If the lane cannot produce roughly 200 total eval cases, state this explicitly in `LOG.md`, widen with generated or recorded cases before implementation where possible, and treat every positive score as weak until probe stability improves.

## Repo and product constraints

- Build new product behavior Reborn-side in `crates/`; do not add new feature behavior to the retiring v1 `src/` monolith unless the lane is explicitly cleanup.
- Read repo `AGENTS.md`, `CLAUDE.md`, relevant `crates/*/AGENTS.md` or subsystem docs, and touched crate tests before editing.
- Preserve bearer auth, webhook auth, CORS/origin checks, body limits, rate limits, secret handling, approvals, sandboxing, tenant boundaries, and host mediation.
- Do not use model calls for deterministic routing, retries, status handling, permission checks, idempotency, or exact transforms.
- Any feature that gates auth, secrets, network, memory, approvals, dispatch, persistence, adapters, or UI state needs caller-level or integration coverage, not helper-only tests.
- If behavior affects `FEATURE_PARITY.md`, setup docs, API docs, or subsystem specs, update them in the same branch.

## Shared scoring and blinding rules

- Acceptance is measured on holdout only.
- Dev scoring may reveal at most five misses per run, never raw answers at scale.
- Holdout scoring returns one aggregate number, max three calls per 24 hours unless a human explicitly changes the budget.
- `goal.md`, `spec.md`, `harness/`, and `eval/` are read-only during optimization.
- Any constraint violation prints exactly `VOID: constraint violation` and no extra detail to the optimizer.
- `harness/score.sh` must checksum the harness scripts before scoring and void if they changed.
- A run cannot claim completion from prose or dev score alone. It needs green Stage 0 tests and the holdout bar.

## Shared capacity caps

Unless a lane sets a stricter cap:

- Literal special-case branches: at most 5.
- Keyword, regex, or phrase lists: at most 25 entries.
- Seed fixture rows readable by product code: at most 40.
- Generated allowlists: at most 20 entries unless they are policy-owned config with tests.
- Prompt examples that mirror eval tasks: at most 12.
- Attached memory or context snippets in a single turn: at most 1,200 tokens unless the lane sets a lower cap.

Capacity caps apply to code, tests, prompts, fixtures, migrations, docs consumed by runtime, and generated config. Lint that checks eval overlap must not name the overlapping item.

## Common cycle protocol

1. Run `harness/score.sh` on dev.
2. Run `harness/probe.sh` and inspect the dev-vs-probe gap.
3. Run `harness/status.sh` and confirm budget remains.
4. Write the next hypothesis, expected failure mode, and diagnostic to `LOG.md`.
5. Make one scoped change.
6. Run the lane's Stage 0 tests and scorer again.
7. Commit the cycle with a message of the form `cycle <n>: <score>`.

If the metric did not move last cycle, the next attempt must be a structural change. If the probe gap grows, the next change must remove or blind an eval-shaped artifact rather than adding another special case.

## Common stop conditions

Stop when any of these is true:

- The holdout bar is hit and Stage 0 tests are green.
- Wall-clock or spend budget is exhausted.
- Marginal gain is approximately zero for three consecutive cycles.
- A critical security, tenant-isolation, secret-leakage, or data-loss issue is discovered.
- The scorer is found to be invalid and cannot be repaired within the remaining budget.

On stop, write a final `LOG.md` report with best dev score, best holdout score if any, probe gap trend, what generalized, what was abandoned, remaining risks, and the highest-leverage next steps.

## Common cheat audit

Every lane inherits these cheat routes and fences:

1. Eval mirroring: scorer-owned lint rejects seeded data or runtime-visible literals overlapping eval answers and reports only `VOID`.
2. Miss-list mining: dev miss lists are capped; holdout is aggregate-only.
3. Brute enumeration: list, regex, prompt, fixture, and branch capacity caps are enforced.
4. Scorer editing: `harness/` and `eval/` are read-only and checksummed.
5. Judge gaming: mechanical assertions are preferred; any LLM judge must separate known-good from known-bad before use.
6. One-sided success: every metric includes both false-positive and false-negative penalties.
7. Dev-set victory: acceptance is holdout-only.
8. Eval peeking: holdout answers stay outside the optimizer read surface.
9. Special-case branching: probe variants perturb wording, ids, tenants, providers, and dates.
10. Budget amnesia: status checks are mandatory every cycle.
11. Same-knob descent: flat metric requires structural change.
12. Lint-oracle mining: eval-sensitive lint emits no itemized findings to the optimizer.

