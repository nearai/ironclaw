# ironclaw_run_state guardrails

- Own durable invocation state and approval request records.
- Do not own authorization policy, approval resolution, dispatch, runtime execution, process lifecycle, or product workflow.
- All lookups and transitions are resource-owner scoped (tenant/user/agent/project/mission/thread); wrong-scope access must look unknown.
- Filesystem-backed run/approval stores use process-local serialization only; production shared roots need transactional/CAS backends before real multi-process callers.
- Do not persist raw replay input or runtime output in run-state records.
- Keep approval records as control-plane state, not authority by themselves.
