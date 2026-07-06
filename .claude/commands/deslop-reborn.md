---
description: One iteration of the IronClaw Reborn de-slop loop — take ONE Reborn crate, fan out parallel review sub-agents (thermo-nuclear quality, paranoid architect, interface/contract/invariants, test-coverage/wiring), synthesize, apply fixes/refactors/missing tests, open a PR. Stop.
disable-model-invocation: true
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo build:*), Bash(git:*), Bash(gh:*), Bash(grep:*), Bash(rg:*), Bash(ls:*), Bash(wc:*), Bash(scripts/check-boundaries.sh:*), Bash(scripts/reborn-e2e-rust.sh:*), Read, Grep, Glob, Edit, Write, Agent
argument-hint: "[crate name, e.g. ironclaw_turns]"
---

You are running **ONE iteration** of the IronClaw **Reborn de-slop** loop. Take exactly one **Reborn
crate** (in `crates/`) that has **not been de-slopped yet**, review it *deeply and from multiple
angles using parallel sub-agents*, then **land one coherent PR** that fixes the findings —
simplifications, refactors, contract/README/invariant corrections, and any **missing unit/integration
tests** needed to keep the component correct forever. Then **STOP** — a human merges. Do not merge
anything.

Unlike `/review-crate` (a read-only audit) and `/review-pr` (which reviews an open PR's diff), this
loop reviews a **whole crate as it stands on the integration branch** and improves it. The unit of
work is a **Reborn crate**, not a diff.

If the user passed a crate name as an argument (`$ARGUMENTS`, e.g. `ironclaw_turns`), de-slop **that**
crate and skip the selection cascade in §1 — but still run the §1 Reborn/legacy check to confirm it is
a legitimate Reborn target. Otherwise pick one per §1.

## 0. Environment & state
- **Reborn-only.** New feature and quality work targets the Reborn stack in `crates/`, never the v1
  `src/` monolith (root CLAUDE.md, "Where to Build — Reborn-First"). This loop only touches `crates/`
  Reborn crates. It must **skip the legacy enclave** — crates that serve *only* the retiring v1
  monolith (`ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`, `ironclaw_embeddings`; `ironclaw_engine`
  is already removed). Don't trust that hardcoded list — it drifts. Verify each candidate's status
  with the orientation recipe (§1) before picking it.
- **Local gate reality.** You can prove `cargo fmt`, `cargo clippy`, per-crate `cargo test -p <crate>`,
  the workspace unit-test tier, and the architecture-boundary test (`cargo test -p ironclaw_architecture`)
  locally. The **integration tier** (`cargo test --features integration`) needs a running PostgreSQL;
  the **live tier** (`-- --ignored`) needs Postgres + LLM API keys; **Reborn e2e**
  (`scripts/reborn-e2e-rust.sh`) may need Docker. If a fix touches those paths, implement it fully and
  mark the gate **"CI-deferred (needs Postgres/Docker/keys)"** in the PR body. Never weaken or delete
  integration/e2e coverage just because it can't run in this environment. (Tiers: `.claude/rules/testing.md`.)
- Read `.work/deslop-ledger.md` (create the file and `.work/` dir if missing; add `.work/` to
  `.gitignore` if it isn't already — the ledger is local scratch, keep it out of PRs). The
  **DESLOPPED** list records crates this loop has already passed through — **skip them** so two
  iterations don't re-chew the same crate. Also run `gh pr list` to see in-flight PRs: **don't pick a
  crate an open PR is actively reshaping** (you would collide on the same files); prefer a crate no
  open PR holds.

## 1. Pick ONE un-de-slopped Reborn crate — stop at first viable
List the crates (`ls crates/`). A crate is a **candidate** when ALL hold:
- **it is a Reborn crate, not legacy-enclave.** Verify: `grep -rl "<crate_name>" crates/*/Cargo.toml Cargo.toml`
  — if the **only** consumer is the root `Cargo.toml` package, it's v1-only; skip it. When unsure,
  consult the `ironclaw-reborn-orientation` skill (it maps which side each crate is on).
- **not already in the ledger DESLOPPED list** (§0),
- **not the hot surface of an open PR** (§0) — leave those to the build/review loops,
- it has real source to review (skip thin aggregator/facade crates with ~no `src` — though their
  *tests* are fair game for the coverage angle).

**Selection bias:** prefer **smaller / leaf crates first** — they fit entirely in context, gate
cleanly, and merge independently. A crate whose `src` is **under ~2k lines** can and SHOULD be read in
full (§3). Clear the small, high-leverage crates before taking on the giants (e.g.
`ironclaw_reborn_composition`, `ironclaw_product_workflow`, `ironclaw_gateway`-scale surfaces need
targeted reading, not a full load, and may warrant splitting the de-slop across iterations
module-by-module). When unsure, prefer a crate that is **load-bearing for invariants** (turns,
dispatcher, authorization, approvals, secrets, run_state, event store) over a purely mechanical one —
a slop fix there protects the whole system.

If **every** Reborn crate is ledger-recorded or PR-held, this is a **no-de-slop iteration**: record
"nothing to de-slop" in the ledger and STOP. Do **not** manufacture a marginal PR to fill the gap.

## 2. FENCES (hard)
- **De-slop the crate — do not redesign its feature or change its observable contract.** Your mandate
  is quality (structure, decomposition, spaghetti, abstraction, file size), **interface hygiene**
  (public surface not over-exposed; `AGENTS.md`/`CLAUDE.md`/`CONTRACT.md`/`README.md` accurate),
  **invariant integrity**, and **test coverage** — not re-scoping what the crate does. If you conclude
  the crate's *premise* or *layer* is wrong, **file a GitHub issue** with the verdict (§6 "Deferred
  findings") and STOP — don't silently rewrite it into something else. When a change would add a trait, a
  crate, a dependency edge, or a re-export, apply the `ironclaw-reborn-architecture-review` skill first.
- **Respect the Reborn layering.** Product/runtime composition flows **downward** through typed
  contracts (`crates/AGENTS.md` → "Dependency Mental Model"). Never introduce an upstream dependency in
  a lower-level crate to make a fix convenient — `cargo test -p ironclaw_architecture` enforces the
  boundaries and will fail. A de-slop that needs a new upward edge is out of scope; record it (§9).
- **Scope = ONE coherent, single-crate PR.** If the crate is huge and the findings sprawl, do the
  **smallest self-contained slice** (one module / one invariant / one test gap) and record the rest in
  the ledger for the next iteration. Never open a sprawling multi-thousand-line refactor PR. Prefer
  **deleting** complexity over accreting layers.
- **Privacy & logging doctrine (root CLAUDE.md — hard blockers).**
  - **Never add a log line that prints a prompt, completion, key material, decrypted content, secret,
    or raw bytes** — log ids/counts/sizes/durations/error-types only. Treat any such existing line you
    find as a finding and fix it.
  - **`info!`/`warn!` corrupt the REPL/TUI.** Background tasks (reflection, trace analysis, heartbeat)
    must NEVER use `info!`. Use `debug!` for internal diagnostics; reserve `info!` for user-facing
    status the REPL intentionally renders. Flag and fix violations.
  - **LLM data is never deleted.** Context, reasoning, tool calls, messages, events, and steps are the
    most valuable data in the system. Never strip, truncate, or delete them from the database; a
    "simplification" that drops retained LLM data is a blocker, not a fix. In-memory caches may be
    evicted; the database is the source of truth.
- **Production-code conventions (root CLAUDE.md).** No `.unwrap()`/`.expect()` outside `#[cfg(test)]`;
  errors via `thiserror` with context; `crate::` imports (not `super::`) in non-test code; multi-line
  prompt templates live in `prompts/*.md` loaded via `include_str!()`, never inline Rust constants. A
  de-slop that *introduces* any of these is a self-inflicted finding — reject it (§6 guard).

## 3. Load the crate & branch
- Read the crate's guidance in order: `crates/<crate>/AGENTS.md`, then `CLAUDE.md`, `CONTRACT.md`,
  `README.md` (note the absence of an expected one as a finding), then `Cargo.toml` (deps, features,
  `pub` surface) and `src/lib.rs`/`src/main.rs` to map the module tree. Pull the matching
  `docs/reborn/contracts/*.md` if the crate has a cross-crate contract.
- **Small crate (`src` < ~2k lines): read every source file in full** so the review is complete, not
  sampled. **Large crate: read the guidance, the public API (`lib.rs` re-exports), the `CONTRACT.md`,
  and the largest / hottest modules**, and scope the PR to the slice you fully understand (§2).
- Branch off the freshest integration base: `git fetch origin && git switch -c deslop/<crate-short-name> origin/main`
  (e.g. `deslop/turns`). Confirm it builds before you start (`cargo build -p <crate> --all-targets`).
  The repo's PR target is **`main`** — you will `gh pr create --base main` in §8 (confirm against
  the base shown by the repo config if unsure).

## 4. Fan out the review — PARALLEL sub-agents (this is the core of the loop)
Spawn the following review sub-agents **concurrently** (issue them in a single message so they run in
parallel via the Agent tool). Give **each** sub-agent the crate path, tell it to **read the crate's
source itself**, and require it to return a **prioritized, structured findings list** where every
finding names the **file/lines**, the **smell**, and a concrete **remedy** (not just the smell). Run
**at least** these four angles:

1. **Thermo-nuclear quality reviewer.** Instruct it: *read
   `.claude/skills/thermo-nuclear-code-quality-review/SKILL.md` and apply it verbatim to the entire
   crate as if it were one large diff* (the skill is `disable-model-invocation`, so it must follow the
   rules directly, not try to invoke it). Hunt for **code-judo** reframings that delete whole
   branches/helpers/modes; flag any file over ~1k lines that should be decomposed; flag ad-hoc
   conditionals, thin wrappers, needless optionality/casts, dead/speculative branches, canonical-helper
   duplication, and logic in the wrong layer. Output in the skill's priority order.

2. **Paranoid senior architect.** Tell it to first read the `ironclaw-reborn-architecture-review`
   skill (`.claude/skills/ironclaw-reborn-architecture-review/SKILL.md`) so its abstraction/boundary
   judgments match this repo, then give it this prompt framed at the whole crate:
   > You are a paranoid senior engineer and architect on IronClaw Reborn. A junior engineer has shipped
   > this crate as if it were a single pull request, and your job is to carefully review it for
   > **correct architecture, completeness, and security**, and to ensure that **testing covers all edge
   > cases and exercises the whole wiring — not just local unit tests**. Check that the crate sits at
   > the right layer, adds no unjustified trait/dependency/re-export, and never reaches upstream. Where
   > the code is a **bug-fix-shaped** pattern, ensure this *whole class* of problem can never recur.
   > Where it is a **feature**, ensure there will be **no future bug reports** against it. Enumerate
   > concrete gaps (missing error handling, unhandled edge cases, race/replay/overflow hazards,
   > security/privacy leaks, partial-state/atomicity bugs, integration paths never exercised
   > end-to-end) with file/line and the fix or the test that would close them.

3. **Interface / contract / invariants auditor.** Instruct it to verify: the **public API is not
   over-exposed** (every `pub`/`pub(crate)` item that doesn't need to be public should be tightened —
   flag leaked internals, `pub` fields that should be private, types re-exported needlessly); the
   crate's **guidance is accurate** (`AGENTS.md`/`CLAUDE.md`/`CONTRACT.md`/`README.md` document the real
   purpose, public surface, and core invariants — flag drift between guidance and code, and any missing
   file the crate should have); and the crate's **core invariants actually hold** in code (find the
   documented/implied invariants and check each is enforced at its boundary, not assumed). Output:
   over-exposed items to seal, guidance corrections, and any invariant that is stated-but-unenforced or
   enforced-but-undocumented.

4. **Test-coverage & wiring auditor.** Tell it to first read the `ironclaw-reborn-testing` skill and
   `.claude/rules/testing.md` (test tiers; test-through-the-caller), then map what the crate's tests
   actually cover vs. what they should: identify **untested public functions, error paths, and edge
   cases**, and — critically — whether the crate is only **unit-tested in isolation** when it has a real
   **integration / e2e path** (through a `*_handler`, `factory::create_*`, or the runtime lane) that is
   never exercised through the full wiring. Output: the specific **unit and integration tests that are
   missing** to guarantee the component always works correctly, each with the behavior it would pin down
   and the tier it belongs in.

Consider **additional angles** when the crate warrants it (spawn them too): a dependency/feature-flag
hygiene pass, a concurrency/`unsafe`/panic-safety pass, or a perf-smell pass. Right-size the fan-out to
the crate — a 200-line crate needs fewer angles than a 3k-line runtime crate.

While the agents run, kick off `cargo build -p <crate> --all-targets` **in the background** to confirm
a green baseline — don't block on it.

**Treat every agent finding as a LEAD, not a fact.** The sub-agents read excerpts, not the whole
workspace, so they routinely disagree with each other and get **external-usage claims wrong** — they
will call a `pub` item "dead" or "test-only" when it has production callers in another crate, and
vice-versa. Any finding that asserts "X has no callers / only test callers / is over-exposed / is
unused" is a hypothesis you MUST verify in §5 before acting on it.

## 5. Synthesize the findings — then VERIFY before acting
Collect every sub-agent's findings into **one deduplicated, prioritized list** (use the thermo-nuclear
ordering: structural regressions → missed simplifications → spaghetti → boundary/type → file size →
modularity → legibility, then fold in the architect's correctness/security/coverage gaps and the
interface/contract/invariant items). Merge overlapping findings; drop low-value nits if bigger
structural issues exist. For each kept finding, decide the **remedy** and whether it fits in this PR's
scope (§2) or should be recorded for a later iteration.

**Before you act on any finding that deletes or seals a public item, grep the workspace yourself to
confirm the caller set** — the agents' external-usage claims (§4) are unreliable and frequently
conflict. For each `pub`/`pub(crate)` item, variant, or function you plan to **delete** or **tighten**:
```
grep -rn "<ItemName>" crates/ src/ --include=*.rs | grep -v "crates/<this-crate>/"
```
Distinguish **construction** from **field/value reads**, and **production** call sites from **test**
ones — a `pub` field with a production reader in another crate is a different decision than one read
only by a test. Only the grep result — not an agent's say-so — decides whether something is dead,
over-exposed, or safe to seal. (Include `src/` in the grep: a Reborn crate may still have a v1 consumer
during the migration.) **Watch for name collisions** — if the symbol shares a name with an unrelated
type elsewhere (e.g. a v1 `db::UserRecord` vs. a Reborn crate's `UserRecord`), the grep floods with
false hits; disambiguate by the import path / `use` line, not the bare name, before calling it dead.
Where grep genuinely can't settle it — e.g. a blanket `impl Trait for Arc<T>` that method resolution
may or may not select — **don't guess: let the §7 workspace build decide** (delete it on the branch;
if the workspace still compiles, it was dead).

**If verification was confusing, that confusion IS a finding — fix the code, don't just route around
it.** When grep floods on a name collision, when you can't tell which of two same-named types a call
site means, when tracing a value across layers required re-reading three files, or when an item's
purpose was unclear until you found (or failed to find) its caller — the defect is in the code's
**naming or comments**, and it is in-scope for this de-slop. Rename the colliding/ambiguous item so the
name is load-bearing for grep and agents (unique names are an invariant — `.claude/rules/type-placement.md`),
or add the one-line comment that would have saved the trace. Record it as a finding alongside the ones
the sub-agents raised; the friction you just hit is the next reader's friction too.

## 6. Apply the fixes
Implement every in-scope finding, applying the skill's **preferred remedies** (delete a layer, collapse
duplicate branches into one flow, extract a helper/module, replace a condition chain with a typed
dispatcher, move logic to its canonical home, **tighten over-exposed `pub`**, **fix/author the crate
guidance**, **rename the ambiguous/colliding item or add the clarifying comment** that the §5 verification
proved was missing, **add the missing unit/integration tests**). Match the crate's surrounding style,
comment density, and idiom.

If a finding is a **deliberate, justified tradeoff** (duplication genuinely clearer than the
abstraction, an invariant deliberately checked elsewhere), don't force it — record the justification in
the PR body. The bar is "every finding *resolved*" — fixed, or explicitly justified.

**Let blast radius (from the §5 grep) decide scope, and keep the PR single-crate by default.** A
seal/delete whose fallout is confined to this crate (+ its own tests) is in-scope — just do it. But when
tightening a `pub` item would force edits to **other crates'** production code (routing their reads
through a new accessor), or would require a new dependency edge / re-export (which
`cargo test -p ironclaw_architecture` guards), that's a cross-crate change masquerading as de-slop:
**defer it** rather than drag this PR into three crates. Likewise defer anything that **changes the
crate's observable contract or adds a dependency**. Fix the clean, contained findings now; hand off the
ripple-y ones per "Deferred findings" below.

### Deferred findings — file them as GitHub issues, don't just mention them
A **correctness, security, race, data-integrity, or architecture** finding that you cannot safely fix
inside this single-crate de-slop PR (because it changes observable behavior, needs a new dependency, or
ripples across crates) does not get buried in a PR-body paragraph — **file it as a GitHub issue** so it
is tracked independently of whether this PR merges:
```
gh issue create --title "<crate>: <one-line defect>" \
  --body "<what/where (file:line) · why it's real · the failing scenario · the fix or the test that would close it · why it was out of scope for the de-slop PR>"
```
Label it if the repo uses labels (`gh label list`). File **one issue per distinct defect** (don't batch
unrelated findings). Then reference the issue number in both the PR body (§8) and the ledger (§9). The
premise-wrong verdict (§2) is filed the same way. Pure-quality deferrals (a bigger refactor that's just
structural, no correctness stake) may instead be recorded as a "remaining slice" in the ledger without an
issue — issues are for **substantive** findings a human needs to see, not for de-slop backlog.

### Anti-reward-hacking guard — inspect YOUR OWN diff
Reject and redo your own change if it:
- adds a net-new `#[ignore]`, `todo!()`/`unimplemented!()`, `.unwrap()`/`.expect()` in production code,
  or `#[allow(...)]` **just to pass a gate or quiet a finding**;
- introduces `super::` imports in non-test code, or inlines a multi-line prompt template as a Rust
  constant instead of a `prompts/*.md` + `include_str!()`;
- **weakens** any existing test assertion or deletes/truncates retained LLM data;
- "addresses" a coverage finding with a test that asserts nothing (no meaningful `assert!`), or fixes
  guidance by documenting the bug instead of fixing it;
- pushes a file from **<1k to >1k lines** without strong justification;
- adds a log line that violates the privacy/logging doctrine (§2).

The job is to make the crate **cleaner, better-documented, and better-tested** — not to silence the
detectors. A new test must actually pin behavior; a sealed `pub` must still compile the workspace.

## 7. Gates — ALL must pass locally (or be CI-deferred with a stated reason)
```
cargo fmt --all --check
cargo clippy -p <crate> --all-targets --all-features -- -D warnings
cargo test  -p <crate>
cargo test  -p ironclaw_architecture          # dependency-boundary enforcement for crates/
cargo build --workspace --all-targets         # sealing a pub can break OTHER crates — this catches it
cargo clippy --all --benches --tests --examples --all-features
```
Sealing a public item can break **other** crates — the workspace build/clippy catches that; fix the
fallout or keep the item public with a note. If the crate has an **integration** path, run
`cargo test --features integration` when Postgres is reachable; if it has a **Reborn e2e** path
(turns, runtime lanes, host services, authorization, approvals, networking, secrets, product
workflow, capability dispatch), run `scripts/reborn-e2e-rust.sh`. Any tier you cannot run here (needs
Postgres/Docker/keys) → note it **"CI-deferred"** in the PR body. Everything runnable must be **green
after your fixes**.

## 8. Self thermo-nuclear pass + land — stop at PR
1. **Re-run the thermo-nuclear standards on your OWN diff** (read the SKILL, apply directly). It must
   clear the **Approval Bar**: no structural regression, no missed obvious simplification, no
   unjustified file-size explosion, no spaghetti branching, no hacky/magical abstraction, no needless
   wrapper/cast/optionality, no boundary leak or canonical-helper duplication. If a new finding appears,
   loop back to §6.
2. **Commit:** `refactor(<crate>): de-slop — <one-line summary>` (use `refactor` for structural
   changes, `test` if the PR is mostly added coverage, `docs` if mostly guidance, `fix` if you
   corrected a real bug a finding exposed). End the message with:
   `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
   (The commit-msg hook requires a regression test with every `fix` — write it test-first, §Testing
   Discipline in CLAUDE.md.)
3. **Push and open the PR** with `gh pr create --base main`. PR body states: the **crate**, the
   **findings grouped by angle** (quality / architecture & security / interface & contract & invariants
   / coverage), **what you changed** for each, any **deliberate tradeoff** left in place with its
   justification, which **gates ran locally** and what is **CI-deferred** (integration/e2e needing
   Postgres/Docker/keys), and the **out-of-scope findings you filed as issues** (§6) linked by number
   (`#NNNN`). End with:
   `🤖 Generated with [Claude Code](https://claude.com/claude-code)`
4. **STOP. Do not merge.**

## 9. Record
Append one line to `.work/deslop-ledger.md`: crate · branch · PR# · gate status · what was fixed ·
**issues filed** (`#NNNN` from §6). Add the crate to the **DESLOPPED** list so the next iteration skips
it. If you only de-slopped a **slice** of a large crate (§2), record the **remaining slices** so the
next iteration continues it rather than marking the whole crate done. If the review concluded the crate's
*premise* is wrong (§2) and you pushed no fixes, record that verdict and the issue number instead so a
human can decide.

Then end the turn; the loop paces the next iteration.
