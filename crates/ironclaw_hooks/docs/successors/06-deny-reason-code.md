# Successor PR: `DenyReasonCode` closed-vocabulary enum

> Successor work from PR #3573 — real-hooks ergonomics finding F2
> (deferred). Adds a curated vocabulary of model-visible denial reasons
> so hook authors can communicate *why* a deny happened without opening
> a free-form prompt-injection channel.

## Problem

Today the model-visible `GateDecisionView::Deny { reason }` collapses
every Installed-tier deny to the static label `hook_predicate_denied`
(see `installed_hook.rs::evaluate`). The collapse is deliberate —
manifest reason strings are author-controlled and would let a malicious
extension smuggle prompt-injection content through the deny path.

The cost: the agent can't tell *why* a hook denied. "Daily cap
exceeded" vs "blocklisted capability" vs "amount over limit" are
useful signals; one undifferentiated `hook_predicate_denied` is not.

## Scope

1. Introduce `DenyReasonCode` enum (closed vocabulary) in
   `ironclaw_hooks::predicate`. Initial set (subject to design review):
   - `Generic` (default, matches today's `hook_predicate_denied`)
   - `RateLimit`
   - `ValueCap`
   - `Blocklist`
   - `RequiresApproval` (when paired with PauseApproval — different
     decision but same reason space)
   - `OutOfPolicy`
2. Extend `OnExceededAction` with a parallel `DenyWithCode` variant:
   ```rust
   pub enum OnExceededAction {
       Deny { reason: String },                                    // existing
       DenyWithCode { code: DenyReasonCode, reason: String },      // new
       PauseApproval { reason: String },
   }
   ```
3. `GateDecisionView::Deny` carries the code as a `&'static str` so
   the model-visible label is stable + auditable. Free-form `reason`
   still goes to audit, never to model.
4. Same shape for `PauseApproval` (a separate `PauseReasonCode`).

## Rejected designs

- **Open string label** — defeats the purpose; restores the
  prompt-injection vector.
- **Numeric code only (no human label)** — harder to read in audit
  logs and forces a separate code-table doc to interpret.

## Required tests

1. Manifest with `DenyWithCode { code: RateLimit, .. }` → outcome
   carries the `rate_limit` label, not `hook_predicate_denied`.
2. Existing `Deny { reason }` manifest still produces
   `hook_predicate_denied` (back-compat).
3. Serde round-trip on the enum.
4. Threat-model regression: a hook author can NOT pass an arbitrary
   `&str` through the new variant — only the enum vocabulary is
   exposed model-side.

## What this PR does NOT do

- Extend the vocabulary to cover every conceivable policy denial. Keep
  the initial enum small; add variants as use cases emerge.
- Localize the labels. They're machine-readable identifiers; the UI
  layer (not in scope) maps them to user-facing strings.

## Risk

Small. Single enum + small projection change. The threat-model risk is
in the *initial vocabulary choice* — if a label is too descriptive
(e.g., `daily_polymarket_cap_exceeded`), authors will gravitate toward
encoding free-form content in label choice. Mitigation: keep the
enum coarse-grained.

## Effort

Small. One enum, one projection change, ~3 tests.
