# Rework: `TriggerSchedule::Once { at }` — first-class one-shot

Replaces the cron-encoded one-shot (year-pinned cron + parallel `completion_policy`
field) with a first-class schedule variant. Deletes the compensation workarounds.

## Design decisions (authoritative — all agents follow this)

### 1. Schedule type (`crates/ironclaw_triggers/src/lib.rs`)
```rust
pub enum TriggerSchedule {
    Cron { expression: String, timezone: String },     // recurring (unchanged)
    Once { at: DateTime<Utc>, timezone: String },       // fires once at `at`, then terminal
}
```
- `next_slot_after(after)`:
  - `Cron` → unchanged.
  - `Once { at, .. }` → `Ok(if at > after { Some(at) } else { None })`.
- `validate()`: `Once` requires `timezone` parse-valid; `at` any instant (future-ness is a
  create-time concern, not a schedule invariant). Cron unchanged.
- Constructor: `TriggerSchedule::once(at, timezone)`.

### 2. Completion is DERIVED, not stored as policy
- **Delete `completion_policy` from the `TriggerRecord` struct** and from all Rust logic.
- A trigger is "done" when, after a fire, `schedule.next_slot_after(fired_slot)` is `None`.
  True for `Once` (always, after its slot) and for an exhausted finite cron. Uniform rule.
- DB: keep the `completion_policy` NOT NULL column (avoid destructive rebuild). On upsert,
  write a **derived** value: `Once → "complete_after_first_fire"`, `Cron → "recurring"`.
  On read, **ignore** it (reconstruct nothing from it).

### 3. Fire-request contract simplified
- **Remove `next_run_at: Option<Timestamp>` from `FireAcceptedRequest` and `FireReplayedRequest`.**
- `mark_fire_accepted` / `mark_fire_replayed` compute the next slot internally from the
  record's own schedule: `let next = record.schedule.next_slot_after(fire_slot)?;` and advance
  `record.next_run_at = next` **only if `Some`** (leave unchanged if `None`; it's gated by
  `active_run_ref` during the active window and recomputed on clear).
- **Delete** `reject_missing_next_run_at_for_recurring` and the `(None, Recurring)` guards.
- **Delete** the `COALESCE(?, next_run_at)` / `(? IS NULL OR ? > fire_slot)` SQL guards and the
  `CASE WHEN completion_policy = ...` SQL. Backends read the record (already done via
  `SELECT ... FOR UPDATE` / fetch-in-txn), compute the next slot in Rust, and write plain values.

### 4. Completion at run termination (`clear_active_fire`, all 3 backends)
```text
next = record.schedule.next_slot_after(active_fire_slot)
if next is Some(t): state stays Scheduled, next_run_at = t      // recurring keeps going
if next is None:    state = Completed                            // Once / exhausted cron — terminal
```
Replaces the `completion_policy == CompleteAfterFirstFire` CASE.

### 5. Worker (`crates/ironclaw_triggers/src/worker/due_fire.rs`)
- Remove `is_fire_once`, `recurring_next_run_at`, and the `failure_disposition` Terminal-vs-Reschedule
  split keyed on fire-once.
- Pre-submit failure axis is "did the run execute?" — these paths never ran the turn, so they must
  NOT complete the trigger (fail-closed):
  - Retryable failure → `Retryable` (leave Scheduled at fire_slot; retries next poll). Unchanged.
  - Permanent failure → reschedule to `schedule.next_slot_after(fire_slot)` if `Some`; if `None`
    (a `Once`, or exhausted cron), **leave Scheduled at fire_slot** (fail-closed retry) — do NOT
    mark Completed. (This is what makes the `trigger_poller_does_not_submit_turn_for_unpaired_actor`
    test pass: an unpaired one-shot is never Completed.)
  - A trigger only reaches `Completed` via `clear_active_fire` after a real run terminates.
- `active_cleanup.rs`: the "blocked fire-once stays pending" rule now keys on the schedule being
  `Once` (i.e. `next_slot_after(fire_slot).is_none()`-style / `matches!(schedule, Once{..})`),
  not on `completion_policy`.

### 6. Persistence migration (libSQL + Postgres)
- Add a `schedule_kind TEXT NOT NULL DEFAULT 'cron'` column via the existing idempotent
  `ALTER TABLE ... ADD COLUMN` pattern (mirror the `schedule_timezone` migration).
- Write: `Cron` → kind='cron', `schedule_expression`=expression, `schedule_timezone`=timezone.
  `Once` → kind='once', `schedule_expression`=`at` as RFC3339, `schedule_timezone`=timezone.
- Read: branch on `schedule_kind`. 'cron' → `TriggerSchedule::cron_with_timezone(expr, tz)`.
  'once' → parse `expr` as RFC3339 → `TriggerSchedule::Once { at, timezone: tz }`.
- `completion_policy` column: write derived value (see §2), do not read into the domain.

### 7. Create API (`crates/ironclaw_host_runtime/src/first_party_tools/`)
- `schemas.rs` `trigger_create` input: replace `cron` + `timezone` + `completion_policy` with a
  `schedule` object discriminated by `kind`:
  - `{ "kind": "cron", "expression": "...", "timezone": "..." }`
  - `{ "kind": "once", "at": "2027-06-24T17:00:00", "timezone": "..." }`  (local wall-clock in `timezone`)
  - `at` is interpreted in `timezone` and converted to a UTC instant at create time.
- `trigger_management.rs`: parse the schedule, drop `completion_policy` from `TriggerCreateInput`
  and from `create_trigger` (record no longer has the field). One-shot create still validates the
  resulting `at`/cron yields a future `next_run_at`.
- Model output JSON (`trigger_output`): drop `completion_policy`; expose `schedule.kind`.

### 8. Wire DTOs (`reborn_composition` automation.rs, `product_workflow`)
- `RebornAutomationSource` gains a `Once { at, timezone }` arm (or the Schedule source carries kind).
- `map_*` functions: render a `Once` schedule; a one-shot's `next_run_at` after firing is gone
  (state Completed). No `completion_policy` in DTOs.

## Out of scope / keep
- `excluded_states` list filter on `list_scoped_triggers` (keep — orthogonal, owner decided).
- Blocked-stays-pending behavior (keep — now keyed on schedule type).
- The `?include_completed` automations query flag (keep).

## Verification gate (every agent)
- `cargo fmt`; `cargo clippy -p <crate> --all-targets --all-features` zero warnings.
- `cargo test -p ironclaw_triggers --features libsql` green (core), incl. **integration**
  `repository_contract` (run it, don't just `--lib`).
- After peripherals: `cargo test -p ironclaw_reborn_composition --test trigger_poller_e2e` green
  (the unpaired test is the canary), and `cargo build --workspace --all-features`.
