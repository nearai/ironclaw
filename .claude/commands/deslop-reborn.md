---
description: One iteration of the Reborn de-slop loop — take ONE Reborn-stack crate, fan out parallel review sub-agents (thermo-nuclear quality, paranoid architect, API/spec/invariants, test-coverage/wiring), synthesize, apply fixes/refactors/missing tests, open a PR. Stop.
argument-hint: "[crate-name]"
---

You are running **ONE iteration** of the IronClaw **Reborn de-slop** loop. Take exactly one
Reborn-stack crate that has **not been de-slopped yet**, review it *deeply and from multiple
angles using parallel sub-agents*, then **land one coherent PR** that fixes the findings —
simplifications, refactors, spec/interface corrections, and any **missing unit/e2e tests** needed
to keep the component correct forever. Then **STOP** — a human merges. Do not merge anything.

Unlike `/review-reborn` (which reviews an open PR's diff), this loop reviews a **whole crate as it
stands on `main`** and improves it. The unit of work is a **crate**, not a diff.

If the user passed a crate name as an argument (`$ARGUMENTS`, e.g. `ironclaw_reborn_config`),
de-slop **that** crate and skip the selection cascade in §1. Otherwise pick one per §1.

## 0. Environment & state
- **Scope is the Reborn stack.** Eligible crates are the `crates/ironclaw_reborn*` family plus
  the substrate crates the `ironclaw-reborn` binary composes
  (`cargo tree -p ironclaw_reborn_cli --features webui-v2-beta` decides membership —
  `ironclaw_product_workflow`, `ironclaw_webui_v2`, `ironclaw_reborn_composition`,
  `ironclaw_engine`, `ironclaw_host_runtime`, `ironclaw_agent_loop`, `ironclaw_turns`, …).
  The legacy `src/` tree and root `ironclaw` crate are **out of scope**.
- You can locally prove: **fmt, clippy, per-crate unit tests, and the deterministic Reborn e2e
  gate** (`scripts/reborn-e2e-rust.sh`). PostgreSQL integration tests and Playwright e2e run only
  if the local environment supports them — if your fixes touch those paths, implement them fully
  and mark them **"verified in CI"** in the PR body. Never weaken or delete coverage just because
  it can't run here.
- Read `.work/deslop-ledger.md` (create if missing; `.work/` is gitignored). The **DESLOPPED**
  list records crates this loop has already passed through — **skip them** so two iterations
  don't re-chew the same crate. Also run `gh pr list`: **don't pick a crate an open PR is
  actively reshaping** (you would collide on the same files); prefer a crate no open PR holds.

## 1. Pick ONE un-de-slopped crate — stop at first viable
List the eligible crates (§0). A crate is a **candidate** when ALL hold:
- **not already in the ledger DESLOPPED list** (§0),
- **not the hot surface of an open PR** (§0) — leave those to the build/review loops,
- it has real source to review (a pure test-harness or static-asset crate is fair game only for
  the coverage / interface angles).

**Selection bias:** prefer **smaller / leaf crates first** — they fit entirely in context, gate
cleanly, and merge independently. A crate whose `src` is **under ~2k lines** can and SHOULD be
read in full (§3). Clear the small, high-leverage crates before taking on the giants
(`ironclaw_engine`, `ironclaw_reborn_composition`, `ironclaw_webui_v2` are large — those need
targeted reading, not a full load, and may warrant splitting the de-slop across iterations
module-by-module). When unsure, prefer a crate that is **load-bearing for invariants** (identity,
event store, config/secrets, product workflow facade, host runtime trust) over a purely
mechanical one — a slop fix there protects the whole system.

If **every** crate is ledger-recorded or PR-held, this is a **no-de-slop iteration**: record
"nothing to de-slop" in the ledger and STOP. Do **not** manufacture a marginal PR to fill the gap.

## 2. FENCES (hard)
- **De-slop the crate — do not redesign its feature or change its observable contract.** Your
  mandate is quality (structure, decomposition, spaghetti, abstraction, file size), **interface
  hygiene** (public surface not over-exposed, crate `CLAUDE.md`/README accurate), **invariant
  integrity**, and **test coverage** — not re-scoping what the crate does. If you conclude the
  crate's *premise* or *layer* is wrong, write that up in the PR body / an issue and STOP — don't
  silently rewrite it into something else.
- **Scope = ONE coherent PR.** If the crate is huge and the findings sprawl, do the **smallest
  self-contained slice** (one module / one invariant / one test gap) and record the rest in the
  ledger for the next iteration. Never open a sprawling 4k-line refactor PR.
- **Boundary invariants are findings when violated, never tools for "cleanup":**
  - Reborn crates must not depend on the root `ironclaw` crate (`reborn_dependency_boundaries`
    enforces this — never weaken it).
  - `ironclaw_webui_v2` consumes only `RebornServicesApi`; substrate handles stay private to
    factories/composition.
  - Identity from the authenticated caller, never the request body.
  - Dual-backend (PostgreSQL + libSQL) parity for persistence.
  - **LLM data is never deleted** — "cleanup" means evicting caches, never deleting DB rows.
- Repo code style: no `.unwrap()`/`.expect()` in production code, `thiserror`, prompt templates
  in files, `debug!` not `info!` for internals. **Never add a log line that prints prompt
  content, completions, key material, or secrets** — ids/counts/sizes/durations/error-types only.
  Treat any such finding as a hard blocker and fix it.

## 3. Load the crate & branch
- Read the crate's **`CLAUDE.md`** (the spec — code follows spec; spec is the tiebreaker) and/or
  `README.md` (note the absence of either as a finding), `Cargo.toml` (deps, features, `pub`
  surface), and its `lib.rs`/`main.rs` to map the module tree.
- **Small crate (`src` < ~2k lines): read every source file in full** so the review is complete,
  not sampled. **Large crate: read the spec, the public API (`lib.rs` re-exports), and the
  largest / hottest modules**, and scope the PR to the slice you fully understand (§2).
- Branch off `main`: `git switch -c deslop/<crate-short-name>` (e.g. `deslop/reborn-config`).
  Confirm it builds before you start.

## 4. Fan out the review — PARALLEL sub-agents (this is the core of the loop)
Spawn the following review sub-agents **concurrently** (issue them in a single message so they
run in parallel via the Agent tool). Give **each** sub-agent the crate path, tell it to **read
the crate's source itself**, and require it to return a **prioritized, structured findings list**
where every finding names the **file/lines**, the **smell**, and a concrete **remedy** (not just
the smell). Run **at least** these four angles:

1. **Thermo-nuclear quality reviewer.** Instruct it: *read
   `.claude/skills/thermo-nuclear-code-quality-review/SKILL.md` and apply it verbatim to the
   entire crate as if it were one large diff* (the skill is `disable-model-invocation`, so it
   must follow the rules directly, not try to invoke it). Hunt for **code-judo** reframings that
   delete whole branches/helpers/modes; flag any file over ~1k lines that should be decomposed;
   flag ad-hoc conditionals, thin wrappers, needless optionality/casts, dead/speculative
   branches, canonical-helper duplication, and logic in the wrong layer. Output in the skill's
   priority order.

2. **Paranoid senior architect.** Give it this prompt, framed at the whole crate:
   > You are a paranoid senior engineer and architect on this project. A junior engineer has
   > shipped this crate as if it were a single pull request, and your job is to carefully review
   > it for **correct architecture, completeness, and security**, and to ensure that **testing
   > covers all edge cases and exercises the whole wiring — not just local unit tests**. Where
   > the code is a **bug-fix-shaped** pattern, the goal is that this *whole class* of problem can
   > never recur. Where it is a **feature**, ensure there will be **no future bug reports**
   > against it. Enumerate concrete gaps (missing error handling, unhandled edge cases,
   > race/replay/atomicity hazards, tenant-isolation or auth-boundary leaks, secrets/privacy
   > leaks in logs or errors, partial-state bugs, dual-backend divergence, integration paths
   > never exercised end-to-end) with file/line and the fix or the test that would close them.

3. **Interface / spec / invariants auditor.** Instruct it to verify: the **public API is not
   over-exposed** (every `pub`/`pub(crate)` item that doesn't need to be public should be
   tightened — flag leaked internals, `pub` fields that should be private, raw substrate handles
   escaping the facade, types re-exported needlessly); the **crate `CLAUDE.md` (and README if
   present) exists and is accurate** (documents the crate's purpose, the real public surface, and
   its core invariants — flag drift between spec and code in *both* directions); and the crate's
   **core invariants actually hold** in code (find the documented/implied invariants and check
   each is enforced at its boundary, not assumed). Output: over-exposed items to seal, spec
   corrections, and any invariant that is stated-but-unenforced or enforced-but-undocumented.

4. **Test-coverage & wiring auditor.** Instruct it to map what the crate's tests actually cover
   vs. what they should: identify **untested public functions, error paths, and edge cases**, and
   — critically — whether the crate is only **unit-tested in isolation** when it has a real
   **integration / e2e path** that is never exercised through the full wiring. Apply the repo
   rule "**test through the caller, not just the helper**" (`.claude/rules/testing.md`): a helper
   that gates a side effect needs a test that drives the call site. Check whether the crate's
   contracts appear in `scripts/reborn-e2e-rust.sh`'s test groups and whether persistence paths
   are covered for **both** backends. Output: the specific **unit and e2e tests that are
   missing**, each with the behavior it would pin down.

Consider **additional angles** when the crate warrants it (spawn them too): a
dependency/feature-flag hygiene pass (the Reborn feature matrix — `webui-v2-beta`,
`openai-compat-beta`, `postgres`, `libsql` — breeds dead `cfg` branches), a
concurrency/panic-safety pass, or a perf-smell pass. Right-size the fan-out to the crate.

While the agents run, kick off `cargo build -p <crate> --all-targets` **in the background** to
confirm a green baseline — don't block on it.

**Treat every agent finding as a LEAD, not a fact.** The sub-agents read excerpts, not the whole
workspace, so they routinely disagree with each other and get **external-usage claims wrong** —
a `pub` item called "dead" or "test-only" may have production callers in another crate, and
vice-versa. Any finding that asserts "X has no callers / only test callers / is over-exposed /
is unused" is a hypothesis you MUST verify in §5 before acting on it.

## 5. Synthesize the findings — then VERIFY before acting
Collect every sub-agent's findings into **one deduplicated, prioritized list** (use the
thermo-nuclear ordering: structural regressions → missed simplifications → spaghetti →
boundary/type → file size → modularity → legibility, then fold in the architect's
correctness/security/coverage gaps and the interface/spec/invariant items). Merge overlapping
findings; drop low-value nits if bigger structural issues exist. For each kept finding, decide
the **remedy** and whether it fits in this PR's scope (§2) or should be recorded for a later
iteration.

**Before you act on any finding that deletes or seals a public item, grep the workspace yourself
to confirm the caller set** — the agents' external-usage claims (§4) are unreliable. For each
`pub`/`pub(crate)` item, variant, or function you plan to **delete** or **tighten**:
```
grep -rn "<ItemName>" crates/ src/ --include=*.rs | grep -v "crates/<this-crate>/"
```
Distinguish **construction** from **field/value reads**, and **production** call sites from
**test** ones — a `pub` field with a production reader in another crate is a different decision
than one read only by a test. Only the grep result — not an agent's say-so — decides whether
something is dead, over-exposed, or safe to seal.

## 6. Apply the fixes
Implement every in-scope finding, applying the skill's **preferred remedies** (delete a layer,
collapse duplicate branches into one flow, extract a helper/module, replace a condition chain
with a typed dispatcher, move logic to its canonical home, **tighten over-exposed `pub`**,
**fix/author the crate spec**, **add the missing unit/e2e tests**). Match the crate's surrounding
style, comment density, and idiom.

If a finding is a **deliberate, justified tradeoff** (duplication genuinely clearer than the
abstraction, an invariant deliberately checked elsewhere), don't force it — record the
justification in the PR body. The bar is "every finding *resolved*" — fixed, or explicitly
justified.

**Let blast radius (from the §5 grep) decide scope, and keep the PR single-crate by default.** A
seal/delete whose fallout is confined to this crate (+ its own tests) is in-scope — just do it.
But when tightening a `pub` item would force edits to **other crates'** production code, that's a
cross-crate change masquerading as de-slop: prefer to **defer it** (record in the PR body +
ledger §9, optionally file an issue) rather than drag this PR into three crates. Likewise defer
anything that **changes the crate's observable contract, its wire/event format, or adds a
dependency** — those want their own focused, deliberately-reviewed PR. Fix the clean, contained
findings now; record the ripple-y ones.

### Anti-reward-hacking guard — inspect YOUR OWN diff
Reject and redo your own change if it:
- adds a net-new `#[ignore]`, `todo!()`/`unimplemented!()`, or `#[allow]` **just to pass a gate
  or quiet a finding**;
- **weakens** any existing test assertion, boundary test, or descriptor contract;
- "addresses" a coverage finding with a test that asserts nothing, or a spec that documents the
  bug instead of fixing it;
- pushes a file from **<1k to >1k lines** without strong justification;
- adds a log line that violates the privacy rule (§2).

The job is to make the crate **cleaner, better-documented, and better-tested** — not to silence
the detectors. A new test must actually pin behavior; a sealed `pub` must still compile the
workspace.

## 7. Gates — ALL must pass locally (or be CI-deferred with a stated reason)
```
cargo fmt
cargo clippy --all --benches --tests --examples --all-features   # zero warnings
cargo test -p <crate>            # plus -p each crate touched by fallout
scripts/reborn-e2e-rust.sh       # deterministic Reborn e2e gate
```
Sealing a public item can break **other** crates — clippy `--all` and the e2e gate catch most of
that; for wide fallout run a full `cargo test`. Touched `ironclaw_webui_v2_static` JS →
`node --check` each file. PostgreSQL integration tests run if available, else note "verified in
CI (`reborn-integration`)". Everything must be **green after your fixes**.

## 8. Self thermo-nuclear pass + land — stop at PR
1. **Re-run the thermo-nuclear standards on your OWN diff** (read the SKILL, apply directly). It
   must clear the **Approval Bar**: no structural regression, no missed obvious simplification,
   no unjustified file-size explosion, no spaghetti branching, no hacky/magical abstraction, no
   needless wrapper/cast/optionality, no boundary leak or canonical-helper duplication. If a new
   finding appears, loop back to §6.
2. **Commit:** `refactor(<crate>): de-slop — <one-line summary>` (use `refactor` for structural
   changes, `test` if the PR is mostly added coverage, `docs` if mostly spec, `fix` if you
   corrected a real bug a finding exposed). End with your standard `Co-Authored-By: Claude …`
   trailer.
3. **Push and open the PR** with `gh`. PR body states: the **crate**, the **findings grouped by
   angle** (quality / architecture & security / interface & spec & invariants / coverage), **what
   you changed** for each, any **deliberate tradeoff** left in place with its justification,
   which **gates ran locally** and what is **CI-deferred**, and any **out-of-scope findings
   deferred** to a later iteration. End with the Generated-with trailer.
4. **STOP. Do not merge.**

## 9. Record
Append one line to `.work/deslop-ledger.md`: crate · branch · PR# · gate status · what was fixed.
Add the crate to the **DESLOPPED** list so the next iteration skips it. If you only de-slopped a
**slice** of a large crate (§2), record the **remaining slices** so the next iteration continues
it rather than marking the whole crate done. If the review concluded the crate's *premise* is
wrong (§2) and you pushed no fixes, record that verdict instead so a human can decide.

Then end the turn; the loop paces the next iteration.
