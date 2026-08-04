# ironclaw_host_api guardrails

- Own shared authority vocabulary only: IDs, scopes, paths, actions, decisions, resources, approvals, audits, dispatch port contracts, host-owned ingress descriptors, and the turn vocabulary (`turn`).
- `turn` is the **complete** canonical turn language, not a partial one: if a crate needs to name a turn — its scope, ids, refs, status, gate kind, event cursor, or origin adapter — it depends on this crate, not on `ironclaw_turns`. When a turn type has to be named outside the turn kernel, the answer is to finish moving it here, never to re-export it from `ironclaw_turns`.
- Do not depend on any other `ironclaw_*` system-service or runtime crate.
- Keep behavior to validation/serialization helpers; do not add runtime execution, persistence, policy engines, or product workflow.
- HTTP ingress contracts are route/policy vocabulary only. Listener binding, Axum/router mounting, auth enforcement, scope extraction, body/rate limits, CORS/Origin checks, audit emission, and effect dispatch belong to host composition.
- Serializable API types must not contain raw `HostPath`, secrets, or backend-specific error details. The narrow exception is bounded `ModelDiagnostic`: producers must scrub credential values and fence injection-shaped text before carrying a cause needed for model recovery.
- Prefer strong enums/types over strings when the shape is known.
- No wildcard re-exports. `lib.rs` exposes modules, never a flat prelude:
  consumers import `ironclaw_host_api::<module>::<Type>` (for example
  `scope::ExecutionContext`, `ids::ExtensionId`). Do not add a per-module glob
  re-export — it hides which module a consumer actually depends on, which is
  exactly what carving a vocabulary family out of this crate needs to see. The
  only crate-root item is the `Timestamp` alias.
