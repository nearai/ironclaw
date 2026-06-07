# ironclaw_runtime_policy guardrails

- Own the runtime policy resolver: `(DeploymentMode, RuntimeProfile, OrgPolicyConstraints) → EffectiveRuntimePolicy`.
- Depend only on `ironclaw_host_api` for vocabulary and `serde`/`thiserror` for plumbing. Do not pull in runtime crates, host runtime, capability host, secrets, network, or product workflow crates.
- Resolution must be **deterministic** and **monotonic with respect to safety**: deployment mode and tenant/org policy may *reduce* the requested profile's authority; they must never *increase* it.
- Fail-closed by default: invalid `(deployment, profile)` pairs are an error, not a silent downgrade. Yolo profiles require explicit caller-supplied disclosure. `EnterpriseYoloDedicated` requires both `EnterpriseDedicated` deployment and explicit org admin approval.
- The resolver is the only sanctioned producer of `EffectiveRuntimePolicy`. Treat values constructed elsewhere as untrusted.
- Output must be serializable for audit/debugging — `EffectiveRuntimePolicy` round-trips through serde and `was_reduced()` flags the narrowing case so audit can render "you asked for X, you got Y".
- Do not re-implement authorization/approvals/grants. The resolver picks backend kinds and policy modes; per-invocation authorization runs on top via `ironclaw_authorization` / `CapabilityHost`.
- No I/O in the resolver. It is a pure function over types from `ironclaw_host_api::runtime_policy`.
