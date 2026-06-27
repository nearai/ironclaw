# Follow-up: extract the Reborn budget feature into focused modules

Status: planned (tracking doc for the file-size findings from the
2026-06-27 thermo-nuclear code-quality review of
`codex/reborn-queued-messages-webui`).

## Context

The queued-messages-webui branch also carried a budget approval-gate +
budget-settings feature. That feature was added inline to two already-large
files, pushing them past the `.claude/rules/architecture.md` §5 size budget:

- `crates/ironclaw_reborn_composition/src/webui.rs` — 1527 → 2209 lines (+682),
  almost entirely the two budget services
  (`BudgetResourceGateResolutionService`, `BudgetSettingsService`) plus their
  ~18 private helpers.
- `crates/ironclaw_product_workflow/src/reborn_services.rs` — 6368 lines
  (already > 3000); the budget DTOs + `ResourceGateResolutionService` /
  `BudgetSettingsService` traits add ~347.

The review-fix PR already did the in-place quality fixes the budget code needed
(typed `BudgetGateId` gate-ref via M2, single `raised_budget_limit` policy via
M3, `RebornServicesError` constructors via §4) and extracted the read-side
budget detail rendering out of `turn_events.rs` into
`projection/budget_gate_details.rs`. The two larger module extractions below are
deliberately deferred to this focused follow-up so the review-fix PR stays
reviewable and the moves can be verified on their own.

## Planned extractions

1. `crates/ironclaw_reborn_composition/src/webui_budget.rs`
   - Move `BudgetResourceGateResolutionService`, `BudgetSettingsService`,
     `runtime_budget_handles`, and the budget free helpers
     (`budget_gate_id_from_ref`, `map_budget_gate_error`,
     `map_resource_error`, `map_budget_settings_resource_error`,
     `budget_gate_not_found`, `budget_gate_conflict`, `ensure_dimension_limit`,
     `raise_dimension_limit`, `decimal_budget_target`, `integer_budget_target`,
     `max_decimal_limit`, `max_integer_limit`, `clamp_u32_limit`) plus the two
     budget tests.
   - Export only the two service types + `runtime_budget_handles` to `webui.rs`.
   - Also fold the `#[cfg(feature = ...)]` `RebornProductionRuntimeServices`
     budget-handle extraction into a runtime-owned `budget_handles()` accessor
     (CLAUDE.md "module-owned initialization") so the cfg branching leaves
     `webui.rs`.

2. `crates/ironclaw_product_workflow/src/reborn_services/budget.rs`
   - Move `ResourceGateResolutionService` + `RejectingResourceGateResolutionService`,
     `BudgetSettingsService` + `RejectingBudgetSettingsService`, and the budget
     DTOs (`RebornBudgetThresholdView`, `RebornBudgetAccountView`,
     `RebornBudgetSettingsResponse`, `ResourceGateResolutionRequest`,
     `ResourceGateResolutionDecision`).
   - Leave only the `Arc<dyn ...>` fields and trait-method delegations in
     `reborn_services.rs`.

## Out of scope (already done in the review-fix PR)

- Typed `BudgetGateId::{to_gate_ref, from_gate_ref, is_budget_gate_ref}` — the
  `gate:budget-<uuid>` convention now has one source of truth.
- `ResourceValue` arithmetic/`display` + the single `raised_budget_limit`
  policy in `ironclaw_resources` (preview and apply paths cannot drift).
- `projection/budget_gate_details.rs` extraction from `turn_events.rs`.
