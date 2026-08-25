# OOBE & Onboarding — Execution Plan

**What this is:** the recommended sequence for shipping the [PROPOSAL](PROPOSAL.md) — phases, gates, PR-sizing rules, and decision points. Phases group the dependencies (D-F\* / D-V\*) from PROPOSAL §5; the checklist ([CHECKLIST.md](CHECKLIST.md)) is the *what*, this is the *when and how*. Nothing here is sacred except the ordering constraints marked ⚠.

**Operating principles** (borrowed from PR #6918's execution discipline, which the July refactor train validated):

1. **Owner-by-owner, never big-bang.** One coherent owner per PR — one event family, one route, one component seam.
2. **Move-only / mock→real swaps are behavior-free and say so.** The frontend seam swap (mock→`fetch`) ships with no component change and is reviewed as such; semantic backend changes never share a PR with it.
3. **Tests and guidance travel with the change.** Every PR updates the contract doc, the affected rule, and this plan's status in the same diff — guidance drift is how agents get confused.
4. **`main` stays shippable after every PR.** Behind the carousel gate (D-F5) until the projection is real, so no PR exposes mock data to users.
5. **Decision gates are cheap to hold, expensive to skip.** The `[decision]` items (PROPOSAL §10) are called before their phase starts — most are one thread each.

---

## Phase F0 — De-risk & unblock (start now, no design risk)

*Everything here is independently landable and shrinks every later phase.*

1. **D-F5 carousel gate** ⚠ — the guard for when the carousel returns. Gate the landing carousel behind DEV / a feature flag (or an empty-projection read) so it never exposes mock "done for you" cards to real users. The earlier prototype's *ungated* mock path is exactly what got it rolled back — the first implementation PR that re-adds the carousel must ship it gated.
2. **Contract reconciliation — ✅ done in this PR.** [AUTOMATION-TASKS-CONTRACT.md](AUTOMATION-TASKS-CONTRACT.md) now reflects the post-#6918 family-folder names: event log `ironclaw_event_log` + durable store `ironclaw_event_store` under `crates/events/`; facade `RebornServicesApi` in `crates/product/ironclaw_assistant`; routes in `crates/product/ironclaw_webui/src/webui_v2/` (confirmed current). Docs-only.
3. **Decision round #1** `[decision]` — close PROPOSAL §10 items 2 (suggestion producer) and 4 (carousel gating). One thread each. *(Item 5, the DESIGN.md pilot, is settled — see below.)*
4. **D-F6 — nothing to seed here.** `DESIGN.md` and the workbench are owned by [`docs/internal/reborn/design-system/`](../../reborn/design-system/README.md) (PROPOSAL §5.6); OOBE's contribution is its card taxonomy + a11y floors, offered into that program's `DESIGN.md` (the Phase-2 work tracked by issue #7042, under Epic #7781) rather than a local draft.

*Exit criteria: the carousel-gate approach decided; contract reconciled (done); §10.2/§10.4 decided (§10.5 settled — governance owned elsewhere). (Implementation is landing behind an off-by-default flag; the docs remain mergeable.)*

## Phase F1 — Automation-task backend (D-F1) — the leverage phase

*Creates the durable source of truth. Everything downstream is a body swap once this exists.*

- **Order inside the phase:** `AutomationTask` model + `AutomationTaskId` newtype → the 5 events (one PR per event family is overkill; land them as one typed set with their persistence/replay/redaction/ordering/serialization tests) → `AutomationTaskProjection` + the ⚠ **cross-user isolation test** (PROPOSAL §7) → the 5 routes + `webui_v2_routes()` descriptor rows (or the descriptor contract test fails) → the 5 facade methods on `RebornServicesApi` (`ironclaw_assistant`), each returning server-confirmed records through the mediated capability host.
- Approve/Revert wire to the existing product adapters (Gmail/Calendar) — **never a second outbound HTTP path**; success admitted from provider evidence + read-back. Modify branches on state (suggested = edit in place; automated = re-run).
- **Milestone:** `list_automation_tasks` returns real (empty) data; the frontend seam flips mock→`fetch` for `list` with **no component change**; D-F5's gate can become "empty projection" instead of a flag.

## Phase F2 — First-run suggestion producer (D-F2) ⚠ gated on decision §10.2

*The one genuinely new mechanism — hold the decision before building.*

- Implement the decided approach (recommended: **deterministic starter set gated on admin-whitelisted / connected extensions** — PROPOSAL §5.2). A first-login hook appends `AutomationTaskProposed` via the durable sink, filtered to the tools present; no model call in the Foundational version.
- Tests: a fresh user with tool set X gets exactly the expected proposals; a user with no whitelisted tools gets the safe/no-tool proposals; isolation holds.
- **Milestone:** a brand-new user sees suggested cards at the first step, from real events — the Foundational cold-start is real end-to-end.

## Phase F3 — Connect wiring + agent mode (D-F3 + D-F4)

*Two independent seams; can land in parallel.*

- **D-F3 connect** ⚠ **reuse, don't rebuild:** the card's Connect action routes into the existing extension-authorization flow via the shared `extension_name` resolver (CLAUDE.md invariant — resolve once in backend, carry through the wire; no frontend-only fallback, no new auth path). Test at the caller (the handler that maps card→authorize), not just the resolver helper (`.claude/rules/testing.md`).
- **D-F4 agent mode:** the settings endpoint + session hydration (mirrors `global_auto_approve`); wire `suggest`/`plan`/`auto` into `resolve_gate`; `auto` = typed per-kind generalization of `global_auto_approve` with gate-suppression tests + audit trail. `bypass` stays out (Vision).
- **Milestone:** connect a tool from a card → it authorizes via the shipped flow → card transitions to suggested; mode persists and changes real gate behavior.

## Phase F4 — Approve / modify / revert end-to-end + CUJ

- Wire the `TaskActionBar` decision model (Approve/Modify/Cancel · Modify/Revert) through the facade; flip the remaining seam methods mock→`fetch`.
- Add the **first-run onboarding CUJ** (PROPOSAL §8.4) to the regression baseline: fresh user → cards → connect → approve → card flips to done → appears in `/automations`.
- **Milestone:** Foundational is feature-complete and demoable on a fresh account; the carousel gate (D-F5) is retired — the projection is the truth.

## Phase F5 — Design track pilot (D-F6) — optional, parallel

*Runs alongside F1–F4. ✎ **2026-08-21:** `DESIGN.md`, tokens and the Storybook workbench are **not** built here — they are owned by [`docs/internal/reborn/design-system/`](../../reborn/design-system/README.md) (PR #7257; Phase 1 under Epic #7038 as PR #7750 · Phases 2–3 under Epic #7781, issue #7042 tracking the Phase-2 `DESIGN.md` work). What is left in F5 is OOBE's pilot contribution (PROPOSAL §5.6).*

- Contribute the OOBE card taxonomy + a11y floors **into** that program's `DESIGN.md` rather than seeding a parallel one.
- Add stories for the pure Tier-2/3 card components (card, action bar, drawer, mode pill) to the Phase-1 catalog once it lands: smoke play test + token/CSS check + one story per state (PROPOSAL §8.3).
- **Milestone:** the OOBE component family is the design-governance pilot; the validation gate is enforceable on future card work.

---

## Vision phases (after Foundational ships)

Each is additive and a superset of a Foundational piece; none redo Foundational work.

- **V1 — Reveal + anticipatory states (D-V2 + D-V3).** The `AutomationTaskAutomated` event already exists (F1); add the ai-spark reveal (`prefers-reduced-motion`) and the "no automations yet" / "working on the first one" projection states. Frontend-heavy.
- **V2 — Cold-start connect panel (D-V1).** Batched multi-tool OAuth orchestration over the same pairing infra as D-F3 — select set → queue → per-tool consent → confirmation.
- **V3 — Docked drawer frame (D-V4).** The composer-docked bordered drawer; reuses the Foundational drawer state machine.
- **V4 — Named greeting + Bypass mode (D-V5 + V6).** Username derivation (⚠ decision §10.6) and the 4th gate mode (privilege-escalating; gate-suppression tests).

## Suggested first PRs (concrete, in order)

1. **D-F5 carousel gate** — the first implementation PR that re-adds the landing carousel ships it DEV/flag-gated (or reading an empty real projection). Small, behavior-free for users.
2. **Contract reconciliation** — crate-name refresh in the contract doc (docs-only). *(✅ already landed in this PR.)*
3. **D-F1 part 1** — `AutomationTask` model + newtype + the 5 events with their persistence/replay/redaction/ordering/serialization tests.
4. **D-F1 part 2** — `AutomationTaskProjection` + the cross-user isolation test; flip the `list` seam mock→`fetch`.
5. **D-F1 part 3** — the 5 routes + descriptor rows + facade methods (mediated), one coherent slice.

## Gates & continuous tracks

- **Testing (every phase):** integration-first through the harness, asserting at a seam; the cross-user isolation test and the gate-suppression tests are non-negotiable; regression-with-every-fix (Rule 2). See `.claude/rules/testing.md` and PROPOSAL §8.
- **Guidance (every phase):** the contract doc, the affected `.claude/rules/*`, and this PLAN's phase status update in the same diff.
- **`main` shippable (every phase):** the carousel gate holds until the projection is real; no PR exposes mock data.

## Coordination notes

- **PR #6994** (this branch) carries the design/plan **and** the rolling Foundational v1 implementation (behind an off-by-default flag; slice 1 landed). The earlier UI prototype on it was rolled back in review (mock automations shown to real users; an autonomy selector execution ignored — IronLoop/CodeRabbit blockers), so the carousel gate (D-F5) and the mode-pill confinement (D-F4) move to the implementation PRs rather than gating this one.
- **Issue #6993** tracks the backend (D-F1, D-F4, and the D-V3 anticipatory states). This plan supersedes its ordering with the phased sequence above; keep #6993 as the tracking issue and tick CHECKLIST boxes in the PRs that land them.
- **#6918 family reorg — landed on `main`.** The branch is merged up to date with it; every new backend piece goes straight into its current family folder (PROPOSAL §6): events/projection → `crates/events/`, facade/DTOs → `crates/product/ironclaw_assistant`, routes → `crates/product/ironclaw_webui`, suggestion producer → `crates/domains/ironclaw_triggers`. Keep semantic changes out of any remaining move-only PRs.
- **WS1.x contracts** (`ironclaw_product_contracts` / `ironclaw_extension_contracts` / `ironclaw_loop_contracts`, landed) — the new task DTOs/ports belong in `ironclaw_product_contracts`, not inlined into `ironclaw_product`.
- **Review load:** expect ~8–12 PRs for Foundational at the sizes above; anything past ~400 effective lines of *semantic* change (mock→`fetch` swaps excluded) should split.
