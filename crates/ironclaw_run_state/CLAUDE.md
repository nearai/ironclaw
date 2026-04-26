# ironclaw_run_state guardrails

- Own durable invocation state and approval request records.
- Do not own authorization policy, approval resolution, dispatch, runtime execution, process lifecycle, or product workflow.
- All lookups and transitions are tenant/user scoped; wrong-scope access must look unknown.
- Do not persist raw replay input or runtime output in run-state records.
- Keep approval records as control-plane state, not authority by themselves.
