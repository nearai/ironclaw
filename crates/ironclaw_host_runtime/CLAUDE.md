# ironclaw_host_runtime guardrails

- Own host-side composition shared across Reborn runtime lanes.
- Keep runtime-specific request shapes in the runtime crates; adapters should translate into host API contracts and delegate here.
- Compose low-level services such as `ironclaw_network` and `ironclaw_secrets`; do not duplicate URL parsing, DNS checks, private-IP filtering, HTTP clients, secret stores, or redaction logic in runtime crates.
- Host HTTP egress lives under `src/egress/`: keep request validation/sanitization, credential-source resolution, staged network-policy lookup, staged secret injection, transport dispatch, response sanitization, and response-body storage as separate pipeline steps instead of rebuilding a monolithic service method.
- Production host HTTP egress must be constructed with staged `NetworkObligationPolicyStore` and `RuntimeSecretInjectionStore` handoffs. Request-carried policy and direct `SecretStoreLease` sources are legacy/test compatibility paths only.
- Preserve the accounting invariant: `network_egress_bytes` is outbound request bytes only, with response bytes tracked separately.
- Keep raw secret material inside the narrow lease/injection path. Reject runtime-supplied manual credentials, scan raw and percent-decoded URL forms, redact leased values from runtime-visible errors and responses, strip sensitive response headers, and block credential-shaped runtime requests/responses before they reach external services or runtime callers.
- Do not own product workflow, authorization/approval policy, persistence migrations, or event emission unless a later Reborn contract explicitly moves that composition here.

## Agent-loop touch points

- `turn_scheduler.rs` owns scheduler-backed run concurrency. It does not own the
  canonical loop tick or product inbound serialization.
  - Concurrency invariant: the scheduler runs up to `max_concurrent_runs` runs in
    parallel, one `tokio::spawn` + `OwnedSemaphorePermit` per run. The single
    `TurnRunnerId` is the scheduler *instance*, not a per-run worker — seeing one
    `TurnRunnerId` is expected and does NOT mean serial execution.
  - `TurnRunSchedulerConfig::default().max_concurrent_runs` must equal
    `DEFAULT_MAX_CONCURRENT_RUNS` (do not re-introduce a divergent literal —
    `Default` previously hard-coded `4` while production used `16`). The
    production worker-count default `ironclaw_reborn::DEFAULT_TURN_RUNNER_WORKER_COUNT`
    is *derived from* `DEFAULT_MAX_CONCURRENT_RUNS`, so the scheduler cap and the
    worker-count default are the same value by construction; keep the constant
    above `1`. A configured `worker_count = 1` is legal but serializes all runs
    through one slot — the CLI resolver warns on it.
- `surface.rs` owns host-runtime capability-surface shaping and versions.
- `production.rs` and `services.rs` compose runtime services and readiness
  evidence used by Reborn loop wiring.
- Production wiring must reject local-only runtime policy shapes, not just require
  that some `EffectiveRuntimePolicy` value is present.
- First-party runtime tools belong under `first_party_tools/`; do not append new
  built-ins to broad runtime files.

## Adding code

- Add a new runtime service module when the service has its own authority,
  readiness, or resource accounting boundary.
- Add a first-party tool file per capability, except for tightly-coupled
  v1-compatible coding-tool families that share one legacy surface contract.
- Keep readiness checks near the runtime service they validate; driver/product
  readiness belongs in `ironclaw_reborn`.

## Common mistakes

- Do not call `AgentLoopDriver` or compose loop families here.
- Do not own product adapter routing or workflow idempotency.
- Do not bypass host API contracts with runtime-specific shortcuts.
