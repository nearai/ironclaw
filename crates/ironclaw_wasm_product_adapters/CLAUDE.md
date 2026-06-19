# ironclaw_wasm_product_adapters guardrails

Owns the thin WASM ProductAdapter host-glue boundary for IronClaw Reborn.

- Keep this crate focused on adapter-specific host control-plane glue: protocol-auth verification, adapter manifest egress preflight, and the WIT shape for ProductAdapter components.
- Do not own product workflow, canonical user/thread binding, turn coordination, outbound projection cursors, delivery-status persistence, runtime dispatch, process spawning, authorization/approval policy, or app startup wiring.
- Do not perform production HTTP, DNS/private-IP checks, redirect handling, response limits, secret injection, leak scanning, or redaction here. Actual network egress must delegate through host-runtime services backed by `ironclaw_network` and secret leases.
- WASM components return parsed adapter DTOs only. Host glue stamps trusted adapter/installation/auth/received context before product workflow code sees an inbound envelope.
- Auth evidence is host-minted only. WASM may declare auth requirements and receive sealed evidence; it must never fabricate verified claims.
- No-op/ignored authenticated events must remain explicit payloads in the parsed DTO contract, not an absent parse result.
- Runtime-specific WIT/request shapes may live here, but shared product-surface DTOs stay in `ironclaw_product_adapters`.
- ProductAdapter components may use v1-style minimal WASI p2 for wasm32-wasip2 compatibility: clock/random are allowed; env, args, stdio, preopened directories, inherited network, and DNS lookup must stay disabled. Current slice is parse/render-only: the ProductAdapter WIT `http-egress` import fails closed until host-runtime egress wiring is injected in a follow-up.
- Keep dependencies minimal. Avoid workflow/runtime crates unless a concrete host-glue call site requires them and architecture tests are updated deliberately.

Tests:

- Unit tests cover verifier and egress-policy behavior.
- `ironclaw_architecture` boundary tests pin crate dependencies, local guardrails, and WIT trust-boundary invariants.
