# ironclaw_network

The network policy boundary and hardened outbound transport: target/method
policy evaluation, DNS resolution with private-address denial, redirect and
response-size hardening, and the pinned `reqwest` transport. It is the
workspace's sole egress-policy owner and its only sanctioned `reqwest`/TLS
home — a module elsewhere would put that cone in the build graph of every
crate that needed even the policy types.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_network` · **Manifest:**
  `crates/substrates/ironclaw_network/Cargo.toml`
- **Use this when:** an outbound HTTP call must exist — it gets policy-checked,
  resolved with private-IP denial, and sent through the hardened transport.
- **Don't use this when:** you need credentials attached → injection is the
  kernel's obligation handling (`ironclaw_host_runtime`), never done here;
  you're inside a lane → lanes receive a policy-scoped egress handle by
  injection and hold no transport of their own; you're writing a vendor
  allowlist → that is manifest data, not code.

## Public surface

- Policy: `StaticNetworkPolicyEnforcer` (`policy`), URL targeting and
  private-IP checks (`url_target`).
- Egress: `NetworkHttpEgress` port + `PolicyNetworkHttpEgress`,
  `NetworkHttpTransport` (`egress`).
- Resolution: `NetworkResolver` (`resolver`) — denies private and reserved
  addresses before a connection opens.
- Transport: `ReqwestNetworkTransport` (`transport`).
- Types: `NetworkRequest`, `NetworkHttpRequest`/`NetworkHttpResponse`,
  `NetworkUsage`, `DEFAULT_RESPONSE_BODY_LIMIT` (`types`); `NetworkHttpError`
  (`error`).
- Test seam: `RewriteNetworkTransport` (`test_rewrite`) — env-gated,
  loopback-only host rewriting for E2E harnesses, compiled only under
  `debug_assertions` or the `test-support` feature; a release build refuses to
  activate it.

## Depends on / consumed by

- **Depends on (workspace):** `ironclaw_host_api` only. External: `reqwest`
  (sole owner), `url`, `percent-encoding`, `zeroize`, `tokio`, `thiserror`,
  `tracing`.
- **Consumed by (measured 2026-08-05):** 5 normal — `ironclaw_host_runtime`
  (the production caller), `ironclaw_composition` (constructs),
  `ironclaw_extension_host`, `ironclaw_extension_manager`, and
  `ironclaw_sandbox` (the lane's egress-interception machinery; a measured
  deviation from the lanes family target — see
  [`crates/lanes/AGENTS.md`](../../lanes/AGENTS.md)).

## Invariants

- **Fail closed:** no matching target pattern, or no allowed targets configured,
  means deny (crate guardrails, `CLAUDE.md`).
- **Host matching stays simple:** exact host or one leading wildcard label
  (`*.example.com`), never regex.
- **No upward edges:** the `ironclaw_network` `BoundaryRule` in
  `reborn_dependency_boundaries.rs` (`reborn_crate_dependency_boundaries_hold`)
  forbids runtime/workflow/secret/filesystem/resource/event/approval/
  authorization crates by name.
- **Runtime crates cannot bypass it:**
  `reborn_runtime_http_egress_has_single_network_boundary` scans the lane and
  kernel `src/` trees for `reqwest::Client`, ad-hoc DNS, and revived SSRF
  helpers.

## Tests

```bash
cargo test -p ironclaw_network
cargo test -p ironclaw_architecture_tests   # boundary + single-egress gates
```

## See also

Working rules: [`CLAUDE.md`](./CLAUDE.md) (canonical crate guardrails). Family
boundary: [`crates/substrates/AGENTS.md`](../AGENTS.md). Contracts:
`docs/reborn/contracts/network.md`, `docs/reborn/contracts/kernel-boundary.md`,
`docs/reborn/contracts/resources.md`.
