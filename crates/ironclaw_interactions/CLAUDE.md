# ironclaw_interactions guardrails

- Own the adapter/UI-safe approval and auth interaction surface. Translate
  scoped blocked run-state (approval records, auth-required gates) into
  redacted product-facing DTOs and route user decisions back through the
  canonical resolution paths.
- Do not become an alternate approval/auth side channel. Source of truth
  remains [`ironclaw_run_state`] approval records / `RunStatus::BlockedAuth`
  records and [`ironclaw_approvals::ApprovalResolver`] / the host-supplied
  `AuthFlowManager`. The interaction services compose these — never bypass
  them and never hold their own decision state.
- Adapter-facing DTOs must be redacted: no raw tool input, approval reasons,
  invocation fingerprints, lease IDs, host paths, secrets, backend
  diagnostics, or runtime output.
- Validate tenant/user/agent/project/thread scope before exposing or
  resolving any record. Wrong-scope lookups must look unknown.
- Do not depend on `ironclaw_dispatcher`, `ironclaw_host_runtime`,
  `ironclaw_mcp`, `ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_engine`,
  or runtime/capability execution crates. The interaction layer is metadata
  + control-plane routing only.
- Tests must exercise approve/deny/resume/cancel happy paths, missing/stale
  records, cross-scope denials, and a no-exposure sentinel that verifies
  the redacted DTO surface does not leak any sentinel marker placed in
  the underlying record.
