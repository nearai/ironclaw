# Spec: missions — durable MissionManager state machine and LFD profile contract

Sources: `docs/plans/2026-03-24-missions.md`, `docs/lfd/roadmap-blue-lanes-2026-07-07/11-missions/goal.md`, `docs/lfd/roadmap-blue-lanes-2026-07-07/LANE-ADDENDA.md` lane 11, `lfd/_briefs/missions.md`, `docs/reborn/contracts/triggers.md`, `docs/reborn/engine-v2-to-reborn-parity.md`, `crates/ironclaw_triggers/AGENTS.md`, and `crates/ironclaw_turns/CLAUDE.md`.

## 1. Product contract

A Mission is a durable outcome process, not a prompt alias and not a routine. A mission record must carry:

- stable mission identity scoped by tenant, agent, project, creator, normalized goal, and trigger/manual source;
- goal text and definition of done;
- budget/resources: spend ceiling, model-call ceiling, wall-clock or recurrence window, and whether explicit approval is required for overrun;
- allowed action classes and denied action classes;
- lifecycle status: `draft`, `active`, `blocked`, `completed`, `failed`, `denied`, or `budget_exhausted`;
- trigger reference and canonical fire slots when scheduled;
- durable checkpoints before model calls, side effects, blocking waits, cancellation, and terminal completion;
- progress summary, blockers, next step, current focus, approach history, and user-visible status;
- terminal report fields: goal, definition of done, outcome, spend, completed work, blockers, remaining risks, memory summary path, and stop reason.

Mission persistence must be backend-backed through Reborn storage, not transcript-only state. A runtime restart or process reload must be able to list missions, load their checkpoints, resume the same identity, and continue from the latest durable checkpoint.

## 2. State-machine distinctions

The implementation must distinguish four user intents:

- **One-off task**: finite, known completion, no recurring scheduler entry. It may be scheduled as a task under a mission when the mission needs one bounded unit of work.
- **Routine**: recurring fixed cadence/action with narrow state. It is suitable for monitoring, daily digests, and repeated checks.
- **Mission**: goal-oriented state machine that may schedule routines and one-off tasks, update focus, retain approach history, and stop when definition of done or budget/terminal policy says stop.
- **Standing mission**: a mission without a final definition of done horizon, but still bounded by budget windows and policy. It keeps checkpoints and can run indefinitely only while budget and authorization remain valid.

Classification is scored. Finite docs migration and redirect cleanup must not become routines. Indefinite monitoring and competitor watch must schedule routines. Mixed growth/outreach missions may schedule both.

## 3. Trigger and run-profile integration

`ironclaw_triggers` owns trigger records, schedule validation, deterministic fire identity, repository operations, and trigger poller worker semantics. A mission trigger fire must route through the normal Reborn turn pipeline; it must not create a second agent loop.

`ironclaw_turns` already defines a privileged `long_running_mission` run profile with durable checkpoint policy. Mission submissions must request that profile through a product/host-authorized path. User-originated attempts to request privileged long-running mission profiles directly must remain unauthorized.

Expected seams:

1. Mission create/update API validates goal, definition of done, budget, allowed action classes, and trigger source before persistence.
2. Trigger fire materializes a mission turn with mission identity, trigger slot, and current mission state.
3. The mission loop writes a checkpoint before each model call, before each side effect, before blocking or waiting, and at terminal status.
4. The mission manager can schedule routine records and one-off task records, but those records point back to the same mission id.
5. Completed missions quiesce their triggers. Duplicate fires for the same slot are idempotent and do not create a second mission identity.

## 4. Meta-prompt from memory

Before each mission turn, the prompt envelope is built from mission state and project memory:

- goal and definition of done;
- budget remaining and stop policy;
- current focus and next step;
- approach history with failures and outcomes;
- retrieved project memory docs, including paths and digest terms;
- trigger payload digest when the mission was event-fired;
- explicit allowed and denied action classes.

If memory docs are absent, the prompt builder emits a minimal prompt with goal, definition of done, budget, and next-step discovery. It must not crash or silently invent memory. Static meta-prompts fail probe because contracts vary memory paths, doc contents, failure categories, and dates.

## 5. Progress, adaptation, and terminal behavior

Every run records progress through durable checkpoints and user-visible status. Failed fake tools create blocker state and a failure category; they are not hidden behind generic `in_progress`. Repeated or significant failures update `next_focus` and append to `approach_history`.

Terminal behavior:

- completed finite mission: write final checkpoint, terminal report, project-memory summary, and quiesce future fires;
- budget exhausted: stop before unapproved spend, write blocker and terminal report, no live external action;
- missing goal/definition/budget: deny before mission identity allocation and schedule no work;
- policy-denied live action: record denied action and approval gate outcome, send nothing;
- recursive spawn request: deny recursive mission creation and allow at most bounded child tasks.

## 6. LFD profile contract

Profile name: `missions`.

The runner reads visible case inputs from `lfd/missions/eval/dev/cases/*.json` and holdout cases from `$LFD_STATE_ROOT/holdout/missions/cases/*.json`. It emits one outcome per case using the shared schema.

### `setup.profile_extra`

The profile-specific input shape is:

```json
{
  "tenant_id": "tenant-lfdmission-blue",
  "agent_id": "agent-missions",
  "project_id": "project-support",
  "clock_start": "2026-07-07T09:00:00Z",
  "mission_ref": "support-queue-20260708",
  "mission_request": {
    "mission_ref": "support-queue-20260708",
    "goal": "...",
    "definition_of_done": "...",
    "budget": {
      "usd_limit": 8,
      "model_call_limit": 64,
      "wall_clock_minutes": 160,
      "requires_explicit_approval_for_overrun": true
    },
    "mission_kind": "finite|standing",
    "allowed_action_classes": ["read_memory", "fake_tool", "write_checkpoint", "schedule_routine", "schedule_task"],
    "forbidden_action_classes": ["live_external_send", "unapproved_paid_api"],
    "trigger_ref": "trg-support-queue-20260708",
    "trigger_source": "cron|manual|event"
  },
  "timeline": [
    {"at": "2026-07-07T09:00:00Z", "op": "mission_request", "mission_ref": "..."},
    {"at": "2026-07-07T09:00:00Z", "op": "fire_trigger", "trigger_ref": "...", "fire_slot": "..."},
    {"at": "2026-07-07T09:07:00Z", "op": "restart_runtime", "expect_resume_mission_ref": "..."}
  ],
  "expected_failure_direction": false
}
```

Top-level `setup.memory_docs` seeds project memory. Top-level `setup.triggers` seeds trigger definitions. `inbound` entries represent user mission requests, scheduler fires, duplicate fires, and delayed event payloads. `llm_script` supplies deterministic fake model/tool steps for the mission turn.

### State queries

The runner executes every `state_queries` entry after the scenario against persisted state. It must not synthesize answers from case JSON.

- `mission_record` params `{mission_ref}` returns `{exists, mission_id, goal, definition_of_done, definition_version, budget, status, terminal_reason, duplicate_identity_count, identity_after_restart, identity_after_update, manual_fire_reused_identity, last_trigger_payload_digest_present, next_focus, approach_history_count, denial}`.
- `mission_plan` params `{mission_ref}` returns `{kind, allowed_action_classes, forbidden_action_classes, created_from, goal_hash, definition_hash}`.
- `checkpoint_log` params `{mission_ref}` returns `{count, durable, survived_restart, retained_after_budget_rollover, latest_checkpoint_at, final_checkpoint, entries}`.
- `budget_ledger` params `{mission_ref}` returns `{usd_limit, spent_usd, model_call_limit, model_calls_used, overrun, blocked, window_rollover_applied}`.
- `scheduler_decisions` params `{mission_ref}` returns `{routine_count, task_count, spawn_attempts, spawned_mission_count, accepted_fire_count, duplicate_mission_count, denied_actions, recursive_spawn_denied, post_completion_fire_count}`.
- `prompt_envelope` params `{mission_ref, fire_index}` returns `{goal, definition_of_done, memory_doc_count, source_doc_paths, memory_digest, minimal_prompt, trigger_payload_digest_present, used_latest_memory_version}`.
- `mission_progress` params `{mission_ref}` returns `{current_status, completed_steps, blockers, last_failure_category, memory_summary_count}`.
- `status_report` params `{mission_ref}` returns `{visible, spend_visible, next_step, blockers_visible, terminal_report_present, checkpoint_age_visible}`.

## 7. Eval composition

Dev has 30 cases. Nine are failure/denial directions: missing budget, missing definition of done, budget exhaustion, unapproved paid action, live outreach denial, duplicate fire, infinite spawn guard, missing memory fail-soft, and visible failed-tool blocker. Every dev case includes state queries and at least one required state/event/tool/gate matcher.

Holdout has 12 cases outside the repo under `/Volumes/NVME/ironclaw-lfd/holdout/missions/`. Holdout introduces structurally different entities and one unseen adaptation category (`calendar_conflict`). Holdout answers are never readable by the optimizer.

## 8. Non-goals

- No live external outreach, purchases, social posting, scraping gated sources, or production API sends.
- No v1 `src/` feature expansion. Build Reborn-side.
- No separate mission-specific agent loop outside the existing Reborn runner/driver/executor path.
- No LLM calls for deterministic routing, budget checks, idempotency, permission checks, or status-code handling.
- No scorer, shared schema, Cargo, docs, or other lane edits during optimization.

## 9. Rollback and risk notes

Mission state touches scheduling, long-running turns, budget enforcement, project memory, and user-visible status. The safest rollback is feature-flagging MissionManager creation/submission while leaving existing trigger and turn stores intact. Do not migrate or delete existing trigger data as part of this lane. If a new mission store schema is added, it must support PostgreSQL and libSQL and include idempotent create/update semantics keyed by mission identity.
