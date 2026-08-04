# ironclaw_host_ingress guardrails

- Own only Axum-carrying host ingress mount carriers.
- Keep route/policy vocabulary in `ironclaw_host_api`; this crate consumes those
  descriptors and pairs them with concrete routers.
- Do not add product workflow, auth verification, listener binding, persistence,
  provider clients, runtime services, or WebUI-specific policy.
- This crate may depend on Axum and `ironclaw_host_api`, and on nothing else in
  the workspace. That is now **enforced as an allowlist**, not just stated here:
  `reborn_crate_dependency_boundaries_hold` in
  `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` derives
  the forbidden set as "every workspace `ironclaw_*` crate except these two", so
  a dependency nobody thought to blocklist still fails.
- **Layer `substrates`** (WS2 re-layer, #7092 — it was `products` until then).
  The layer matrix will not hold the rule above on its own: `substrates →
  substrates` is legal, so the matrix would permit `ironclaw_filesystem` or
  `ironclaw_secrets` here. The allowlist is what refuses them. Do not "fix" a
  new dependency by adding it to the allowlist — the crate exists so that a
  consumer wanting only the carrier shapes never compiles a listener's
  dependency cone.

