# ironclaw_run_state guardrails

- Own durable invocation state, approval request records, and gate records (the model-visible `GateRecord` a pending gate renders from, keyed by `GateRef`; arch-simplification §5.2.9).
- Do not own authorization policy, approval resolution, dispatch, runtime execution, process lifecycle, or product workflow.
- All lookups and transitions are resource-owner scoped (tenant/user/agent/project/mission/thread); wrong-scope access must look unknown.
- Durable persistence is the `Filesystem*Store` pair over a `ScopedFilesystem`; there are no separate per-backend run-state stores. The PostgreSQL/libSQL choice (gated by the `postgres`/`libsql` features) is made at the `RootFilesystem` layer underneath, not here. Writes use compare-and-swap (`CasExpectation::Version`) over versioned roots; only byte-only/`Unsupported` roots fall back to process-local serialization.
- Do not persist raw replay input or runtime output in run-state records.
- Keep approval records as control-plane state, not authority by themselves.
