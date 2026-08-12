# Automation Tasks — wiring contract (proposed)

Status: **design reference (proposed wiring)**. An earlier UI prototype
implemented the frontend against a mock data seam (`lib/automation-tasks*.ts`,
`hooks/useAutomationTasks.ts`); that code was **rolled back** so the branch stays
code-free, and its target UI now lives in the [mockup](mockup.html). Nothing here
is implemented yet. This document is the reviewable wiring path — the durable
events, projection, transport frame, HTTP surface, and facade methods a follow-up
must add to make the two OOBE concepts real. It follows the Reborn
rules in `.claude/rules/gateway-events.md`, `.claude/rules/lifecycle.md`, and
`.claude/rules/types.md`. *(Crate names below reflect current `main` after the
#6918 family-folder reorg: the event log is `ironclaw_event_log`, the durable
store `ironclaw_event_store`, both under `crates/events/`; the facade
`RebornServicesApi` lives in `crates/product/ironclaw_assistant`; the routes stay
in `crates/product/ironclaw_webui/src/webui_v2/`.)*

The two concepts:

1. **Completed-automations carousel** above the landing composer
   (`components/automation-carousel.tsx`).
2. **Inline calendar reschedule rich-preview** inside a thread
   (`components/calendar-reschedule-card.tsx`).

Both share one action model: *suggested* → Approve / Modify / Cancel; *automated*
→ Modify / Revert.

---

## 1. Domain model

An automation task is a durable, projected record. Typed shape (mirror of the TS
`AutomationTask`, `crates/ironclaw_common` newtypes for identifiers):

```rust
pub struct AutomationTaskId(String);            // newtype, validated (types.md)

pub enum AutomationApp {                         // #[serde(rename_all = "snake_case")]
    Gmail, GoogleCalendar, GoogleDocs, Slack, Notion,
}

pub enum AutomationTaskKind {                    // snake_case
    EmailTriage, CalendarAccept, CalendarReschedule, DocInsights,
}

pub enum AutomationTaskState {                   // snake_case
    Suggested, InProgress, Automated, Reverted, Cancelled,
}
```

Kind-specific payloads (`CalendarReschedule`, `TriagedEmail`) match the TS
interfaces in `lib/automation-tasks.ts` field-for-field. Payloads are
**sensitive by default** (email bodies, attendee lists) and carry the owning
redaction obligation before any log / durable append / projection / transport /
model-visible result (`.claude/rules/safety-and-sandbox.md`).

---

## 2. Durable events (source of truth)

New typed variants on the Reborn runtime event enum (owning crate:
`ironclaw_event_log`; appended via the durable sink in `ironclaw_event_store`
following the owning domain's ordering contract — never a direct handler
broadcast):

| Event | When | Carries |
|---|---|---|
| `AutomationTaskProposed` | agent proposes a task (Suggest/Plan mode, or Auto before it runs) | full task in `suggested` state |
| `AutomationTaskModified` | user edits a suggested or automated task | task id + validated patch |
| `AutomationTaskAutomated` | task runs to completion (approved, or Auto/Bypass ran it) | task id + provider-issued evidence (see §6) |
| `AutomationTaskReverted` | user undoes an automated task | task id + revert evidence |
| `AutomationTaskCancelled` | user dismisses a suggestion | task id |

Each variant is redacted, replayable, and tested for persistence / replay /
projection visibility / redaction / ordering / transport serialization.

---

## 3. Projection

Extend `ironclaw_event_projections` with an `AutomationTaskProjection`:

- scope-filtered by `(tenant, user)` — a caller only ever sees their own tasks;
- carries a projection cursor for replay/resume;
- folds the events above into the current `AutomationTaskState`;
- the carousel reads `state ∈ {automated, reverted}`; the inline card reads a
  single task by id.

Cross-user isolation gets a dedicated regression test (a task proposed for user
A must never appear in user B's projection).

---

## 4. Transport — the inline rich-preview

The inline calendar card is durable UI state, so it rides the existing
projection → `EventStreamManager` → SSE/WebSocket path
(`.claude/rules/gateway-events.md`), **not** a bespoke message. Add a redacted
`WebChatV2EventFrame` variant:

```
capability_display_preview{ kind: "automation_task", task: <redacted AutomationTask> }
```

`MessageList` already renders non-message children (gates, onboarding); the
frontend maps this frame to `<CalendarRescheduleCard>` (and future kinds) the
same way. Reconnect recovers it from replay, never from optimistic frontend
state.

---

## 5. HTTP surface (`ironclaw_webui` — `src/webui_v2/`)

Add routes **and** matching `webui_v2_routes()` descriptor rows (or
`tests/webui_v2_descriptors_contract.rs` fails). These map 1:1 to the TS seam in
`lib/automation-tasks-api.ts`:

| Route ID | Method | Pattern | Effect path |
|---|---|---|---|
| `webui.v2.list_automation_tasks` | GET | `/api/webchat/v2/automations/tasks` | `ProjectionOnly` |
| `webui.v2.approve_automation_task` | POST | `/api/webchat/v2/automations/tasks/{id}/approve` | `TurnCoordinator` |
| `webui.v2.modify_automation_task` | PATCH | `/api/webchat/v2/automations/tasks/{id}` | `ProductWorkflow` |
| `webui.v2.cancel_automation_task` | POST | `/api/webchat/v2/automations/tasks/{id}/cancel` | `ProductWorkflow` |
| `webui.v2.revert_automation_task` | POST | `/api/webchat/v2/automations/tasks/{id}/revert` | `TurnCoordinator` |

Authenticated-caller routes (tenant/user-scoped), not operator-gated. Handlers
consume only `RebornServicesApi`; errors go through `WebUiV2HttpError`.

---

## 6. Facade + effect (`RebornServicesApi` in `ironclaw_assistant`)

New facade methods, each returning the **server-confirmed** task record, never an
optimistic echo (`.claude/rules/gateway-events.md`):

- `list_automation_tasks(caller) -> Vec<AutomationTask>`
- `approve_automation_task(caller, id) -> AutomationTask`
- `modify_automation_task(caller, id, patch) -> AutomationTask`
- `cancel_automation_task(caller, id) -> AutomationTask`
- `revert_automation_task(caller, id) -> AutomationTask`

Approve/Revert are real third-party effects (Gmail send/archive, Calendar
move/restore) and must run through the mediated capability host + product
adapters (the `ProductAdapter` surface in `ironclaw_host_api`, wired via
composition) — **never** a second
outbound HTTP path. Success is admitted only from the provider-issued evidence
(message id / event id / revision) plus a minimal read-back; the
`AutomationTaskAutomated` / `AutomationTaskReverted` event carries that evidence.

**Modify semantics differ by state.** Modifying a *suggested* task edits the
proposal in place (no effect runs until Approve). Modifying an *already-automated*
task **re-runs** it with the change — a real re-execution against the provider —
and returns the fresh automated record with new evidence (the frontend models
this as `rerunModified`: refreshed completion + recomputed derived fields like
the email send-count). `modify_automation_task` therefore branches on the current
state server-side.

---

## 7. Agent mode (composer pill)

The pill (`components/mode-selector.tsx`, store `lib/agent-mode.ts`) currently
persists to scoped localStorage. Durable home:

- `GET /api/webchat/v2/settings/agent-mode` → `{ mode: AgentMode }`
- `POST /api/webchat/v2/settings/agent-mode` `{ mode }` → confirmed `{ mode }`
- surface `agent_mode` on the session (`session.features`/settings) so the pill
  hydrates on load, exactly as `global_auto_approve` already does.

Semantics wire into the existing approval system (`ApprovalCard`, the
`resolve_gate` path, and the `global_auto_approve` feature):

| Mode | Gate behavior |
|---|---|
| `suggest` | every action raises an approval gate (today's default) |
| `plan` | agent emits a batched plan of proposed tasks, one approval resolves the set |
| `auto` | approved task **types** (email triage, invite accepts, doc insights) skip the per-action gate; others still gate. This is the typed generalization of `global_auto_approve` |
| `bypass` | no gates raised at all (full automation) |

`auto`/`bypass` are privilege-escalating and need explicit tests for the gate
suppression path and an audit trail on every auto-run action.

---

## 8. What is stubbed today

- Everything in §§2–7 is **not implemented**. The frontend is fully built and
  runs against mock data + the localStorage mode store.
- The TS types (`lib/automation-tasks.ts`) and the endpoint-shaped seam
  (`lib/automation-tasks-api.ts`) are the frontend half of the contract; wiring
  the backend is a body swap in the seam (mock → `fetch`) with no change to
  components.
- Rust event/projection/route stubs are intentionally **not** added on this
  design branch to keep the workspace warning-clean (the zero-warning clippy gate
  would flag unused scaffold types). Add them together with their first
  implementation + tests, per the tiers in `.claude/rules/testing.md`.
