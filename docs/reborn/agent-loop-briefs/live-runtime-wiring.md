# WS-16 — Reborn Runtime Wiring + Real Default-Path Smoke

**Workstream:** WS-16
**Crates touched:** `ironclaw_reborn` + `ironclaw_reborn_cli` + `ironclaw_loop_support` + `ironclaw_turns` as needed for wiring-only contract gaps
**Depends on:** WS-14 plus the integrated WS-9 / WS-10 / WS-11 / WS-12 / WS-13 / WS-15 adapter set
**Feeds:** WS-17 product cutover
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §12

---

## 1. Goal

WS-16 is the first point where the Reborn runtime can run the planned
loop on the normal no-profile path with real host adapters composed.
It is not the final product cutover. It proves the runtime wiring is
complete enough that product entrypoints can safely depend on it.

The difference from WS-14 is important:

- WS-14 registers the planned driver and adds the planned run profile.
- WS-16 builds the actual runtime composition that uses that driver,
  resolver, coordinator, runner, host factory, and adapter set together.
- WS-17 makes a product-facing path use this composition by default.

## 2. Scope

1. Add a Reborn runtime composition helper in `ironclaw_reborn`, for
   example `build_default_planned_runtime(...)` or an equivalent
   `RebornRuntimeLoopComposition` type.
2. The helper composes:
   - `DriverRegistry` with `TextOnlyModelReplyDriver` and
     `PlannedDriver` registered.
   - `RunProfileResolver` with `reborn-planned-default` as the
     implicit no-profile default.
   - `DefaultTurnCoordinator` using that resolver.
   - `TurnRunnerWorker` using the same registry and coordinator.
   - `RebornLoopDriverHostFactory` with real adapter instances from
     WS-9 / WS-10 / WS-11 / WS-12 / WS-13 / WS-15.
3. Make planned runs build hosts with the profile-scoped capability
   surface instead of the current empty/no-capability text-only host
   path. Explicit text-only runs may continue to use the text-only
   host shape.
4. Ensure the runtime shell in `ironclaw_reborn_cli` can exercise this
   composition, even if the root product still has not cut over.
5. Preserve explicit legacy/profile-selected text-only behavior.

## 3. Non-goals

- No root application or web/channel default cutover. That belongs to
  WS-17.
- No removal of `TextOnlyModelReplyDriver`.
- No new loop family beyond `families::default()`.
- No broad model-route-chain work beyond what the planned default path
  already needs.

## 4. Verification

WS-16 is gated by runtime-level caller tests, not helper-only tests.

Required coverage:

1. A no-profile turn through `DefaultTurnCoordinator` +
   `TurnRunnerWorker` resolves `reborn-planned-default`, finds the
   planned driver in `DriverRegistry`, builds a real host, and returns
   `LoopExit::Completed`.
2. A capability/tool request goes through `HostRuntimeLoopCapabilityPort`
   with a test dispatcher and a profile-derived allow set. The test
   must not use `MockAgentLoopDriverHost` as the execution host.
3. Identity context from `WorkspaceIdentityContextSource` appears in
   the prompt bundle on the planned default path.
4. Checkpoint load/resume and cancellation handle reads are exercised
   through the composed runtime ports.
5. An explicit text-only profile still routes to
   `TextOnlyModelReplyDriver`.

If a test uses in-memory stores, that is acceptable. What matters is
that the same runtime composition and adapter boundaries are used, not
the mock driver host from the WS-14 registration smoke.
