# OOBE Foundational v1 — Implementation Plan

> **⚠ Historical — Foundational is cut. Everything below is a superseded record.** This
> plan describes the Foundational v1 build; the program has retargeted to **Vision** against
> the durable backend suggestions contract from
> [PR #7694](https://github.com/nearai/ironclaw/pull/7694). The live slice plan and the
> keep/change/delete map for this branch's code are in
> [VISION-RECONCILIATION.md](VISION-RECONCILIATION.md) §5, which **governs**. Reversed or
> retired since this was written: **slice 3's connect model** (cards carry no tool identity;
> connect is a separate surface), **slice 5's single-active lock** (each suggestion starts
> its own thread — cards now **run in parallel**, no lock), and **slice 4's "+ Automation"**
> (no backing in the shipped contract — **removed**). The retired slice 2b (live card
> status) becomes buildable. Do not treat any "change 1–6" below as current.

**Grounded in current `main`.** Every seam named here was verified to ship on `main` (backend + frontend). The premise this plan proves: **v1 is almost entirely a frontend build over already-enabled capabilities** — the only net-new backend piece is the first-run *suggestion producer*, and even that is optional for the first slices. Companion to [PROPOSAL.md](PROPOSAL.md) §2A (the wiring spec) and [CHECKLIST.md](CHECKLIST.md).

Scope: the six Phase-1 UX changes (PROPOSAL §2A). Foundational only — Vision is unchanged.

---

## 0. What `main` already enables (verified)

| v1 need | Shipped seam on `main` (verified) | Status |
|---|---|---|
| Approve → run a job in the thread | `POST /api/webchat/v2/threads/{id}/messages` → `send_message` → `RebornServices::submit_turn` → `TurnCoordinator::submit_turn` (returns `run_id`). Frontend client: `lib/api.ts` **`sendMessage`** (`:537`, POSTs `/messages`). | **Enabled** |
| Live activity + card status | `WebChatV2Event` stream (SSE `GET …/threads/{id}/events`); frontend **`pages/chat/lib/useChatEvents.ts`** already folds frames → `queued`/`running`→processing, `capability_activity`→running, `final_reply`→done, `failed`/`cancelled`→error. | **Enabled** |
| One active job at a time | `submit_turn` returns `DeferredBusy`/`RejectedBusy` when a run is active on the thread (`RebornSubmitTurnResponse`). | **Enabled** (drives change 3) |
| Connect a tool | `POST …/extensions/{id}/setup` → `builtin.extension_setup_submit`; auth surfaces as the `AuthRequired` frame. Frontend: `lib/extension-pairing-api.ts`, `lib/product-auth-oauth-events.ts`. | **Enabled** |
| Gate/approval inside a run | `POST …/runs/{run_id}/gates/{gate_ref}/resolve` → `resolve_gate`. | **Enabled** |
| "+ Automation" → scheduled | Inject a prompt so the agent calls tool `builtin.trigger_create` (no REST create by design). Dashboard: `GET …/automations` + `pages/automations/` (`hooks/useAutomations.ts`, `lib/automations-presenters.ts`). | **Enabled** (create is agent-tool-driven) |
| Gate the surface for safe rollout | Session **feature flags** — `app/auth.ts` reads `features?.…` off the session; the existing onboarding routing gate is `lib/onboarding-gate.ts`. | **Enabled** |
| Emit the first-run suggested cards | *Nothing emits "suggested task cards" yet* — `onboarding-gate.ts` only decides the LLM-provider first-run redirect. | **GAP → net-new (D-F2)** |
| Durable `AutomationTask` event/projection | Not present (`RunStatusProjection`/`RunProjectionStatus` is the closest). | **Not needed for v1** — a card drives an ordinary foreground turn keyed by `run_id`. |

**Conclusion:** the card actions all map onto shipped seams. The single net-new backend dependency is the *suggestion producer* (D-F2), and the first slices can be built and demoed behind a flag without it (a card is just a prompt + a run subscription).

## 1. Frontend architecture

Re-add the rolled-back prototype components (recoverable from git history, pre-`aa24023af`), adapted to §2A:

- **Components** (net-new frontend, `pages/chat/components/`): `SuggestedTaskCard` (states: unconnected / suggested / running / completed / failed), the suggestion **drawer/carousel**, the **Connect** CTA, and the **status chips** (Completed / error-incomplete). Mount in **`empty-state.tsx`** above the composer (the landing already owns `onSend` + `onSuggestion`).
- **Client (all reuse):** Approve → the existing send path (`onSend` → `lib/api.ts sendMessage`); status → **`useChatEvents`** keyed by the returned `run_id`; Connect → `extension-pairing-api` + `product-auth-oauth-events`; "+ Automation" → `sendMessage` (a scheduling prompt) then link to `pages/automations/`.
- **State model:** a card holds `{ state, runId? }`. Approve sets `runId` from the `sendMessage`/`submit_turn` response and derives its visible state from `useChatEvents(runId)` (running → completed on `final_reply`, error on `failed`). No local timers, no mock data.
- **Single-active (change 3):** while any card has an active `runId`, disable the others; `submit_turn`'s `DeferredBusy`/`RejectedBusy` is the backstop if two are attempted.

## 2. Gating — the merge-safety requirement (D-F5, done right)

The earlier prototype was rolled back for showing mock cards to all users. v1 must not repeat that:

- Gate the entire task-card surface behind a **session feature flag** (e.g. `features?.oobe_suggestions`), **off by default**. With the flag off, `empty-state.tsx` renders exactly as today. No mock data path ships enabled.
- Local dev can also read `import.meta.env.DEV`.
- The flag stays **off in production** until the suggestion producer (D-F2) exists to seed *real* cards. So every slice below is safe to merge to `main` with the flag off.

## 3. PR slices (vertical, each independently mergeable behind the flag)

1. ✅ **Flag + surface + `SuggestedTaskCard`** *(landed)* — component (all states) from a static list under the off-by-default flag; per-state tests.
2. ✅ **Approve → foreground turn** *(landed)* — Approve submits the task's `approvePrompt` through the existing `chat.tsx` `handleSend` (display content = card title), running a real foreground turn; the thread streams the activity by reuse; the card flips to `running` optimistically. (Change 2.)
2b. ❌ **RETIRED — was based on a wrong assumption.** Originally scoped as "keep the surface mounted across the landing→thread transition (a composer-docked drawer) + mirror live status via `useChatEvents`." Traced against `chat.tsx`: `EmptyState` and `MessageList` are **mutually exclusive** (`showLanding ? <EmptyState/> : <MessageList/>`) — the moment `handleSend` navigates into the new thread, `EmptyState` (and the surface's local state) **fully unmounts**. A persistent drawer would mean building a second live-status surface duplicating what the thread's own message/event rendering already does, and would import Vision's docked-drawer architecture into Foundational (explicitly Vision-only). **The correct model — already fully delivered by slices 1+2+5** — is: the card gives instant local feedback for the pre-navigation instant; once the user is in the thread, the thread *is* the live-status surface, with no new code needed. What genuinely remains — status shown back **on the card** for a *returning* user — requires a durable per-task record, i.e. it is not a separate frontend slice; it's **slice 6** (below).
3. ✅ **Connect CTA** *(landed — reuse, not reimplement; live OAuth QA pending)* — an unconnected card's **Connect** resolves the card's `app` → a **real** catalog extension (`resolveConnectExtension` over `useExtensions()`), then opens the **existing** extensions setup/OAuth modal (`configure-modal.tsx`, `React.lazy`-loaded so its OAuth watcher/state-machine weight never touches the surface chunk, let alone eager /chat). On a successful save the card flips `unconnected → suggested`. This drives the **one real connect path** — no cloned `useOauthSetup`, `openAuthPopup`, or flow-status polling. Rejected the alternatives after reading `useExtensions.ts`: `useOauthSetup` is a ~250-line page-level state machine keyed on a real `packageRef`+secret descriptor (not a helper to extract), and hand-rolling a parallel popup/polling/error-mapping flow would duplicate it and risk unverifiable bugs. **Pure unit coverage:** `connect-extension.test.ts` (6 tests — resolution/tolerance/preference/fallback/null) + `suggested-task-surface.test.ts` (Connect reports the task, mounts `ConfigureModal` with the resolved extension, flips to suggested on save, shows a notice when no catalog match). **What tests can't cover** and needs a browser: the live OAuth popup round-trip (window open → provider consent → completion callback → poll), inherently gated on a real third-party consent grant. Demo `app` ids (`gmail`/`google_calendar`/`google_docs`) resolve tolerantly against the live catalog; if a real package ref differs, QA reveals it and the id is corrected (or slice 6 supplies authoritative ids). (Change 5.)
4. ✅ **"+ Automation"** *(landed)* — a completed card's **+ Automation** submits the task's `automationPrompt` through the existing `handleSend`, so the agent schedules it via `builtin.trigger_create` (prompt injection, no REST create); the card flips to an optimistic "Automation scheduled" chip. Deep-link into `pages/automations/` is a later polish. (Change 4.)
5. ✅ **Single-active + error polish** *(landed — the achievable half)* — every other card locks (disabled + dimmed) while one runs, resolving the queue-vs-block UX as **block** (matches the mockup). The `failed`-frame error/incomplete status is, per the 2b finding above, inherently the thread's job once navigation happens — nothing further to build here for Foundational v1. (Change 3.)
6. **Suggestion producer (D-F2, backend)** — emit the first-run suggested set (deterministic starter set gated on connected/whitelisted tools; home `ironclaw_triggers` or a first-login hook). This is also where a real, durable per-task status record would live if "status persists on the card across visits" becomes a hard requirement. Only when this lands does the flag flip on in prod. (The one net-new backend piece.)

Slices 1–5 are frontend-only and safe with the flag off; slice 6 is the backend gate to turning it on.

## 4. Tests (per `.claude/rules/testing.md`)

- **Frontend:** per-state `SuggestedTaskCard` render tests; **test through the caller** — Approve invokes `sendMessage` with the right thread + prompt (not just a handler unit); status-derivation from injected `useChatEvents` frames (`running`/`final_reply`/`failed`); gating test (flag off → nothing renders); Connect calls the shared `extension_name` resolver path. Extend the existing `useChat-send.test.ts` / `useChatEvents.test.ts` rather than adding parallel suites.
- **Integration (slice 6):** the suggestion producer through the harness, asserting at a seam; cross-user isolation; per `tests/integration/`.

## 5. Open items to resolve before slice 5 / 6

- **Queue vs block** when a second card is approved mid-run (`submit_turn` defers/rejects) — mockup blocks; confirm.
- **Suggestion producer approach** (D-F2 / epic #7044 conflict 1): deterministic starter set (recommended for v1) vs. learn-from-data. The flag lets slices 1–5 ship before this is decided.
- **Flag name + ownership** — pick the session-feature key and where it's set (operator config vs. build).

## 6. What this plan deliberately does *not* build

- No new `AutomationTask` durable event/projection/routes/facade (the old contract §2–§6) — unnecessary for v1; a card is a foreground turn.
- No new "create automation" REST route — "+ Automation" uses the agent tool via prompt injection, by design.
- No agent-mode backend (§2A change 1 removed the selector from Foundational).
