# OOBE & Onboarding — Integration Proposal (Full Specification)

**Status:** Proposal, under review · Companion to [README.md](README.md) (overview), [PLAN.md](PLAN.md) (execution), [CHECKLIST.md](CHECKLIST.md) (definition of done).

This is the evidence-backed specification behind the overview. It states the problem, defines the two-phase model, inventories **what extends shipped code vs. what is net-new** (§3–§4 — the core framing), enumerates every dependency with an implementation approach (§5), maps the work onto the existing codebase and the #6918 family folders — now landed on `main` (§6), and records the security model, test strategy, risks, and open decisions. §11 explains how the APDD kit and PR #6918 frameworks are applied.

> **Branch is code-free.** The earlier UI prototype (#6994) was rolled back in review; this branch carries the design artifacts (the [mockup](mockup.html) + [integration-review.html](integration-review.html)) and this written plan. Where this spec says *"the prototype"* or *"mock→`fetch` swap,"* it describes the *demonstrated* target UI and the earlier prototype — not code in this PR. The first implementation builds fresh against the [contract](AUTOMATION-TASKS-CONTRACT.md) and the mockup.

---

## 1. Problem & goal

**Goal:** design and ship the first five minutes for a brand-new IronClaw user landing in WebChat v2 right after their workspace is provisioned.

**Today (on `main`):** the landing view is a hero title, a composer, and three static suggestion chips ([`empty-state.tsx`](../../../crates/product/ironclaw_webui/frontend/src/pages/chat/components/empty-state.tsx)). The automation surfaces that would give a first moment of value — a "done for you" carousel, inline rich-preview cards, a Plan card, an agent-mode pill — all assume automations *already exist*. A fresh account has none, so the cold-start and the first-automation moments are undesigned. That gap is what this fills.

**Non-goals:** this proposal does not redesign the steady-state chat, the automations management page, or the extension catalog. It adds a first-run layer on top of them and reuses them where they already do the job.

## 2. The two-phase model

The mockup ([mockup.html](mockup.html)) carries two design tracks behind a **Version** switch; this proposal maps each to a delivery phase.

### Phase F — Foundational (near-term, ships against current `main`)

Scoped to what is feasible and lightweight on today's v2 system for a **multi-tenant enterprise** deployment:

- **Tools are admin-whitelisted, user-authorized.** No separate connect *panel*; each suggested card carries a **Connect &lt;Tool&gt;** CTA, and authorizing runs the existing extension-authorization flow. Once connected, the card becomes an actionable suggestion.
- **Suggested cards are present at the first step** — the agent surfaces its first proposals immediately; each starts in a *connect* state and becomes approve / modify / dismiss once its tool is authorized; cards read as proposals ("Triage your inbox") and flip to results ("Triaged your inbox").
- **No username** unless derivable from a deterministic source; default is a nameless greeting and a plain account chip.
- **Three agent modes**, default **Suggest**: Suggest (always ask), Plan (describe steps, then wait), Auto Approve (auto-approve task types the user already approved, plus explicit requests).
- **Main's composer + the plain pills-collapse drawer** — cards condense to a pill row above the composer when the user types; a section dismiss (✕) with an in-composer **Show suggestions** restore. No bordered/docked frame.

### Phase V — Vision (north-star, follows Foundational)

Everything in Foundational **plus**:

- **Cold-start connect flow** — a connect-your-tools panel where the user selects tools, then a *queued* OAuth walk (sign in once, approve scopes per tool) until all are authorized; then the anticipatory beat → first card reveal. Named greeting; the four-mode set (adds **Bypass**).
- **First-automation reveal** — a brief anticipatory beat (branded `NearProcessIndicator` + skeleton tiles) then the first card is conjured in with an *ai-spark* border sweep, honored under `prefers-reduced-motion`.
- **Attached task drawer** — the cards sit in a bordered drawer frame that **docks onto the composer** and extends up from it, cards inset; header carries the subtitle + collapse/expand + dismiss.

**The load-bearing property:** every Vision piece is a *superset* of a Foundational piece — per-card connect → batched connect; static first card → animated reveal; plain drawer → docked frame; 3 modes → 4 modes. Foundational is never thrown away; Vision extends it.

## 3. Foundational scope — builds-on-preexisting vs. net-new

This is the section the review turns on. Each Foundational capability is classified: **Reuse shipped** (call existing code unchanged), **New UI over shipped** (new presentation, existing mechanism), or **Net-new** (new surface + backing).

### 3.1 Builds on preexisting functionality / design

| # | Capability | Preexisting basis on `main` (verified) | What Foundational adds | Class |
|---|---|---|---|---|
| P1 | Landing surface for the cards | [`empty-state.tsx`](../../../crates/product/ironclaw_webui/frontend/src/pages/chat/components/empty-state.tsx) — hero, composer, suggestion chips | Mount a suggestion drawer above the composer; the hero/composer are untouched | Extend shipped |
| P2 | Card busy / "Automating…" state | `near-process-indicator.tsx` (branded streaming indicator, #6901) | Render it verbatim inside a running card | Reuse shipped |
| P3 | Per-card **Connect** CTA + OAuth | extension-authorization path: `lib/extension-pairing-api.ts`, `lib/product-auth-oauth-events.ts`, `lib/channel-connection-events.ts`, `components/telegram-setup-panel.tsx`, `components/pairing-web-code-panel.tsx` | Card's connect action routes into that flow via the shared `extension_name` resolver — **no new auth path** (CLAUDE.md extension/auth invariant) | Reuse shipped |
| P4 | Agent modes (Suggest / Plan / Auto) | approval-gate system: `resolve_gate`, the `global_auto_approve` feature, `ApprovalCard` | The mode pill is a **new presentation** over these; `auto` is the typed generalization of `global_auto_approve` (§7) | New UI over shipped |
| P5 | "Done for you" → manage it | `pages/automations/` — list, detail panel, delivery defaults, recent runs, summary strip (all on `main`) | A completed suggestion becomes a managed automation; the card deep-links into that page | Reuse shipped |
| P6 | Decision model (Approve / Modify / Cancel · Modify / Revert) | `ApprovalCard` + gate resolution | `TaskActionBar` generalizes the same accept/modify/reject decision for task cards | New UI over shipped |
| P7 | Card brand logos + theming | v2 design tokens, `design-system/icons.tsx` | Real brand product logos, theme-aware; no new token system | Extend shipped |

**Reviewer takeaway:** the entire *connect*, *autonomy*, *busy-state*, and *manage-result* surface of Foundational is a thin layer over functionality that already ships. Foundational does **not** add a new auth path, a new approval system, a new streaming indicator, or a new automations manager.

### 3.2 Net-new in Foundational

| # | Capability | Why net-new | Where it lands |
|---|---|---|---|
| N1 | Suggested-task card + carousel / drawer components | New component family | frontend (prototyped earlier in #6994, rolled back; shape in the mockup) |
| N2 | Pills-collapse drawer interaction + section dismiss + in-composer restore | New interaction model | frontend (in the mockup; prototype rolled back) |
| N3 | `AutomationTask` domain model + 5 durable events + projection | New records/events; the source of truth for cards | `ironclaw_event_log`, `ironclaw_event_projections` (contract §§1–3) |
| N4 | 5 HTTP routes + 5 facade methods | New capability commands | `ironclaw_webui/src/webui_v2/`, `ironclaw_assistant` (contract §§5–6) |
| N5 | **First-run suggestion producer** | Nothing emits the first proposals for a fresh user — the contract covers *completed* and *inline* tasks, not the *first-run suggestion feed* | new; proposed home `ironclaw_triggers` (§5.2) |
| N6 | Agent-mode persistence + typed gate wiring | Mode is localStorage-only in the prototype; needs a durable home + real gate behavior | `ironclaw_webui` settings + the gate path (contract §7) |

## 4. Vision scope — additive net-new (deferred)

None of these block Foundational; each is a superset of a Foundational piece.

| # | Vision capability | Superset of | Net-new work |
|---|---|---|---|
| V1 | Cold-start connect panel (queued multi-tool OAuth) | P3 per-card connect | Batched OAuth *orchestration* over the same pairing infra (§5.7) |
| V2 | First-automation reveal (ai-spark conjure) | static first card | Reveal animation infra driven by `AutomationTaskAutomated`; `prefers-reduced-motion` honored (§5.8) |
| V3 | Anticipatory / "no automations yet" / "working on the first one" states | empty landing | New projection states (#6993 explicitly lists these) (§5.9) |
| V4 | Composer-docked drawer frame | plain pills-collapse drawer | New docked-container component + composer layout (§5.10) |
| V5 | Named greeting | nameless greeting | Username derivation from a deterministic source — **open decision** (§10) |
| V6 | Bypass mode (4th) | 3-mode set | One more gate-behavior branch (no gates at all) — privilege-escalating (§7) |

## 5. Dependency inventory & implementation approach

Each dependency states *what it needs* and *how to build it*; the PLAN sequences them. Foundational deps are **D-F\***; Vision-only deps are **D-V\***. The typed API seam the frontend needs is specified in the [contract](AUTOMATION-TASKS-CONTRACT.md) (§§5–6, §8) and demonstrated by the mockup (the earlier prototype implemented it as a mock client); the first implementation builds against that shape — it is not carried in this branch.

### 5.1 D-F1 — Automation-task backend (events · projection · routes · facade)

*Needs:* the durable source of truth the cards read and act on. *Approach* (from [AUTOMATION-TASKS-CONTRACT.md](AUTOMATION-TASKS-CONTRACT.md) §§2–6, reconciled to current crate names):

- **Events** in `ironclaw_event_log` (`runtime_event.rs`): `AutomationTaskProposed`, `AutomationTaskModified`, `AutomationTaskAutomated`, `AutomationTaskReverted`, `AutomationTaskCancelled` — redacted, replayable, appended via the durable sink (never a direct handler broadcast). Payloads are **sensitive by default** (email bodies, attendee lists) and carry the owning redaction obligation.
- **Projection** in `ironclaw_event_projections`: an `AutomationTaskProjection` scope-filtered by `(tenant, user)`, folding the events into `AutomationTaskState`, with a replay cursor. **Cross-user isolation gets a dedicated regression test** (a task for user A must never appear in user B's projection).
- **Routes** in `ironclaw_webui/src/webui_v2/` (+ matching `webui_v2_routes()` descriptor rows, or the descriptor contract test fails): `list` (GET, ProjectionOnly), `approve` (POST, TurnCoordinator), `modify` (PATCH, ProductWorkflow), `cancel` (POST, ProductWorkflow), `revert` (POST, TurnCoordinator). Authenticated caller-scoped, not operator-gated.
- **Facade** — the 5 methods land on the existing facade **`RebornServicesApi`** (`crates/product/ironclaw_assistant/src/reborn_services.rs`); the typed capability contract `ProductSurface` and its task DTOs live in `ironclaw_product_contracts`. `list/approve/modify/cancel/revert`, each returning the **server-confirmed** record. Approve/Revert are real third-party effects (Gmail send/archive, Calendar move/restore) and **must run through the mediated capability host + product adapters** — never a second outbound HTTP path. Success is admitted only from provider-issued evidence + a minimal read-back. **Modify branches on state:** a *suggested* task edits the proposal in place; an *automated* task **re-runs** against the provider and returns fresh evidence.

*Where it lands (family folders now on `main`):* events/projection → `crates/events/`; facade/DTOs → `crates/product/ironclaw_assistant`; routes → `crates/product/ironclaw_webui`.

### 5.2 D-F2 — First-run suggestion producer  ⚠ the load-bearing new dependency

*Needs:* for a brand-new user, *something* must emit `AutomationTaskProposed` so cards appear at the first step. The existing contract covers completed-carousel and inline-card flows but **not** the first-run suggestion feed — this is the one genuinely new mechanism, and it is the biggest open design question (see §10, decision 2).

*Approach — three candidates, to be decided before Phase F2:*

1. **Deterministic starter set gated on whitelisted/connected extensions** (recommended for Foundational): on first login, the projection seeds a fixed proposal set filtered to the tools the admin whitelisted (e.g. Gmail present → "Triage your inbox"). Simplest, no model call, fully testable; matches the enterprise "admin-whitelisted" posture. Home: a first-login hook that appends `AutomationTaskProposed` via the durable sink.
2. **`ironclaw_triggers`-hosted onboarding suggester** — a trusted-fire trigger scheduled on account creation that runs a bounded suggestion turn and emits proposals. Reuses the triggers trusted-submit path — **which already exists on `main`**: `crates/app/ironclaw_composition/src/automation/{trigger_poller, trigger_poller_trusted_submit, conversation_turn_submitter}` wires exactly this for the shipped scheduled-automations feature. More flexible, heavier.
3. **Agent-driven suggester in the loop** — the agent proposes during the first turn. Most "alive," least deterministic; likely Vision-tier.

*Recommendation:* ship **(1)** in Foundational (deterministic, safe, testable), design **(2)** as the Vision upgrade path. *Where it lands:* `crates/domains/ironclaw_triggers` (+ the composition automation wiring above).

### 5.3 D-F3 — Connect wiring (reuse, not rebuild)

*Needs:* the card's **Connect &lt;Tool&gt;** action must authorize the user's own account for that extension. *Approach:* map the card's `conn: [...]` extension ids to the **existing** install/authorize flow (`extension-pairing-api` + `product-auth-oauth-events`); resolve `extension_name` **once in shared backend logic** and carry it through the wire contract (CLAUDE.md invariant: never route setup UI directly from `credential_name`; chat and settings use the same path). **No new auth path, no frontend-only fallback.** On success, the card transitions unconnected → suggested.

### 5.4 D-F4 — Agent-mode persistence + gate semantics

*Needs:* the mode pill (localStorage-only in the prototype) needs a durable home and real gate behavior. *Approach* (contract §7):

- `GET/POST /api/webchat/v2/settings/agent-mode`; surface `agent_mode` on the session so the pill hydrates on load exactly as `global_auto_approve` already does.
- Wire semantics into the existing approval system: `suggest` = every action gates (today's default); `plan` = batched plan, one approval resolves the set; `auto` = approved task **types** skip the per-action gate (the **typed generalization of `global_auto_approve`**), others still gate; `bypass` (Vision) = no gates. `auto`/`bypass` are privilege-escalating and require explicit gate-suppression tests + an audit trail on every auto-run action.

### 5.5 D-F5 — Carousel de-risk / gating  ⚠ current merge blocker

*Needs:* PR #6994's landing carousel calls `listAutomationTasks()` → `MOCK_COMPLETED_TASKS` for **all** users and is **not** DEV-gated (only `/design-preview` is). Merging as-is shows fabricated "done for you" cards to real users. *Approach:* wire the carousel to the real projection (empty until tasks exist) — which D-F1 provides — **or** gate it behind DEV / a feature flag until D-F1 lands. This dependency is why PR #6994 is a draft; it must close before Foundational ships.

### 5.6 D-F6 — DESIGN.md + design tokens (cross-cutting, APDD design track)

*Needs:* IronClaw has no root `DESIGN.md` and no component workbench; the OOBE cards are a greenfield component family — the natural pilot to seed the APDD design governance track (see the prior APDD kit evaluation, `docs/plans/apdd-governance-kit/`). *Approach:* seed a `DESIGN.md` capturing the v2 token system, theming, a11y floors, and the card component taxonomy; optionally stand up a Storybook workbench for the card/drawer/action-bar components (they are pure Tier-2/3 presentational components — ideal isolation candidates). Deferrable, but cheapest to do while the components are being productionized.

### 5.7 D-V1 — Cold-start queued OAuth orchestration

*Needs:* the Vision connect panel authorizes several tools in one guided pass. *Approach:* a batched orchestration layer over the **same** pairing infra as D-F3 — select set → queue → per-tool sign-in/consent → confirmation. Frontend-heavy; no new backend auth path.

### 5.8 D-V2 — First-automation reveal animation

*Needs:* the emotional peak — the first card conjured in. *Approach:* an *ai-spark* border-sweep reveal triggered by the `AutomationTaskAutomated` projection event (D-F1 already emits it); gated by `prefers-reduced-motion`. Pure frontend once the event exists.

### 5.9 D-V3 — Anticipatory / empty projection states

*Needs:* "no automations yet" and "working on the first one" states. *Approach:* projection-level states surfaced through the existing stream path; #6993 lists these as backend work. Pairs with D-V2.

### 5.10 D-V4 — Composer-docked drawer frame

*Needs:* the bordered drawer that docks onto the composer. *Approach:* a new docked-container component + composer layout change; reuses the Foundational drawer state machine (open / collapsed / dismissed). Frontend-only.

### 5.11 D-V5 — Username derivation

*Needs:* the named greeting. *Approach:* resolve a deterministic source (admin preconfig / email / Slack profile) with a documented precedence and a nameless fallback. **Open decision** (§10).

## 6. Architecture impact — integration into the existing codebase

A WebChat v2 feature crosses five layers (CLAUDE.md): `ironclaw_webui` (SPA + routes) → `ProductSurface` (typed capability contract) → `ironclaw_assistant` (orchestration + facade) → `ironclaw_composition` (wiring by profile) → runtime (`ironclaw_event_log` / `ironclaw_event_projections` / `ironclaw_event_streams`, plus `ironclaw_triggers`). The #6918 family-folder reorg has landed on `main`, so these are the current crate homes (`crates/product/`, `crates/app/`, `crates/events/`, `crates/domains/`). The OOBE work touches each:

```text
 LAYER                         SHIPPED (main)            OOBE ADDS
 ─────────────────────────────────────────────────────────────────────────────────
 ironclaw_webui/frontend       empty-state.tsx           suggestion drawer + cards        [N1,N2]
   (SPA)                       NearProcessIndicator       (reused in card busy state)      [P2]
                               extension-pairing-api      Connect CTA → existing flow      [P3]
                               pages/automations/         "manage" deep-link               [P5]
                               mode-selector (proto)      agent-mode pill                  [P4]
 ironclaw_webui/src/webui_v2   v2 route descriptors      5 task routes + agent-mode       [N4,N6]
 ProductSurface (contracts)    ProductSurface* types     5 task DTOs / commands           [N4]
 ironclaw_assistant            facade + effects          5 facade methods (mediated)      [N4]
 ironclaw_composition          profile wiring            wire projection + producer       [N3,N5]
 ironclaw_event_log            runtime_event.rs          5 AutomationTask* events         [N3]
 ironclaw_event_projections    projections               AutomationTaskProjection         [N3]
 ironclaw_triggers             trusted-fire              first-run suggestion producer     [N5]
 approval-gate system          resolve_gate/global_...   typed auto-approve (per-kind)    [P4,N6]
 ─────────────────────────────────────────────────────────────────────────────────
 legend: [P*]=reuse/extend shipped (§3.1)   [N*]=net-new (§3.2)
```

Effects run through the **mediated capability host + product adapters** — no bypass of authorization, approvals, resource accounting, or host mediation (CLAUDE.md capability-dispatch boundary). LLM output (proposals, evidence, results) is **never deleted** — events are redacted and retained, caches evict, the projection is the read model (CLAUDE.md LLM-data invariant).

## 7. Security & isolation

- **Tenant/user scoping.** The projection is filtered by `(tenant, user)`; a dedicated cross-user isolation regression test is mandatory (contract §3). Routes are authenticated-caller-scoped, not operator-gated.
- **Mediated effects only.** Approve/Modify(rerun)/Revert are real third-party effects and run through the capability host + product adapters; success is admitted only from provider-issued evidence + read-back, never an optimistic echo.
- **Redaction.** Every event payload is sensitive by default and carries the redaction obligation before any log / durable append / projection / transport / model-visible result.
- **Autonomy escalation.** `auto` (Foundational) and `bypass` (Vision) suppress approval gates for whole task types; both require explicit gate-suppression tests and an audit trail on every auto-run action. `auto` generalizes `global_auto_approve` from a single boolean to typed per-kind consent.
- **No new auth path.** Connect reuses the shared extension-authorization resolver; credential vs. extension identity are kept distinct (CLAUDE.md).

## 8. Test strategy

Following `.claude/rules/testing.md` (integration-first; test through the caller; regression-with-every-fix) and the APDD design validation gate.

- **8.1 Backend (integration-first).** Each new event: persistence / replay / projection-visibility / redaction / ordering / transport-serialization tests. The projection: the cross-user isolation test (§7). Each route/facade method: a Reborn integration test through the harness asserting at a seam — not `wait_for_status(Completed)` alone. The `auto`/`bypass` gate-suppression paths: explicit privilege-escalation tests + audit-trail assertions. Modify-rerun: a test that an automated-task modify re-executes and returns fresh evidence.
- **8.2 Frontend.** The prototype already ships `automation-tasks.test.ts` + VM-sandbox stubs (1032 tests green). Add: seam contract tests (the `fetch` shapes match the route DTOs), drawer state-machine tests, connect-flow wiring test (the card's connect calls the shared resolver with the right `extension_name`).
- **8.3 Design validation gate (APDD).** If D-F6 is adopted: `DESIGN.md` conformance (tokens, theming, a11y floors — contrast, focus rings, state-not-by-color-alone), 1:1 parity against the mockup, and — if Storybook is stood up — smoke play test + token/CSS check + one story per card state.
- **8.4 CUJ.** Add a first-run onboarding Critical User Journey (fresh user → cards appear → connect a tool → approve → card flips to done → appears in `/automations`) to the regression baseline once Foundational lands.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Mock cards shown to real users (D-F5) | Draft gate on PR #6994; carousel reads real projection or DEV/flag before merge |
| Suggestion producer (D-F2) scope-creeps into an agent feature | Ship the deterministic starter set first; keep the suggester an explicit Vision upgrade |
| `auto`/`bypass` over-suppress gates | Typed per-kind consent + gate-suppression tests + audit trail; `bypass` deferred to Vision |
| Contract drifts from current crate names | Names reconciled to current `main` (§5.1, F0) after the #6918 reorg; the contract's `src/webui_v2/` path is still current (verified) |
| Backend churns during the #6918 reorg | The reorg has largely landed on `main`; new pieces go straight into the current family folders (§6); move-only PRs stay behavior-free |
| Real third-party effects fail silently | Success admitted only from provider evidence + read-back (§7) |

## 10. Feedback & decisions  *(APDD anchor — leave open until review folds in)*

1. **[OPEN]** Phase boundary — is any Vision piece (docked drawer V4, reveal V2) worth pulling into Foundational?
2. **[OPEN]** D-F2 suggestion producer — deterministic starter set (recommended) vs. triggers-hosted suggester vs. agent-driven? Bounds the first-run feed.
3. **[OPEN]** D-F4 `auto` semantics — exact definition/bounds of "approved task types" (per-tool, per-action, spend/impact caps)?
4. **[OPEN]** D-F5 — real projection now, or DEV/flag gate first, to unblock PR #6994?
5. **[OPEN]** D-F6 — adopt the APDD design track (DESIGN.md + tokens now, Storybook later) with the OOBE cards as pilot?
6. **[OPEN]** D-V5 — username-derivation precedence when several sources resolve; fallback when none.
7. **[OPEN]** Enterprise tool config — does the admin-whitelisted set surface to the user as read-only "connected by your workspace," or invisibly?

## 11. How the two reference frameworks are applied

- **PR #6918 (target-architecture) framing** — this package mirrors #6918's document set: an executive **README** (overview + reviewer decisions + doc index), an evidence-backed **PROPOSAL** (this file), a sequenced **PLAN** (waves/gates/PR-sizing), and a **CHECKLIST** (definition of done). It borrows #6918's execution discipline: move-only/behavior-free PRs kept separate from semantic changes, guidance travels with the change, deletions use the un-masking discipline, and `main` stays shippable after every PR (PLAN).
- **APDD kit (product/design governance)** — this package follows the kit's **docs-first feature workflow**: spec → team review (this §10 stays open until folded in) → plan → test plan, with the binding anchors present (*Feedback & Decisions* §10, *Regression Tests* §8, and a *Critical Bug Fix Log* below per Rule 2). The **design track** (D-F6) proposes seeding `DESIGN.md` + tokens + a Storybook workbench for the new component family, exactly the kit's core design governance. The kit's evaluation for IronClaw is recorded at `docs/plans/apdd-governance-kit/`.

<a name="review-artifact"></a>**Human-review artifact.** A self-contained visual review aid — the 5-layer integration schematic, the dependency graph, the phase timeline, and the shipped-vs-net-new map — lives in this package as [integration-review.html](integration-review.html) ([rendered preview](https://html-preview.github.io/?url=https://github.com/nearai/ironclaw/blob/feat/oobe-chat-automations/docs/design/oobe/integration-review.html)). It renders §3, §5, and §6 for reviewers who prefer the visual.

## 12. Critical Bug Fix Log  *(APDD Rule 2 — the single canonical log)*

*No critical bug fixes yet — this is a proposal. Once Foundational code lands, every critical fix (data loss, isolation breach, crash, auth failure, user-visible breakage) records a row here in the same diff, revises the affected PLAN sections, and adds a failing→passing regression test.*

| Date | Fix | Event/Projection/Route affected | Regression test |
|---|---|---|---|
| — | — | — | — |
