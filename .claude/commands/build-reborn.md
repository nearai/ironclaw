---
description: One iteration of the supervised Reborn build loop — pick one Reborn-stack task, gate it, open a PR, stop.
---

You are running **ONE iteration** of the IronClaw **Reborn** autonomous build loop. Land exactly
one PR-sized improvement to the Reborn stack, fully gated, then **STOP** — a human merges. Do not
merge anything.

## 0. Environment & state
- **Scope is the Reborn binary, not the legacy v1 binary.** This workspace contains both: the
  legacy `src/` tree (root `ironclaw` crate) and the Reborn stack (`crates/ironclaw_reborn_cli`
  → binary `ironclaw-reborn` and everything it composes). A crate is in scope when it is in the
  `crates/ironclaw_reborn*` family or reachable from `ironclaw_reborn_cli`'s dependency graph
  (`cargo tree -p ironclaw_reborn_cli --features webui-v2-beta` decides membership — e.g.
  `ironclaw_product_workflow`, `ironclaw_webui_v2`, `ironclaw_reborn_composition`,
  `ironclaw_engine`, `ironclaw_host_runtime`, `ironclaw_agent_loop`, `ironclaw_turns`, …).
  Legacy `src/` and the root crate are **out of scope** except deletions explicitly sanctioned by
  a closeout issue.
- You can locally prove: **fmt, clippy, per-crate unit tests, and the deterministic Reborn e2e
  gate** (`scripts/reborn-e2e-rust.sh` — pure cargo tests, no DB or Docker needed).
  **PostgreSQL-backed integration tests** (`cargo test --features integration`, the
  `reborn-integration` CI workflow) run only if a local PostgreSQL is available — otherwise
  implement the coverage fully and mark it **"verified in CI"** in the PR body. Same for
  Playwright e2e (`tests/e2e/`) and Docker sandbox tests. Implement, don't skip.
- **Spec-first:** before touching a crate, read its `CLAUDE.md` (most Reborn crates have one —
  `crates/ironclaw_reborn_composition/CLAUDE.md`, `crates/ironclaw_webui_v2/CLAUDE.md`,
  `crates/ironclaw_product_workflow/CLAUDE.md`, `crates/ironclaw_engine/CLAUDE.md`, …). Code
  follows spec; spec is the tiebreaker. For any feature that crosses the stack
  (product_workflow → composition → webui_v2 → runtime/serve → frontend), follow
  `.claude/skills/reborn-feature/SKILL.md` — it exists precisely so you don't rebuild what's
  already wired.
- Read `.work/loop-ledger.md` (create if missing; `.work/` is gitignored). It lists **PARKED**
  tasks (failed ≥2×, skip) and the **open-PR stack manifest** (§9) so you know what each
  in-flight branch holds.
- Run `gh pr list` to **map the in-flight stack** — which open branch holds which hot files. This
  is *not* a blocklist: you may stack on top of an open PR (§2a). Only avoid picking the **same
  task** another PR already implements.

## 1. Pick ONE task — priority cascade, stop at first viable
1. **Rebase a stale open Reborn PR** if one is `CONFLICTING` or has fallen behind `main` (check
   `gh pr list` + `gh pr view <n> --json mergeable`). Un-sticking an orphaned PR beats opening a
   new one. Rebase onto `main`, re-gate, push.
2. **Open GitHub issues** labeled `reborn` or `module:M1-webui-product` … `module:M5-events-streaming`
   (use `gh issue list --label reborn`) — human intent wins. **Skip issues that already have an
   assignee** — an assignee means another agent/human is on it. On picking, *claim* it (§1b).
   Large epics (e.g. QA automation, channel ports) are picked **one sub-task at a time**, never
   whole.
3. **Machine-discovered gaps** — often the highest-value output:
   - `#[ignore]`d or `todo!()`/`unimplemented!()` markers in Reborn-stack crates
   - drift between a crate's `CLAUDE.md` spec and its code
   - WebUI v2 routes missing from `crates/ironclaw_webui_v2/tests/webui_v2_descriptors_contract.rs`, or facade methods
     still returning default "unavailable" bodies that should be implemented
   - dual-backend gaps (a persistence feature implemented for only one of PostgreSQL/libSQL)
   - clippy / quality debt inside the Reborn stack
4. **Roadmap decomposition**: one concrete sub-task from `docs/plans/` (engine-v2 architecture,
   Reborn budgets follow-ups, channel port maps) or `docs/reborn/` — prefer tasks gateable on
   this host; DB/e2e-dependent halves are implemented fully and marked "verified in CI".

**Selection bias:** prefer work in **leaf crates / files no open PR holds** — it gates cleanly,
merges independently, and never enters the rebase race. Skip PARKED tasks and any task another
open PR already *implements* (stacking on top is allowed; duplicating is not).

### Escalation when the free surface is thin — DO NOT default to a no-PR iteration
"Nothing small, clean, and independent is left" is **not** "nothing to do." When the easy surface
is dry, **escalate in this order** before considering a no-PR iteration:
1. **Stack** on an open PR's branch (§2a) to reach work its files were blocking.
2. **CI-deferred halves** — integration-test coverage, Playwright scenarios, Docker-path work you
   can implement fully here and verify in CI. *Do not skip it just because it can't run locally.*
3. **Larger multi-file Reborn features** — a facade method + composition impl + route + frontend
   slice per the reborn-feature skill. Real, locally-gateable, and under-rated precisely because
   they aren't one-file changes. Confirm the feature isn't already shipped under a different
   design before starting. Scope to ONE coherent PR (§2); split if it sprawls.
4. **Issue hygiene** — if a merged PR fully implemented an issue, close it (§8) so the backlog
   reflects reality instead of looking artificially empty.

Only after 1–4 yield nothing real is a **no-PR iteration** correct — and even then, never
manufacture a marginal PR or weak test to fill the gap (§6).

## 1b. Claim the issue — make "in progress" visible (parallel-safe)
Multiple agents run this loop against the same repo, so an **unclaimed issue looks free even when
someone is already on it**. The moment you commit to an **issue-backed** task:
- **Assign it to yourself:** `gh issue edit <N> --add-assignee @me`. ("Assigned *at all*" — not
  *who* — is the in-progress signal; that's why §1 rung 2 skips already-assigned issues.)
- **Post your implementation plan as a comment:** `gh issue comment <N> --body "<plan>"` — a few
  bullets: the approach, the crates/files you'll touch, and how you'll gate it. Keep it short;
  it's a claim + sketch, not a design doc.
- Do this **right after picking, before branching** (§3). If the issue turns out non-viable and
  you drop it, `gh issue edit <N> --remove-assignee @me` so it reads as free again.

Machine-discovered gaps / roadmap sub-tasks with **no** backing issue have nothing to claim — the
branch + PR are the visibility. Rebasing a stale PR (§1 rung 1) is already claimed by that PR.

## 2. FENCES (hard)
- **Reborn stack only** (§0). Do NOT modify legacy `src/` paths, and do NOT add new code that
  couples a Reborn crate to the root `ironclaw` crate — the
  `reborn_dependency_boundaries` architecture test enforces this; respect it, never weaken it.
- **Boundary invariants** (from the crate specs — violating any of these is an automatic redo):
  - `ironclaw_webui_v2` handlers consume **only** `RebornServicesApi` — never the dispatcher,
    host_runtime, stores, or DB directly.
  - Tenant/user/agent identity comes from the **authenticated caller**, never the request body.
  - New persistence supports **both** PostgreSQL and libSQL.
  - **LLM data is never deleted** — mark, filter, retain; never strip or truncate.
  - `credential_name` (backend secret identity) vs `extension_name` (user-facing identity) must
    not be conflated.
- Repo code style: no `.unwrap()`/`.expect()` in production code, `thiserror` error types,
  prompt templates in files not Rust constants, `debug!` not `info!` for internals.
- Scope = **ONE coherent PR**. If it sprawls, split and do the smaller, self-contained half.

## 2a. Stacking — never idle waiting for a merge
Throughput is bounded by human merge cadence, not your output. **Do not stall** when every clean
free file is held by an open PR — **stack** instead:
- **Branch off the dependency, not `main`.** If your task needs code from open PR `#N` (branch
  `B`), `git switch -c <type>/<slug> B`. If it's independent, branch off `main` as usual.
- **One logical change per PR still holds** — stacking means *ordering* PRs, not bundling them.
- **Record the stack** in the ledger manifest (§9): `branch → base (main | #N) → holds <files>`.
- **Rebase forward when a base merges.** When `#N` lands, rebase its children onto `main` and
  re-gate them (this is also priority-cascade rung 1).
- **Prefer independent leaf work first** (§1 selection bias); keep stacks **shallow**.
- A **no-PR iteration is still valid** when there is genuinely no real task — *do not manufacture
  a marginal PR or a weak test just to avoid idling* (§6).

## 3. Branch
`git switch -c <type>/<short-slug>` — type ∈ {feat, fix, refactor, perf, docs, test}. Work in an
isolated worktree if your current one is dirty.

## 4. Implement
- If the task matches a skill, **follow it** as a gate, not a shortcut: `reborn-feature` for
  cross-stack features, `add-sse-event` / `add-tool` for their scaffolds, the
  thermo-nuclear-code-quality-review standards for your own diff (§7).
- Match surrounding code style, comment density, and idiom. Read the crate `CLAUDE.md` first.

## 5. Gates — ALL must pass locally (or be CI-deferred with a stated reason)
```
cargo fmt
cargo clippy --all --benches --tests --examples --all-features   # zero warnings
cargo test -p <each touched crate>                               # plus cargo test for wide changes
scripts/reborn-e2e-rust.sh                                       # deterministic Reborn e2e gate
```
- Touched `ironclaw_webui_v2_static` JS → `node --check <file>` each touched file (no build step).
- Touched WebUI v2 routes → `crates/ironclaw_webui_v2/tests/webui_v2_descriptors_contract.rs` must be updated and green.
- PostgreSQL available → also run `cargo test --features integration` for touched persistence;
  otherwise state "integration coverage verified in CI (`reborn-integration`)" in the PR body.
- The pre-commit hook (`scripts/pre-commit-safety.sh`) must pass — never `--no-verify` past it.

## 6. Anti-reward-hacking guard — inspect YOUR OWN diff before committing
Reject and fix your own work if the diff:
- adds a net-new `#[ignore]`, `todo!()`/`unimplemented!()`, or `#[allow]` **just to pass a gate**
- **weakens** any test assertion, architecture boundary test, or descriptor contract
- adds a `// dispatch-exempt:` or similar escape-hatch annotation to dodge the safety pipeline
- pushes a file from **<1k to >1k lines** without strong justification

The job is to **close** gaps, not silence the detectors.

## 7. Self-review — thermo-nuclear code-quality pass (REQUIRED before every PR)
Apply the **`thermo-nuclear-code-quality-review`** standards to your own diff and act on the
findings *before* pushing. (The skill is `disable-model-invocation`, so follow its rules directly
— read `.claude/skills/thermo-nuclear-code-quality-review/SKILL.md`, don't try to invoke it.)
Specifically:
- hunt for a "code-judo" reframing that **deletes** complexity rather than rearranging it;
- no file pushed from <1k to >1k lines without a strong reason — decompose instead;
- no new ad-hoc conditionals / special-case branches bolted onto existing flows;
- no dead, redundant, or speculative branches;
- no thin wrappers, casts, or optionality that obscure the real contract;
- keep logic in its canonical layer (port → facade → impl → route, per the reborn-feature skill),
  reuse existing helpers, don't leak substrate handles across crate boundaries.

Record any **deliberate** tradeoff the review surfaced in the PR body.

## 8. Land — stop at PR
- Commit: `<type>(<scope>): <subject>`, reference the issue if any. End with your standard
  `Co-Authored-By: Claude …` trailer.
- Push and open the PR with `gh`. PR body: **what + why**, which gates ran locally, and what is
  **CI-deferred** (integration / Playwright e2e / Docker). End with the Generated-with trailer.
- **Issue hygiene:** if the PR fully implements an issue, put `Closes #N` in the body. If you
  notice an *already-merged* PR that closed an issue without the keyword, close that issue
  directly (`gh issue close`) with a pointer to the PR.
- **STOP. Do not merge.**

## 9. Record
Append one line to `.work/loop-ledger.md`: task · branch · PR# · gate status. If the gates failed
twice this iteration, mark the task **PARKED** with the reason and move on — do **not** open a
broken PR.

Maintain an **open-PR stack manifest** in the ledger so the next iteration can rebase in order:
```
## Stack (open PRs)
| branch | base (main | #N) | holds (hot files) | status |
```
Update it whenever you open a stacked PR, and prune rows as PRs merge (rebasing any children onto
`main`).

Then end the turn; the loop paces the next iteration.
