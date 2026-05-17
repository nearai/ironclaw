# WS-17 — Product-Live Readiness Evidence

**Workstream:** WS-17
**Crates touched:** `ironclaw_host_runtime` + product entrypoint crates that submit normal turns + `ironclaw_product_workflow` tests
**Depends on:** WS-16
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §12

---

## 1. Goal

WS-17 proves the product-facing no-profile turn path can be composed with the
WS-16 Reborn runtime and that the composition fails closed when required live
adapters are missing or inert.

This brief is not, by itself, the production app/gateway cutover. Do not claim
the Reborn agent loop is fully live or the default product path until the
external cutover issue wires the production product composition and the external
tool-result evidence issue proves tool-using runs are visible through the
product surface.

## 2. Scope

1. Add product-facing contract coverage that submits a normal no-profile turn
   through the WS-16 default planned runtime composition.
2. Add a readiness check that fails closed when required pieces are
   missing: driver registry entries, implicit planned profile,
   capability surface resolver, model routing, checkpoint stores,
   input queue, progress sink, an explicitly product-controllable
   cancellation source, and identity context source.
3. Preserve an explicit text-only or legacy profile path for rollback
   and controlled comparison.
4. Emit enough local run evidence to prove the planned loop handled the test turn:
   selected run profile, selected driver id, milestone/progress events,
   checkpoint writes, capability execution evidence when a tool is
   requested, and final assistant message persistence.
5. Keep feature-flag or rollout branching at the product composition
   boundary. Do not scatter planned-loop conditionals through host
   adapters or the runner.

## 3. Non-goals

- No removal of legacy text-only runtime support.
- No migration of every named run profile.
- No new capability family or tool policy model.
- No new database schema unless a required readiness or evidence record
  cannot be represented by existing event/checkpoint tables.
- No broad product binding/run ownership rewrite. The current Reborn runtime
  composition accepts a `ThreadScope` at construction time; a real per-run,
  per-user product binding must be handled by the production product composition
  follow-up instead of being hidden inside loop code. The follow-up issue must
  explicitly cover production app/gateway cutover, per-run/user `ThreadScope`
  binding, and product-visible tool-result evidence.

## 4. Verification

WS-17 needs product-facing tests because this is the readiness gate before live
cutover.

Required coverage:

1. A normal product turn with no explicit profile uses
   `reborn-planned-default`, runs the planned driver, and persists the
   assistant reply in the product-visible turn/thread state.
2. A tool-using planned-loop turn executes through the host capability
   path and records progress/evidence events. If product-surface tool-result
   evidence is not in this branch, leave the live/default claim blocked on the
   external tool-result evidence issue.
3. Startup/readiness fails closed when a required adapter is absent or
   misconfigured.
4. An explicit legacy/text-only profile still routes to the legacy
   driver.
5. If the cutover stores new persistent state or config, cover both
   PostgreSQL and libSQL. If it only consumes existing persistence,
   use the existing product-workflow persistence test matrix.

After WS-17 lands, it is accurate to say the product-live readiness gate exists
and the local product workflow can exercise the planned runtime. It is not
accurate to say the Reborn agent loop is fully live on the default product path
until production app/gateway cutover and product-visible tool-result evidence
land.
