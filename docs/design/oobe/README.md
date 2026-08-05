# IronClaw OOBE & Onboarding — Integration Proposal (Executive Overview)

**Status:** Proposal, under review · **Authored against:** `origin/main` @ post-#6980 (WS1.4 contracts landed) · **Track:** WebChat v2 first-run / onboarding
**Documents:** this overview · [PROPOSAL.md](PROPOSAL.md) (full spec: phasing, shipped-vs-net-new scope, dependency inventory) · [PLAN.md](PLAN.md) (how to execute — phases, gates, PR sizing) · [CHECKLIST.md](CHECKLIST.md) (definition of done) · [oobe.md](../oobe.md) (design brief) · [mockup.html](mockup.html) (interactive mockup — self-contained, open in any browser)

This is the plan for turning the OOBE prototype (PR #6994, presentational + mock data) into shipped product, **in two phases that match the mockup's two versions** — a near-term **Foundational** track that ships against current `main`, and a north-star **Vision** track that follows. It exists so the onboarding work has a fixed destination and a sequenced path, and so a reviewer can tell at a glance which pieces *extend code that already ships* and which are *net-new*.

---

## What this proposes

The WebChat v2 landing view today is a hero + composer + three static suggestion chips ([`empty-state.tsx`](../../../crates/ironclaw_webui/frontend/src/pages/chat/components/empty-state.tsx)). A brand-new user has no idea what IronClaw can take off their plate, and no first moment of value. The OOBE work fills that gap with **suggested task cards** the agent surfaces on first run — each connects a tool, then becomes an approve / modify / dismiss proposal, then flips to a "done for you" result.

The proposal splits that into two tracks so the near-term build is decoupled from the north-star:

- **Foundational** — feasible on top of today's v2 system for a multi-tenant enterprise deployment. Suggested cards at the first step, each with a **Connect** CTA that reuses the extension-authorization path already on `main`; three agent modes that are a new presentation over the **approval-gate system already on `main`**; the plain pills-collapse drawer. **Most of Foundational is a thin layer over shipped functionality plus one net-new backend feed.**
- **Vision** — everything in Foundational plus the aspirational first-run experience: a cold-start connect panel with queued OAuth, the composer-docked drawer frame, the first-automation reveal animation, a named greeting, and the fourth (Bypass) mode.

Both tracks converge on the same card language and the same populated carousel once automations exist.

## Why phase it this way

1. **Foundational rides existing rails.** The connect CTA, the busy/streaming states, the agent-mode semantics, and the "manage what I automated" destination are all **already shipped on `main`** — Foundational reuses them rather than inventing them (see the scope table below and PROPOSAL §3). The genuinely new surface area is small and testable.
2. **The prototype is already the frontend half of the contract.** PR #6994 ships the components and a typed, endpoint-shaped data seam; making it real is a mock→`fetch` body swap plus the backend the seam expects — no component rewrites (PROPOSAL §5, [AUTOMATION-TASKS-CONTRACT.md](../../../crates/ironclaw_webui/frontend/src/pages/chat/AUTOMATION-TASKS-CONTRACT.md)).
3. **Vision is additive, not a redo.** Every Vision piece is a superset of a Foundational piece (per-card connect → batched connect; static reveal → animated reveal; plain drawer → docked frame), so nothing built in Foundational is thrown away.
4. **It lands cleanly in the target architecture.** Each new backend piece has a home in the #6918 target tree (events→`events/`, projection→`events/`, facade/DTOs→`product/`, routes→`product/webui`, the suggestion producer→`domains/triggers`) — this work does not fight the refactor train (PROPOSAL §6).

## The two-track map

```text
                         OOBE / Onboarding
                                │
        ┌───────────────────────┴────────────────────────┐
        ▼                                                 ▼
  FOUNDATIONAL  (Phase F — near-term, v2-faithful)   VISION (Phase V — north-star)
  ─────────────────────────────────────────────    ─────────────────────────────────
  Suggested cards at first step                     + Cold-start connect panel
    · Connect CTA   → REUSE ext-auth (main)            (queued multi-tool OAuth)
    · Busy states   → REUSE NearProcessIndicator     + Composer-docked drawer frame
    · Manage result → REUSE /automations page        + First-automation reveal (ai-spark)
  3 agent modes (Suggest/Plan/Auto)                  + Anticipatory / skeleton states
    → REUSE approval-gate system (main)              + Named greeting (username derivation)
  Pills-collapse drawer + dismiss/restore   NEW      + 4th mode (Bypass)
  AutomationTask events + projection        NEW
  5 routes + 5 facade methods               NEW      All Vision pieces are supersets of a
  First-run suggestion producer             NEW ◄──  Foundational piece — nothing is redone.
  Agent-mode persistence + gate wiring      NEW

  legend:  REUSE = extends code shipped on main   ·   NEW = net-new feature/surface
```

## Shipped vs. net-new — the scope at a glance

The single most important framing for review: **how much of Foundational is new.** Full detail in PROPOSAL §3–§4; the summary:

| Capability | Foundational basis | Classification |
|---|---|---|
| Landing surface the cards mount on | [`empty-state.tsx`](../../../crates/ironclaw_webui/frontend/src/pages/chat/components/empty-state.tsx) (hero + composer) | **Extend shipped** |
| Card busy / "Automating…" state | `NearProcessIndicator` (#6901, on `main`) | **Reuse shipped** |
| Per-card **Connect** CTA + OAuth | extension-authorization path on `main` (`extension-pairing-api`, `product-auth-oauth-events`, pairing/telegram panels) | **Reuse shipped** |
| Agent modes (Suggest / Plan / Auto) | approval-gate system on `main` (`resolve_gate`, `global_auto_approve`) | **New UI over shipped** |
| "Done for you" → manage it | `pages/automations/` management page (on `main`) | **Reuse shipped** |
| Decision model (Approve/Modify/Cancel · Modify/Revert) | generalizes `ApprovalCard` / gate resolution | **New UI over shipped** |
| Suggested-task card + carousel/drawer | — | **Net-new** (prototype in #6994) |
| Pills-collapse drawer + dismiss + restore | — | **Net-new** |
| `AutomationTask` events + projection + routes + facade | — | **Net-new backend** (#6993) |
| First-run **suggestion producer** | — | **Net-new backend** (the load-bearing new dependency) |
| Agent-mode persistence + typed gate wiring | generalizes `global_auto_approve` | **Net-new backend** |

## Dependencies at a glance

Every dependency has an implementation approach in [PLAN.md](PLAN.md); the full inventory is PROPOSAL §5.

**Foundational (must land before Foundational ships):**
- **D-F1 — Automation-task backend:** 5 durable events + projection + 5 routes + 5 facade methods (contract §§2–6).
- **D-F2 — First-run suggestion producer:** something must emit `AutomationTaskProposed` for a fresh user. *Not covered by the existing contract — the largest net-new piece.*
- **D-F3 — Connect wiring:** route each card's connect through the shared `extension_name` resolver + existing OAuth path (no new auth path — CLAUDE.md invariant).
- **D-F4 — Agent-mode persistence + gate semantics:** contract §7; `auto` is the typed generalization of `global_auto_approve`.
- **D-F5 — Carousel de-risk / gating:** the current draft blocker — the landing carousel shows mock cards to *all* users; must read the real projection (empty until tasks exist).
- **D-F6 — DESIGN.md + tokens (cross-cutting):** IronClaw has no `DESIGN.md` yet; the OOBE cards are the natural pilot to seed the APDD design track.

**Vision (additional):**
- **D-V1** cold-start queued OAuth orchestration · **D-V2** reveal-animation infra (`prefers-reduced-motion`) · **D-V3** anticipatory / "no automations yet" projection states (#6993) · **D-V4** composer-docked drawer component · **D-V5** username-derivation source (open decision).

## The decisions reviewers should weigh

1. **Phase boundary** — is the Foundational cut correct, or should any Vision piece (the docked drawer, the reveal) graduate into Foundational? (PROPOSAL §2, §10)
2. **The suggestion producer (D-F2)** — first-login trigger vs. deterministic starter set gated on connected extensions vs. an agent-driven suggester. This is the one genuinely new mechanism and the biggest open design question. (PROPOSAL §5.2)
3. **Agent-mode gate semantics (D-F4)** — `auto` skips the per-action gate for approved task *types*; confirm the typed generalization of `global_auto_approve` and its audit-trail requirements. (PROPOSAL §7)
4. **Carousel gating (D-F5)** — the merge blocker on PR #6994: real projection now, or DEV/flag gate first? (PROPOSAL §5.5)
5. **DESIGN.md pilot (D-F6)** — adopt the APDD design track here (seed `DESIGN.md`, tokens, and — later — a Storybook workbench for the card components), or defer. (PROPOSAL §8.3)

## How to review

- Skim this file, then read [PROPOSAL.md](PROPOSAL.md) §3 (shipped-vs-net-new scope — the core framing) and §5 (dependencies + implementation approach).
- Open [mockup.html](mockup.html), switch **Version** (Vision / Foundational) and **Scene** (First run / Thread / Plan) — every claim here is demonstrated there.
- Argue sequencing in [PLAN.md](PLAN.md): the phases, gates, and suggested first PRs; only the ordering constraints marked ⚠ are load-bearing.
- Challenge [CHECKLIST.md](CHECKLIST.md): it is the definition of done — anything missing goes there.
- The **[human-review artifact](https://claude.ai/code/artifact/734b1b6a-e35d-4736-9ac2-952dcdf84ab4)** renders the schematics — the 5-layer code integration map, the dependency graph, and the phase timeline — for reviewers who prefer the visual.

---

*This package follows the documentation framing of merged PR #6918 (`docs/reborn/target-architecture/`) — executive README → evidence-backed PROPOSAL → sequenced PLAN → definition-of-done CHECKLIST — and the APDD product/design governance kit's docs-first feature workflow (spec → review → plan, with a Feedback & Decisions anchor and a Critical Bug Fix Log). See PROPOSAL §11 for how both frameworks are applied.*
