# Goal: let admins configure shared skills and tools without collapsing user boundaries

Source page: https://app.notion.com/p/36e29a6526bf80408720fa4c6c636df6

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for tenant-level admin capability configuration. The spec must define tenant, project, admin, user, shared capability, private capability, default permission, user override, and per-user auth binding.

The roadmap says projects are access boundaries, not capability bundles. Preserve that distinction. Stage 0 must include tests for:

- Admin-created shared tool or skill visible to tenant users.
- User-created private tool or skill visible only to that user.
- Admin default permission resolved deterministically with user override.
- Admin-configured tool requiring per-user auth without sharing another user's credentials.
- Audit and diagnostics for effective capability resolution.

## Target (outer loop)

Optimize multi-tenant capability configuration score:

- 30% admin-created shared capability is visible and invocable by intended tenant users.
- 25% per-user auth binds safely to admin-configured capability.
- 20% user private capability remains private and does not pollute admin defaults.
- 15% admin defaults and user overrides resolve deterministically.
- 10% audit, UI/API diagnostics, and troubleshooting are correct.

Bar: at least 0.92 holdout, zero cross-tenant exposure, zero cross-user credential use.

## Eval design

Create 50 dev and 120 holdout tenant/project/user matrices. Inputs include admin shared tools, admin shared skills, user private tools, user private skills, per-user auth-required shared tools, denied tools, override conflicts, deleted admin capability, tenant switch, and same user id in two tenants.

Answers include visible registry, effective permission decision, credential subject, invocation result, audit event, and diagnostic message class.

## Harness design

Drive product workflow or WebUI facades with fake tenant and user identities. Invoke representative capabilities through the real registry and authorization path rather than inspecting config only.

`harness/score.sh` must:

- Query visible capability registry for each user.
- Attempt invocation and assert effective permission.
- Assert credential lookup subject for auth-required tools.
- Assert private capability isolation.
- Run cross-tenant and same-id collision probes.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $15 LLM/API spend; no live external tools.
- Surface allowlist: `crates/ironclaw_product_workflow/src/reborn_services*`, `crates/ironclaw_reborn_composition`, `crates/ironclaw_skills`, `crates/ironclaw_extensions`, authorization/approvals, operator config tests, and related docs.
- Capacity caps: shared caps; admin/user matrix seed fixtures capped at 40 visible rows.
- New behavior belongs in Reborn-side product services, not v1 `src/tools/permissions.rs`.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include at least one shared capability case and one isolation case. If a permission change is made, run the equivalent invocation to verify the decision gates the actual side effect.

## Entropy rules

- Rotate between visibility, invocation, credential binding, override resolution, and diagnostics every 4 cycles.
- If visibility passes but invocation fails, target registry-to-runtime wiring before adding UI or config polish.
- If isolation probes fail, all further cycles target isolation until the cross-scope score is perfect.

## Cheat audit

Lane-specific cheap wins to block:

1. Flatten tenant and user scope; probe reuses ids across tenants.
2. Grant all shared tools by default; denied and override cases fail.
3. Use admin credentials for user invocations; credential subject is scored.
4. Put new behavior in legacy permissions code; lint rejects new v1 feature path.
5. Inspect config without invoking tools; scorer requires actual invocation.
6. Hide private tools from UI but leave runtime accessible; invocation probes catch it.
7. Make user overrides mutate admin config; audit and visibility answers catch it.
8. Treat project as capability bundle; project-boundary cases fail.
9. Hardcode capability names from eval; probe swaps names and ids.
10. Skip audit diagnostics; diagnostics are part of the score.

## Stop conditions

Stop when holdout is at least 0.92 with zero isolation failures and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any user can invoke another tenant's or user's capability/credential.

