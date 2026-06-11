---
description: One iteration of the supervised Reborn review loop — take an un-reviewed open Reborn-stack PR, thermo-nuclear review it, fix every finding, re-review to approval, push, comment. Stop.
argument-hint: "[PR number]"
---

You are running **ONE iteration** of the IronClaw **Reborn** autonomous review loop. Take exactly
one open Reborn-stack PR that has **not been reviewed yet**, run the thermo-nuclear code-quality
review against its diff, **address every finding**, re-run the review until it clears the
approval bar, push the fixes back to the PR branch, and leave a note summarizing what changed.
Then **STOP** — a human merges. Do not merge anything, and do not open a new feature PR.

If the user passed a PR number as an argument (`$ARGUMENTS`), review **that** PR and skip the
selection cascade in §1. Otherwise pick one per §1.

## 0. Environment & state
- **Scope is the Reborn stack.** A PR is in scope when its changed files live in the
  `crates/ironclaw_reborn*` family or the substrate crates the `ironclaw-reborn` binary composes
  (`ironclaw_product_workflow`, `ironclaw_webui_v2`, `ironclaw_webui_v2_static`,
  `ironclaw_reborn_composition`, `ironclaw_engine`, `ironclaw_host_runtime`,
  `ironclaw_agent_loop`, `ironclaw_turns`, …) — the `reborn` / `module:M1..M5` labels are a
  strong hint, but the file list (`gh pr view <N> --json files`) decides. Legacy `src/`-only PRs
  belong to a different loop; leave them.
- You can locally prove: **fmt, clippy, per-crate unit tests, and the deterministic Reborn e2e
  gate** (`scripts/reborn-e2e-rust.sh`). PostgreSQL integration tests and Playwright e2e run only
  if the local environment supports them — if your fixes touch those paths, implement them fully
  and mark them **"verified in CI"** in the note. Never weaken or delete coverage just because it
  can't run on this host.
- Read `.work/loop-ledger.md` (create if missing; `.work/` is gitignored). The **REVIEWED** list
  records PRs this loop has already passed through — **skip them** so two iterations don't
  re-chew the same PR. The open-PR stack manifest (§9 of the build loop) tells you which branch
  bases on which, so a fix you push to a base branch is understood to flow to its children on
  their next rebase.

## 1. Pick ONE un-reviewed PR — stop at first viable
Run `gh pr list --json number,title,reviewDecision,reviews,isDraft,mergeable,headRefName,author,labels`.
A PR is a **candidate** when ALL hold:
- **in Reborn scope** (§0),
- **not a draft** (`isDraft == false`),
- **never reviewed** — `reviews` is empty **and** `reviewDecision` is `""` (no `APPROVED` /
  `CHANGES_REQUESTED` / `COMMENTED` from anyone, human or bot reviewer). A PR that already
  carries a review is someone else's turn; leave it.
- **not already in the ledger REVIEWED list** (§0),
- **mergeable is not `CONFLICTING`** — a conflicted PR needs a rebase first (that's the build
  loop's job, §1 rung 1); don't try to review a diff that won't apply.

Among candidates, **prefer the oldest open PR** (lowest number) — un-reviewed PRs rot while the
loop chases newer work; clearing the front of the queue is the point. If two agents could race
the same PR, the review you post (§7) is the claim — check once more right before posting that no
review landed in the meantime.

If **no** candidate exists (every open PR is draft, already reviewed, conflicting, out of scope,
or ledger-recorded), this is a **no-PR iteration**: record "nothing to review" in the ledger and
STOP. Do **not** invent a review of an already-reviewed PR to fill the gap.

## 2. FENCES (hard)
- **You review and fix the PR you picked — you do not redesign its feature.** Your mandate is the
  thermo-nuclear *quality* bar (structure, decomposition, spaghetti, abstraction, file size), not
  re-scoping what the PR set out to do. If the PR's *premise* is wrong (wrong layer, duplicates
  shipped work, violates a crate spec), say so in the review note and STOP — don't silently
  rewrite it into a different PR.
- **Keep the fix coherent with the PR.** Push only quality fixes for *this* PR's diff to *this*
  PR's branch. Don't fold in unrelated changes, don't bump scope, don't merge.
- **Boundary invariants are review findings when the PR violates them** (and your fixes must
  never introduce a violation):
  - `ironclaw_webui_v2` handlers consume only `RebornServicesApi`; no Reborn crate may depend on
    the root `ironclaw` crate (`reborn_dependency_boundaries` enforces this — never weaken it).
  - Identity from the authenticated caller, never the request body.
  - Dual-backend (PostgreSQL + libSQL) parity for new persistence; **LLM data is never deleted**.
  - `credential_name` vs `extension_name` must not be conflated.
  - New WebUI v2 routes must appear in `crates/ironclaw_webui_v2/tests/webui_v2_descriptors_contract.rs`.
- Repo code style: no `.unwrap()`/`.expect()` in production code, `thiserror`, prompt templates
  in files, `debug!` not `info!` for internals, no log line that prints prompt content, key
  material, or secrets.

## 3. Check out the PR branch
`gh pr checkout <N>` — this puts you on the PR's head branch with its commits. Confirm you are on
the PR branch and that it builds **before** you start (a PR that's already red tells the review
where to focus). Record the head SHA so you can diff your fixes later. Read the `CLAUDE.md` of
each crate the PR touches — code follows spec; spec is the tiebreaker.

## 4. Thermo-nuclear review (REQUIRED — this is the gate)
The `thermo-nuclear-code-quality-review` skill is `disable-model-invocation`, so **follow its
rules directly — read `.claude/skills/thermo-nuclear-code-quality-review/SKILL.md`, don't try to
invoke it.** Apply it to the PR's **full diff vs `main`** (`git diff main...HEAD`), not just the
latest commit. Be ambitious and demanding exactly as the skill prescribes:
- hunt for a **"code-judo"** reframing that **deletes** whole branches/helpers/modes rather than
  rearranging them;
- flag any file the PR pushes from **<1k to >1k lines** — decompose instead;
- flag new **ad-hoc conditionals / special-case branches** bolted onto existing flows;
- flag thin wrappers, casts, needless optionality, dead/speculative branches, canonical-helper
  duplication, and logic landing in the wrong layer/crate (port → facade → impl → route, per
  `.claude/skills/reborn-feature/SKILL.md`).

Produce a concrete, prioritized **findings list** (use the skill's ordering: structural
regressions first, then missed simplifications, spaghetti, boundary/abstraction, file size,
modularity, legibility). Each finding must name the file/lines and the **remedy**, not just the
smell. If the diff genuinely clears the approval bar on the first pass with zero findings, jump
to §7 and post an approval note — but hold the skill's bar honestly; a real review rarely finds
nothing.

## 5. Address EVERY finding
Fix every finding the review raised, applying the skill's **preferred remedies** (delete a layer,
collapse duplicate branches into one flow, extract a helper/module, replace a condition chain
with a typed dispatcher, move logic to its canonical home). Match the PR's surrounding style,
comment density, and idiom.

If a finding is a **deliberate, justified tradeoff** the original author made (e.g. duplication
that is genuinely clearer than the abstraction), don't force it — instead record the
justification in the note (§7) so the human merger sees the reasoning. The bar is "every finding
*resolved*" — fixed, or explicitly justified — not "every finding mechanically rewritten."

### Anti-reward-hacking guard — inspect YOUR OWN fix diff
Reject and redo your own change if it:
- adds a net-new `#[ignore]`, `todo!()`/`unimplemented!()`, or `#[allow]` **just to pass a gate
  or quiet the review**;
- **weakens** any test assertion, boundary test, or descriptor contract the PR introduced;
- "addresses" a finding by deleting the test/coverage that surfaced it;
- pushes a file from **<1k to >1k lines** without strong justification.

The job is to make the PR **cleaner**, not to silence the detectors. Suppressing a finding is
worse than leaving it open with a note.

## 6. Re-gate — ALL must pass locally (or be CI-deferred with a stated reason)
```
cargo fmt
cargo clippy --all --benches --tests --examples --all-features   # zero warnings
cargo test -p <each touched crate>
scripts/reborn-e2e-rust.sh                                       # deterministic Reborn e2e gate
```
Touched `ironclaw_webui_v2_static` JS → `node --check` each file. PostgreSQL integration tests
run if available, else note "verified in CI (`reborn-integration`)". The PR must be **green after
your fixes**, including any test the original PR shipped.

## 7. Re-review to approval, then push + comment
1. **Re-run the thermo-nuclear pass (§4) on the now-fixed diff.** It must clear the skill's
   **Approval Bar**: no structural regression, no obvious missed simplification, no unjustified
   file-size explosion, no spaghetti branching, no hacky/magical abstraction, no needless
   wrapper/cast/optionality, no boundary leak or canonical-helper duplication. If a *new* finding
   appears, loop back to §5. Iterate §5→§6→§7 until the bar is clean (or a remaining item is
   recorded as a justified tradeoff).
2. **Commit your fixes** onto the PR branch:
   `style|refactor(<scope>): address thermo-nuclear review findings` (use `refactor` for
   structural changes, `fix` if you corrected a real bug the review exposed). End with your
   standard `Co-Authored-By: Claude …` trailer.
3. **Push to the PR branch** (the branch already has an upstream from `gh pr checkout`, so a
   plain `git push` updates the PR). Never force-push someone else's branch unless you only
   rebased it; for added fix commits a fast-forward `git push` is correct.
4. **Post the review note as a PR review**, re-checking first that no other review landed (§1):
   `gh pr review <N> --approve --body "<note>"` once the bar is clean, or
   `gh pr review <N> --comment --body "<note>"` if you pushed fixes but want a human to look at a
   recorded tradeoff. The note states: **the findings** (grouped by the skill's priority order),
   **what you changed** for each (with the fix commit SHA), any **deliberate tradeoff** left in
   place with its justification, which **gates ran locally** and what is **CI-deferred**
   (integration / Playwright e2e), and a one-line **verdict**. End with the Generated-with
   trailer.
5. **STOP. Do not merge.**

## 8. Record
Append one line to `.work/loop-ledger.md`: PR# · what was fixed · gate status ·
approved|commented. Add the PR to the **REVIEWED** list so the next iteration skips it. If the
review concluded the PR's *premise* is wrong (§2) and you pushed no fixes, record that verdict
instead so a human can decide.

Then end the turn; the loop paces the next iteration.
