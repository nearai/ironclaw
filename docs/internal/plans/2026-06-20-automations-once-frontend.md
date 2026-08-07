# Plan: Render one-shot (`Once`) triggers in the WebUI Automations list

## Problem

The backend already models one-shot triggers end-to-end:
- Domain `TriggerSchedule::Once { at, timezone }`.
- Model `trigger_list` serializes `{"kind":"once","at":…,"timezone":…}`.
- WebUI wire DTO `RebornAutomationSource::Once { at, timezone }` → `{"type":"once","at":…,"timezone":…}`.

But the **WebUI Automations frontend** (came from `main`, predates the `Once`
variant) does not understand `source.type === "once"`:

1. `automations-presenters.js:54` — `normalizeAutomations` filters
   `source?.type === "schedule"`, so one-shot rows are **dropped from the list
   and every summary count entirely**.
2. `normalizeAutomation` (`:245`) builds `schedule_label` via
   `scheduleLabel(automation.source?.cron, …)`. A `once` source has no `.cron`,
   so it resolves to `"automations.schedule.custom"` → a one-shot would render
   as **"Custom schedule"**, not "Fires once at …".

Net: even though the data flows, the browser would mislabel and undercount
one-shots. (Today they're filtered out before that even shows.)

## Goal

A still-pending one-shot (state `Scheduled`) appears in the default Automations
list and renders a clear "Fires once on <datetime> (<tz>)" label, contributes
to counts correctly, and shows its `next_run` time. No backend changes.

## Out of scope (explicit)

- Surfacing **completed** one-shots in the WebUI. After a one-shot fires it
  soft-completes and the default list intentionally omits it (`include_completed`
  is not requested by `useAutomations`). Adding an `include_completed` toggle +
  a "Completed" filter tab is a separate follow-up (tracked alongside #5083).
  This plan only ensures a *pending* one-shot is shown and labeled correctly.
- Any change to `scheduleLabel`'s cron grammar.

## Design (code-judo: dispatch on the discriminated union)

The source is already a tagged union (`type: "schedule" | "once"`). Today the
presenter *probes* `.cron` ad hoc. Replace that with a single dispatcher so the
union drives the label and a future source kind is a one-line addition, not a
scatter of `.cron`/`.at` probes.

### 1. `automations-presenters.js`

- Add a `SUPPORTED_SOURCE_TYPES` constant `["schedule", "once"]` (single source
  of truth) with a `// Add new source types here as the backend gains them`
  comment, and change the `normalizeAutomations` filter (`:54`) to
  `SUPPORTED_SOURCE_TYPES.includes(automation?.source?.type)`. Keeps unknown
  future sources out of the UI (the original intent of the `=== "schedule"`
  guard) while admitting `once`. **Maintenance invariant:** a new backend source
  type silently vanishes from the UI until added here.
- **Reuse the existing date formatter — do NOT add a third `Intl` block.**
  Extend `formatAutomationDate(value, fallback, locale, timezone)` with an
  optional 4th `timezone` arg: when present, add `timeZone: timezone` to the
  Intl options. The existing `catch` fallback keeps formatting **without** a
  `timeZone` (browser-local) — never substitute UTC. All other callers pass 3
  args and are unaffected.
- Add a PRIVATE (non-exported) `automationScheduleLabel(source, t, locale)`
  dispatcher — switch on the discriminated union, the single call site:
  - `source.type === "once"` → `onceScheduleLabel(source.at, source.timezone, t, locale)`
  - `source.type === "schedule"` → `scheduleLabel(source.cron, source.timezone, t, locale)`
  - else → `tr(t)("automations.schedule.custom")` (defensive; filtered upstream).
- Add `onceScheduleLabel(at, timezone, t, locale)`:
  - If `at` is absent → `"automations.schedule.custom"`.
  - `const datetime = formatAutomationDate(at, null, locale, timezone)`; if it
    returns the `null` fallback (invalid/unparseable) → `"automations.schedule.custom"`.
  - Append the `(tz)` parenthetical the SAME way cron labels do — **concatenated
    OUTSIDE the template**, not embedded:
    `return tr(t)("automations.schedule.onceAt", { datetime }) + tzSuffix;`
    where `tzSuffix = timezone ? ` (${timezone})` : ""`. The i18n template
    therefore has only a `{datetime}` slot, no `{tz}`.
- In `normalizeAutomation` (`:245`) replace the `scheduleLabel(source?.cron, …)`
  call with `automationScheduleLabel(automation.source, t, locale)`.
- Keep `schedule_timezone` as-is (already `source?.timezone || "UTC"`).
- `scheduleLabel` stays cron-only, pure, and exported (other callers / tests use
  it directly). `automationScheduleLabel`/`onceScheduleLabel` are internal; the
  test drives them through `normalizeAutomations` (the caller).

No summary changes needed: `scheduled: automations.length` and the `nextRun`
calc both already work once `once` rows pass the filter (a pending one-shot has
`state: scheduled` and a `next_run_at`).

### 2. i18n — add `automations.schedule.onceAt` to all 11 locale packs

`en.js`: `"automations.schedule.onceAt": "Once on {datetime}"` (the `(tz)` suffix
is appended in code, NOT in the template — so the ONLY placeholder is
`{datetime}`). Add the same key with a faithful translation to: `ar, de, es, fr,
hi, ja, ko, pt-BR, uk, zh-CN`. Place it adjacent to the other
`automations.schedule.*` keys.

**Placeholder-typo risk:** every pack MUST keep the literal `{datetime}` token
(translators sometimes copy `{date}`/`{time}` from neighboring keys — that would
render the raw key). Verify each pack contains exactly `{datetime}` before
shipping.

### 3. Tests — `automations-presenters.test.mjs`

- Update the existing `"normalizeAutomations keeps only schedule rows …"` test
  (it currently feeds a `schedule` + a `webhook` row and asserts `length === 1`).
  Rename to "keeps schedule and once rows, drops unknown" and feed THREE rows —
  one `schedule`, one `once`, one unknown (`webhook`) — asserting **`length === 2`**
  and that the `once` row's `schedule_label` is the once label (not "Custom
  schedule"), while the unknown is dropped.
- Add `automationScheduleLabel` / `onceScheduleLabel` cases:
  - valid once → label contains the localized date + tz parenthetical, and is
    NOT "Custom schedule".
  - missing `at` → falls back to "Custom schedule".
  - `at` rendered in the source `timezone` (assert a tz-sensitive instant maps
    to the expected wall-clock for that tz, not the runner's local tz).
- Keep driving through `normalizeAutomations` (the caller) per repo testing rule,
  not only the leaf helper.

## Files touched

- `crates/ironclaw_webui_v2_static/static/js/pages/automations/lib/automations-presenters.js`
- `crates/ironclaw_webui_v2_static/static/js/pages/automations/lib/automations-presenters.test.mjs`
- `crates/ironclaw_webui_v2_static/static/js/i18n/{en,ar,de,es,fr,hi,ja,ko,pt-BR,uk,zh-CN}.js`

No component changes: `automations-list.js` and `automation-detail-panel.js`
render `automation.schedule_label` generically.

## Validation

- `node --test` on the presenter test (and the package's JS test runner).
- Manual sanity: a pending one-shot row shows "Once on <date> (<tz>)" and a
  recurring row is unchanged.
