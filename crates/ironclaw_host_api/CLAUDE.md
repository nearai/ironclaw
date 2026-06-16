# ironclaw_host_api guardrails

- Own shared authority vocabulary only: IDs, scopes, paths, actions, decisions, resources, approvals, audits, dispatch port contracts, and host-owned ingress descriptors.
- Do not depend on any other `ironclaw_*` system-service or runtime crate.
- Keep behavior to validation/serialization helpers; do not add runtime execution, persistence, policy engines, or product workflow.
- HTTP ingress contracts are route/policy vocabulary only. Listener binding, Axum/router mounting, auth enforcement, scope extraction, body/rate limits, CORS/Origin checks, audit emission, and effect dispatch belong to host composition.
- Serializable API types must not contain raw `HostPath`, secrets, or backend-specific error details.
- Prefer strong enums/types over strings when the shape is known.
