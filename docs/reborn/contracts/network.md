# IronClaw Reborn network service contract

**Date:** 2026-04-26
**Status:** V1 policy + hardened HTTP egress slice
**Crate:** `crates/ironclaw_network`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_network` is the scoped network policy and hardened HTTP egress service for Reborn.

It still turns a host API `NetworkPolicy` plus a scoped target request into a metadata-only permit:

```text
NetworkRequest { ResourceScope, NetworkTarget, NetworkMethod, estimated_bytes }
  -> NetworkPolicyEnforcer::authorize(...)
  -> NetworkPermit or NetworkPolicyError
```

It also provides a low-level `HardenedHttpEgressClient` for runtime-owned HTTP effects. That client performs HTTP I/O only after policy checks, DNS resolution, private-address rejection, redirect re-validation, timeout bounds, and response-size limits. It does not inject credentials, reserve resources, emit audit/events, or own product workflow.

---

## 2. Boundary

The public contract is intentionally small:

```rust
NetworkRequest
NetworkPermit
NetworkPolicyError
NetworkPolicyEnforcer
StaticNetworkPolicyEnforcer
HttpEgressRequest
HttpEgressResponse
HttpEgressError
HttpEgressClient
HardenedHttpEgressClient
network_policy_allows(...)
target_matches_pattern(...)
host_matches_pattern(...)
is_private_or_loopback_ip(...)
```

Ownership remains:

```text
host_api       -> NetworkPolicy, NetworkTarget, NetworkMethod shapes
network        -> scoped policy evaluation, metadata-only permits, and hardened HTTP egress
authorization  -> whether a caller has a grant with network authority
capabilities   -> caller-facing workflow and fail-closed obligation handler seam
host_runtime   -> ApplyNetworkPolicy obligation preflight and WASM host-HTTP policy/egress handoff
runtimes        -> perform I/O only after host-side authorization and network policy/permit/egress handling
```

---

## 3. Policy semantics

V1 policy semantics are centralized here for both metadata-only permits and hardened HTTP egress:

- empty `allowed_targets` fails closed
- `NetworkTargetPattern.scheme` must match when present
- `NetworkTargetPattern.port` must match when present
- `host_pattern` is exact host or one leading wildcard label such as `*.github.com`
- wildcard patterns do not match the apex host itself
- `deny_private_ip_ranges` blocks literal private, loopback, link-local, documentation, broadcast, multicast, unspecified, carrier-grade NAT, cloud metadata, IPv4-mapped IPv6, and unique-local IP targets
- `max_egress_bytes` denies requests whose estimated bytes exceed the configured limit

For `HardenedHttpEgressClient`, hostnames are resolved before connection and every resolved address is checked when `deny_private_ip_ranges` is enabled. The validated resolution is pinned into `reqwest` with redirects disabled. Simple GET redirects are followed only after each `Location` target is parsed, policy-checked, DNS-checked, and re-pinned. Non-simple redirects fail closed.

---

## 4. Current API flow

Metadata-only policy checks still use `NetworkPolicyEnforcer`:

```rust
let enforcer = StaticNetworkPolicyEnforcer::new(policy);
let permit = enforcer
    .authorize(NetworkRequest {
        scope,
        target,
        method: NetworkMethod::Post,
        estimated_bytes: Some(512),
    })
    .await?;
```

`NetworkPermit` carries only metadata needed by a runtime adapter to proceed. It does not hold sockets, HTTP clients, response bodies, secrets, raw host paths, or resource reservations.

Runtime-owned HTTP execution uses the hardened egress client:

```rust
let client = HardenedHttpEgressClient::new();
let response = client.request(HttpEgressRequest {
    scope,
    policy,
    method: NetworkMethod::Get,
    url: "https://api.example.test/v1".to_string(),
    headers: Vec::new(),
    body: Vec::new(),
    timeout: None,
    max_response_bytes: Some(1024 * 1024),
})?;
```

`HttpEgressError` variants are stable and sanitized; they do not expose raw backend error strings, response bodies, secret material, or host paths.

---

## 5. Non-goals

This slice does not implement:

- resource reservation for network egress
- runtime enforcement of `ApplyNetworkPolicy` for non-WASM lanes
- credential or secret injection
- durable audit/event emission
- per-method policy matrices
- per-tenant persisted policy stores
- OAuth/token refresh flows

The hardened HTTP client is not a general agent-facing `http` tool. It intentionally omits product-layer approval prompts, trace recording, credential injection, request/response leak scanning, and OAuth repair flows; those stay in the caller/tool/composition layers.

Those should be added as separate service/composition slices without moving runtime execution or product workflow semantics into this crate.

---

## 6. Contract tests

The crate tests cover:

- exact scheme/host/port allow path
- one-label wildcard host matching
- wildcard apex denial
- scheme/host/port mismatch denial
- estimated egress limit denial
- literal private/loopback/link-local IP denial
- hardened HTTP execution for allowed targets
- DNS-resolved private target denial before connect
- redirect target re-validation before follow
- response-size cap enforcement while reading
- fail-closed empty policy behavior
- crate boundary remains low-level and does not depend on workflow/runtime/secret/observability crates


---

## Contract freeze addendum — provider HTTP boundary (2026-04-25)

All host-side and provider-side HTTP in V1 must go through `ironclaw_network` or an adapter explicitly built on top of its policy/hardening primitives.

This includes:

```text
embedding providers
external memory adapters such as Mem0/Letta/Zep
OAuth/token repair clients
first-party provider SDK wrappers
host-mediated MCP/provider HTTP calls
runtime HTTP egress
```

A provider crate must not instantiate an unrestricted HTTP client that bypasses scoped network policy, DNS/private-address checks, redirect revalidation, response-size limits, credential leak scanning, or sanitized error handling.

Script and MCP are first-class V1 runtime lanes. Their network behavior must therefore be enforceable through `ironclaw_network`; otherwise network-capable Script/MCP operations fail closed.
