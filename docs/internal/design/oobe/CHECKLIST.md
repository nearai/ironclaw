# OOBE & Onboarding — Definition of Done

**What this is:** the checklist that says the work is finished — grouped by phase ([PLAN.md](PLAN.md)), each box a verifiable outcome. This is the challengeable artifact: if you think something is missing, it goes here. Boxes are ticked in the PR that lands them. Foundational (F) must fully close before Vision (V) starts.

Legend: `[ ]` not started · `[~]` in progress · `[x]` done. Every code box implies its tests (`.claude/rules/testing.md`, PROPOSAL §8) are green.

---

## F0 — De-risk & unblock

- [x] Prototype rolled back; design reworked (§2A). The `SuggestedTaskCard` surface now mounts unconditionally over the durable suggestion feed.
- [x] Branch merged up to date with `main` (post-#6918 family-folder reorg); [`AUTOMATION-TASKS-CONTRACT.md`](AUTOMATION-TASKS-CONTRACT.md) remains in the canonical `docs/internal/design/oobe/` directory.
- [x] **Carousel data safety (D-F5)** — the surface reads the durable projection and never returns `MOCK_COMPLETED_TASKS` to real users.
- [x] Contract reconciled to the post-#6918 family-folder names (`ironclaw_event_log` + `ironclaw_event_store` under `crates/events/`; facade `RebornServicesApi` in `ironclaw_assistant`; `src/webui_v2/` confirmed current).
- [x] Decision round #1 recorded (PROPOSAL §10 items 2 and 4 resolved; item 5 settled 2026-08-21).
- [x] D-F6 settled — `DESIGN.md`, tokens and the workbench are owned by [`docs/internal/reborn/design-system/`](../../reborn/design-system/README.md); no local seed is landed here (PROPOSAL §5.6).

## F1 — Automation-task backend (D-F1)

- [ ] `AutomationTask` model + `AutomationTaskId` newtype (validated) mirror the TS shapes field-for-field.
- [ ] 5 events on `ironclaw_event_log` (`Proposed`/`Modified`/`Automated`/`Reverted`/`Cancelled`), each redacted, replayable, appended via the durable sink.
- [ ] Per-event tests: persistence · replay · projection-visibility · redaction · ordering · transport-serialization.
- [ ] `AutomationTaskProjection` on `ironclaw_event_projections`, scope-filtered by `(tenant, user)`, with a replay cursor.
- [ ] ⚠ **Cross-user isolation regression test** — a task for user A never appears in user B's projection.
- [ ] 5 routes in `src/webui_v2/` **and** matching `webui_v2_routes()` descriptor rows (descriptor contract test passes).
- [ ] 5 facade methods on `RebornServicesApi` (`ironclaw_assistant`), each returning the server-confirmed record (no optimistic echo).
- [ ] Approve/Revert run through the mediated capability host + product adapters (no second outbound HTTP path); success admitted only from provider evidence + read-back.
- [ ] Modify branches on state (suggested = edit in place; automated = re-run with fresh evidence) — tested both ways.
- [ ] `list` seam flipped mock→`fetch` with no component change.

## F2 — First-run suggestion producer (D-F2)

- [ ] Decision §10.2 recorded (approach chosen).
- [ ] Producer emits `AutomationTaskProposed` for a fresh user, filtered to admin-whitelisted / connected tools.
- [ ] Tests: tool set X → expected proposals; no whitelisted tools → safe fallback set; isolation holds.
- [ ] A brand-new account shows suggested cards at the first step from real events.

## F3 — Connect wiring + agent mode (D-F3 + D-F4)

- [ ] Card **Connect** routes into the existing extension-authorization flow via the shared `extension_name` resolver — no new auth path, no frontend-only fallback (CLAUDE.md invariant).
- [ ] Connect wiring tested **at the caller** (card→authorize handler), not just the resolver helper.
- [ ] Card transitions unconnected → suggested on authorize success.
- [ ] Agent-mode settings endpoint + session hydration (mirrors `global_auto_approve`).
- [ ] `suggest`/`plan`/`auto` wired into `resolve_gate`; `auto` = typed per-kind generalization of `global_auto_approve`.
- [ ] Gate-suppression tests + audit-trail assertions for `auto` (privilege-escalating).
- [ ] Mode persists and hydrates on load.

## F4 — End-to-end + CUJ

- [ ] `TaskActionBar` decision model (Approve/Modify/Cancel · Modify/Revert) wired through the facade; remaining seam methods flipped mock→`fetch`.
- [ ] First-run onboarding **CUJ** added to the regression baseline (fresh user → cards → connect → approve → done → appears in `/automations`).
- [ ] Carousel gate (D-F5) retired — projection is the source of truth.
- [ ] Foundational demoable end-to-end on a fresh account.

## F5 — Design track pilot (D-F6) — optional, parallel

*✎ 2026-08-21: `DESIGN.md`, tokens and the workbench are owned by [`docs/internal/reborn/design-system/`](../../reborn/design-system/README.md) (PR #7257) — OOBE contributes the pilot, not the governance (PROPOSAL §5.6).*

- [ ] OOBE card taxonomy + a11y floors contributed **into** that program's `DESIGN.md` (Phase-2 work, issue #7042 under Epic #7781) — no parallel constitution seeded here.
- [ ] Card / action-bar / drawer / mode-pill stories added to the Phase-1 catalog (#7750) once it lands: smoke play test + token/CSS check + one story per state.
- [ ] Design validation gate passes on the OOBE components (1:1 parity vs. mockup; tokens; light+dark; a11y).

## Foundational exit gate

- [ ] All F0–F4 boxes closed (F5 optional).
- [ ] No mock data reachable by real users anywhere in the landing/onboarding path.
- [ ] Onboarding CUJ green against a real build.
- [ ] PROPOSAL §10 Foundational decisions (2, 3, 4, 5) all closed.

**Success criteria (adopted from epic #7044):**

- [ ] **Time to first automation** — sign-in → a working automation under the target the epic sets ([X] min).
- [ ] **First-session activation** — % of new users who accept ≥1 automation in session one meets target.
- [ ] **Suggestion quality** — first-suggestion accept-vs-dismiss rate meets target (proxy for profile quality).

---

## V1 — Reveal + anticipatory states (D-V2 + D-V3)

- [ ] ai-spark first-automation reveal driven by `AutomationTaskAutomated`; honored under `prefers-reduced-motion`.
- [ ] "no automations yet" / "working on the first one" projection states surfaced through the stream path.

## V2 — Cold-start connect panel (D-V1)

- [ ] Batched multi-tool OAuth orchestration over the existing pairing infra (select → queue → per-tool consent → confirmation).
- [ ] No new backend auth path introduced.

## V3 — Docked drawer frame (D-V4)

- [ ] Composer-docked bordered drawer component; reuses the Foundational drawer state machine.

## V4 — Named greeting + Bypass (D-V5 + V6)

- [ ] Username derivation with documented source precedence + nameless fallback (decision §10.6).
- [ ] `bypass` mode wired (no gates) with gate-suppression tests + audit trail.

## Vision exit gate

- [ ] All V1–V4 boxes closed.
- [ ] Vision and Foundational render the same populated carousel once automations exist (convergence verified).
