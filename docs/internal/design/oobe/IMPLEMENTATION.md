# OOBE Foundational v1 — Implementation Plan

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
2b. **Persistent drawer + live card status** — keep the surface across the landing→thread transition (a composer-docked drawer) and mirror the run's status on the card via `useChatEvents` (running → completed on `final_reply`, error on `failed`). *Split out of the original slice 2 because it needs the surface to live in the thread view, not just the landing.* (Change 6.)
3. **Connect CTA** — unconnected card → existing extension `setup`/OAuth; on success → suggested. (Change 5.)
4. **"+ Automation"** — completed card → scheduling prompt via `sendMessage` → agent `trigger_create`; deep-link to `pages/automations/`. (Change 4.)
5. **Single-active + error polish** — disable other cards while one runs; `failed`-frame error/incomplete state; resolve the queue-vs-block UX (mockup blocks). (Change 3 + 6.)
6. **Suggestion producer (D-F2, backend)** — emit the first-run suggested set (deterministic starter set gated on connected/whitelisted tools; home `ironclaw_triggers` or a first-login hook). Only when this lands does the flag flip on in prod. (The one net-new backend piece.)

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
