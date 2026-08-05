# ironclaw_host_ingress guardrails

- Own only Axum-carrying host ingress mount carriers.
- Keep route/policy vocabulary in `ironclaw_host_api`; this crate consumes those
  descriptors and pairs them with concrete routers.
- Do not add product workflow, auth verification, listener binding, persistence,
  provider clients, runtime services, or WebUI-specific policy.
- This crate may depend on Axum and `ironclaw_host_api`, and on nothing else in
  the workspace — **in any dependency kind**, `[dependencies]`,
  `[dev-dependencies]`, and `[build-dependencies]` alike. That is now
  **enforced as an allowlist**, not just stated here:
  `assert_host_ingress_names_no_other_workspace_crate` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` derives
  the forbidden set as "every workspace `ironclaw_*` crate except
  `ironclaw_host_api`", so a dependency nobody thought to blocklist still fails,
  and it reads every kind rather than only `normal`. The all-kinds scope is
  deliberate and unlike the layer matrix beside it: a dev-dependency on a
  higher layer is legal elsewhere in the workspace, but a dev- or
  build-dependency here still resolves and builds, which defeats the one
  property this crate exists to provide.
- **Layer `substrates`** (WS2 re-layer, #7092 — it was `products` until then).
  The layer matrix will not hold the rule above on its own: `substrates →
  substrates` is legal, so the matrix would permit `ironclaw_filesystem` or
  `ironclaw_secrets` here. The allowlist is what refuses them. Do not "fix" a
  new dependency by adding it to the allowlist — the crate exists so that a
  consumer wanting only the carrier shapes never compiles a listener's
  dependency cone.

