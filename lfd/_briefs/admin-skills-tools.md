# LFD Brief: admin-skills-tools — Admin configurable skills/tools

**State**: partial — webui_v2 settings.tools routes exist (caller-scoped);
operator-gated admin surface + backing store are scaffolding (403 on missing
`operator_webui_config`). **Bar**: 0.90 holdout. **Profile**: `admin_config`.

## Outcome

An operator-scoped admin surface: per-tenant and per-user tool + skill
allowlists that persist, are enforced AT DISPATCH (not just hidden in UI),
compose with the existing per-tool permission/auto-approve settings, emit
audit records for every admin mutation, and gate skill selection through
the allowlist during skill scoring.

## Spec sources

- `crates/ironclaw_webui_v2/CLAUDE.md` (settings.tools routes, operator
  capability boundary), `crates/ironclaw_authorization/` (typed decisions),
  `crates/ironclaw_approvals/`
- `crates/ironclaw_skills/` (selection pipeline: gating → scoring → budget →
  attenuation — allowlist plugs into gating/attenuation)
- v1 `src/tools/permissions.rs` (behavioral reference)
- "Everything Goes Through Tools" dispatch rule (root CLAUDE.md) — admin
  mutations route through ToolDispatcher, not direct store writes.

## Stage 0 inner suite

`ironclaw_webui_v2` crate + JS tests, `ironclaw_authorization`,
`ironclaw_skills` tests. Green every cycle.

## Eval themes (dev ~35 / holdout ~12)

1. Enforcement at dispatch (8): operator sets tenant allowlist →
   non-listed tool invocation → typed deny decision + gate event; listed
   tool → allowed. Contract asserts at the dispatch seam (decision events +
   absent tool_invocation), NOT UI listing.
2. Operator boundary (5): non-operator calls admin routes → 403/typed
   denial (required status); operator identity comes from capability
   check, not user-id string compare (holdout uses a different operator
   principal).
3. Precedence (6): tenant allowlist ∩ user override ∩ per-tool permission
   compose with documented precedence (state_pred on effective policy;
   cases cover each layer winning).
4. Persistence (5): allowlists survive restart (group storage reload);
   mutation audit records exist per change (state query; forbidden:
   mutation without audit record — the dispatch-exempt rule).
5. Skill gating (6): skill outside allowlist is not selected even when
   keyword-matched (envelope contracts: skill absent; allowed skill
   present under same inputs).
6. Fail-soft (5): malformed allowlist payloads → typed validation errors,
   store unchanged (state_eq to prior).

## Feature-specific cheats → fences

- **UI-only enforcement** (hide tools, dispatcher still executes) →
  theme-1 contracts require the deny at dispatch (decision event) with the
  tool_invocation forbidden.
- **Hardcode eval tool names** → eval uses SYNTHETIC extension tools with
  generated names; probe renames them; caps: dev tool-name literals in
  `crates/**` diff = 0.
- **Operator check via user string** → holdout operator principal differs;
  state_pred requires capability-check event.
- **Allow-by-default on store miss** → holdout includes empty-store cases
  whose contract is deny-by-default for admin-scoped surfaces (per repo
  security posture: authority boundaries fail closed).
- **Audit records fabricated without mutation** (or vice versa) → paired
  matchers: mutation state change AND audit record, forbidden on either
  alone.

## caps.json extras

Synthetic tool-name literals from dev set in diff: max 0. New
`dispatch-exempt` annotations in diff: max 0 (admin surface must go through
the dispatcher).

## Live mode

3 live cases: real model asked to use a denied tool → required: surfaced
denial explanation referencing policy (reply_contains class), forbidden:
invocation attempt loop > 2 (no-progress discipline).
