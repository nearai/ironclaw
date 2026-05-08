# ironclaw_host_api guardrails

- Own shared authority vocabulary only: IDs, scopes, paths, actions, decisions, resources, approvals, audits, and dispatch port contracts.
- Do not depend on any other `ironclaw_*` system-service or runtime crate.
- Keep behavior to validation/serialization helpers; do not add runtime execution, persistence, policy engines, or product workflow.
- Serializable API types must not contain raw `HostPath`, secrets, or backend-specific error details.
- Prefer strong enums/types over strings when the shape is known.
