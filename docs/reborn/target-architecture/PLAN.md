# Target Architecture — Proposed Execution Plan

**What this is:** the recommended way to start and keep tackling the restructure — waves, gates, PR-sizing rules, and decision points. It sequences the workstreams defined in [CHECKLIST.md](CHECKLIST.md) (WS0–WS12); the checklist is the *what*, this is the *when and how*. Nothing here is sacred except the four load-bearing ordering constraints, which are marked ⚠.

**Operating principles** (learned from the July train, and from what went wrong in it):

1. **Owner-by-owner, never big-bang.** The extractions that worked (#6529…#6669) moved one coherent owner per PR. The four god-crate narrowings are *many* PRs each, not one.
2. **Move-only PRs are behavior-free and say so.** `git mv` + path updates + guidance updates, nothing else in the diff. Splits and semantic changes never share a PR with moves.
3. **Tests and guidance travel with the change** — every PR updates the crate guides + family AGENTS.md + boundary rules it invalidates, in the same diff (the audit's clearest lesson: guidance drift is how agents get confused).
4. **Deletions use the un-masking discipline**: full unfiltered suites for every touched crate; a surfaced failure is a candidate behavior to preserve, not a test to edit.
5. **`main` stays shippable after every PR.** No wave leaves a half-flipped dependency edge; each PR ends with the architecture suite green and the exception count monotonically non-increasing.
6. **Decision gates are cheap to hold, expensive to skip.** The **[decision]** items (Strategy confirmation, rename batch, triggers/hooks SQL, trust's signed-registry path, skills revival, identity binding-store, tools default-members) get called before their wave starts — most are one Slack thread each.

---

## Wave 0 — "Start now" (no design risk, immediate payoff)

*Everything here is independently landable today and shrinks the problem for every later wave.*

1. **WS0 de-wildcard of `host_api`'s prelude** ⚠ — the single prerequisite most later rows lean on. Behavior-free, mechanical, big diff but trivially reviewable.
2. **WS8 safe deletions, first tranche** — the zero-consumer items with the smallest blast radius: `dispatcher`, `embeddings`, `llm::reasoning`, `common::trust_boundary`, `events` jsonl helpers, `outbound`/`approvals` dead traits, `event_projections`' three dead subsystems (this alone shrinks its dep set to events+host_api), unused dep edges, root `fuzz/`, `auth::loopback_oauth`, gating `auth::fakes`. Each with the un-masking discipline.
3. **WS11 drift hotfixes that mislead agents today** — the references to seven nonexistent crates, `build_reborn_services`, `NetworkPolicyDecider`, the composition guide's phantom `src/webui/` section, stale feature-gating claims. These don't wait for the restructure; they are wrong *now*.
4. **WS0 baselines + WS10 §11.2.2 exception ratchet** (armed at 20, must only go down) and the §11.2.7 include-scan in warn mode (it will fail until WS2 — that's the point; it makes the debt visible).
5. **Decision round #1** **[decision]**: confirm Strategy B + the rename batch scope + tools default-members. One thread with Illia.

*Exit criteria: exception count still 20 but ratcheted; dead-surface tranche gone; prelude de-wildcarded; team sign-off recorded.*

## Wave 1 — Contracts (WS1) — the leverage wave

*Creates the three contracts crates and completes the turn vocabulary. Everything afterward gets cheaper.*

- Order inside the wave: turn vocabulary → `loop_contracts` → `extension_contracts` → `product_contracts` → evidence-mint consolidation ⚠ (security-sensitive; its refute-tests land in the same PR) → `common` narrowing → the two single-symbol product edges (`runner`, `loop_host`).
- Each new crate lands with its §11.2.3 allowlist + §11.2.4 port-location scan in the same PR (new-crate-adds-rule discipline).
- **Milestone:** exceptions 20 → 12 (all W4.3 + `auth→turns` gone); `agent_loop` passes contracts-only with zero exceptions; webui/openai/channel crates *can* now compile against contracts (the flips happen in Wave 2).

## Wave 2 — Extensions + product flips (WS2 + WS5)

*The most ordering-sensitive wave.* ⚠ **Port inversions land before the layer flip**: extension_host implements `product_contracts` ports first; only when its `ironclaw_product` dep is gone does its layer move to loops (otherwise it cannot compile). Same discipline for `operator` and `webui`/`openai_compat` flips.

- Sequence: port flips (extension_host, operator, webui, openai_compat) → `extension_manager` split (the #6616/#6669 inventory moves as a unit, like it arrived) → strays out (pairing routes→webui, skill-learning seam, bundled skills) → include_str kills + package colocation + telegram merge → re-layer extensions/extension_host → naming-trap fixes (conversations/threads) → attachments widening.
- **Milestone:** `extension_host→product` edge gone; packages self-contained under `extensions/packages/`; Discord-proof (§10.1) is now literally true — a new channel touches one package dir + one binding line.

## Wave 3 — Kernel + loop narrowing (WS3 + WS4, non-gated parts)

- Sequence: first-party tools → package (registrar pattern; one tool family per PR) → `sandbox` lane merge (no production behavior — verify at land time) → `mcp` contracts flip → obligations/builder internal splits → secrets direct-consumer tightening ⚠ (port replacements before edge removal) → runner sheds (composition functions out, model gateway → loop_host, tool disclosure) → re-layer runner/hooks/processes → `wit/` move.
- **Milestone:** exceptions 12 → 0. The ratchet pins it. `host_runtime` has no Docker/DB-driver cone; runner is the thin loop-hosting adapter.

## Wave 4 — Composition, app, domains (WS6)

*Overlaps Waves 2–3 freely — every eviction is independent.* This wave is deliberately aligned with **PR #6691 (IN FLIGHT)**: if it lands first, several evictions become rebases instead of new work; if it doesn't, this wave subsumes its intent. Either way, coordinate with its author before starting the same modules.

- Composition evictions one owner per PR (the §6.10.1 inventory); `local_dev` misnomer retired; `RebornRuntime` slimming; config vendor-section removal with its compat window; CLI vendor-resolution shed; the rename batch (whatever Decision round #1 approved) as pure-rename PRs.
- **Milestone:** composition reads as assembly (its mass ratchet re-baselined at the new floor); config has no vendor sections; renames done.

## Wave 5 — Physical family moves (WS7)

*Deliberately late for split crates, flexible for stable ones.* Two allowed modes: (a) **move-with-your-milestone** — a crate moves when its narrowing lands (preferred; one churn each); (b) **early batch-move** of untouched retain-as-is crates (substrate/events/domains leaves) any time after Wave 0 if the team wants the tree visible sooner — it's pure `git mv` churn, decide by taste **[decision]**.

- Last move lands with the §11.2.1 family⇄layer test and the tree-comparison script.
- **Milestone:** `crates/` matches PROPOSAL §5 exactly.

## Wave 6 — Process-journal-gated work (WS9) **[#6696 gate]**

*Blocked until #6696-or-equivalent is approved and landed with its import/rollback contract.* Then: processes widening, runner scheduler/await-edge shed, approvals absorption, `run_state` deletion. Until the gate opens, `run_state` carries its freeze charter and nothing else waits on this wave — it is the only wave with an external dependency.

## Continuous tracks (run alongside every wave)

- **Enforcement (WS10):** every rule lands with or before the change it protects — never after.
- **Guidance (WS11):** family AGENTS.md files are written *when the family directory first exists*; crate guides move/update with their crates; the ten-family set is complete by end of Wave 5.
- **Verification (WS12):** the full gauntlet runs at each wave boundary; the extension user-journeys re-verify after Waves 2 and 3; the final 100% gate (including the fresh-agent placement test) closes the checklist.

## Suggested first five PRs (concrete, in order)

1. `host_api`: de-wildcard prelude + repoint consumers (WS0.1).
2. Dead-surface tranche #1: `dispatcher` + `embeddings` + `llm::reasoning` + unused dep edges (WS8).
3. Guidance drift hotfix: nonexistent-crate references + phantom modules across root docs, `crates/AGENTS.md`, composition guides, `.claude` skills (WS11.3 subset).
4. Turn vocabulary completion in `host_api::turn` + delete the turns re-export shims + repoint the six vocabulary-only consumers (WS1.1) — exceptions drop 20→13 in one PR.
5. `contracts/ironclaw_loop_contracts` extraction + `agent_loop` flip (WS1.2) — exceptions 13→12, and the loop tier has its contract home.

## Coordination notes

- **#6691:** review it against this plan's Wave 4 inventory; landing it substantially advances Wave 4. Don't duplicate its modules while it's open.
- **#6696:** this plan neither waits for it nor prejudges it; Wave 6 is cleanly severable. If it is closed instead of landed, Wave 6 items need a replacement design decision before `run_state` can be deleted.
- **Review load:** expect ~35–50 PRs total across waves at the sizes above; the July train demonstrated this cadence is sustainable. Anything trending past ~400 effective lines of *semantic* change (moves excluded) should split.
- **Where to record progress:** tick [CHECKLIST.md](CHECKLIST.md) boxes in the PRs that land them; PROPOSAL.md stays frozen as the decision record; disagreements found during implementation go back through a PROPOSAL amendment, not silent divergence.
