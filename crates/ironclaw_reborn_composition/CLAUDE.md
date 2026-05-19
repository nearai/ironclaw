# ironclaw_reborn_composition guardrails

- Own only top-level Reborn composition for production/app startup.
- Expose facade-shaped handles only: `HostRuntime`, `TurnCoordinator`, readiness.
- Keep lower substrate handles private to factories and owning crates.
- Do not depend on the root `ironclaw` crate or `src/` modules.
- Do not add legacy bridge modes here until an accepted migration contract exists.
- Do not route live v1/product traffic here; callers must opt in through explicit Reborn adapters.
- Production and migration-dry-run profiles must fail closed on local-only or missing required handles.
