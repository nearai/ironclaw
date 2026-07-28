# ironclaw_host_ingress guardrails

- Own only Axum-carrying host ingress mount carriers.
- Keep route/policy vocabulary in `ironclaw_host_api`; this crate consumes those
  descriptors and pairs them with concrete routers.
- Do not add product workflow, auth verification, listener binding, persistence,
  provider clients, runtime services, or WebUI-specific policy.
- This crate may depend on Axum and `ironclaw_host_api`; any additional
  workspace dependency needs architecture-test coverage and an explicit reason.

