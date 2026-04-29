# IronClaw Reborn network service contract

**Date:** 2026-04-26
**Status:** V1 service-boundary slice
**Crate:** `crates/ironclaw_network`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_network` is the scoped network policy evaluation service for Reborn.

It turns a host API `NetworkPolicy` plus a scoped target request into a metadata-only permit:

```text
NetworkRequest { ResourceScope, NetworkTarget, NetworkMethod, estimated_bytes }
  -> NetworkPolicyEnforcer::authorize(...)
  -> NetworkPermit or NetworkPolicyError
```

This crate does not perform HTTP I/O, DNS resolution, proxying, credential injection, resource reservation, audit emission, or product workflow. It is the boundary that future runtime/network adapters can call before they perform network effects. It only evaluates metadata policy; actual egress enforcement, DNS/redirect revalidation, and response limiting belong to the runtime/network execution boundary.

---

## 2. Boundary

The public contract is intentionally small:

```rust
NetworkRequest
NetworkPermit
NetworkPolicyError
NetworkPolicyEnforcer
StaticNetworkPolicyEnforcer
network_policy_allows(...)
target_matches_pattern(...)
host_matches_pattern(...)
is_private_or_loopback_ip(...)
```

Ownership remains:

```text
host_api       -> NetworkPolicy, NetworkTarget, NetworkMethod shapes
network        -> scoped policy evaluation and metadata-only permits
authorization  -> whether a caller has a grant with network authority
capabilities   -> caller-facing workflow; currently fails closed on ApplyNetworkPolicy obligations
host_runtime   -> future composition of network enforcers into runtime adapters/obligation handlers
runtimes        -> perform I/O only after host-side authorization and network permit handling
```

---

## 3. Policy semantics

V1 semantics intentionally mirror the current WASM network import policy checks so they can later be centralized:

- empty `allowed_targets` fails closed
- `NetworkTargetPattern.scheme` must match when present
- `NetworkTargetPattern.port` must match when present
- `host_pattern` is exact host or one leading wildcard label such as `*.github.com`
- wildcard patterns do not match the apex host itself or deeper multi-label subdomains
- `deny_private_ip_ranges` blocks literal private, loopback, link-local, documentation, broadcast, multicast, unspecified, carrier-grade NAT, IPv4-mapped IPv6 private ranges, and unique-local IP targets
- `max_egress_bytes` denies requests without an estimated byte count and requests whose estimated bytes exceed the configured limit

DNS/IP resolution safeguards against rebinding remain future work for the actual network execution/proxy boundary. This crate currently checks literal IP targets only.

---

## 4. Current API flow

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

---

## 5. Non-goals

This slice does not implement:

- HTTP client/proxy execution
- DNS resolution or DNS-rebinding protection
- resource reservation for network egress
- credential or secret injection
- durable audit/event emission
- per-method policy matrices
- per-tenant persisted policy stores
- response body limiting or streaming
- OAuth/token refresh flows

Those should be added as separate service/composition slices without moving runtime execution or product workflow semantics into this crate.

---

## 6. Contract tests

The crate tests cover:

- exact scheme/host/port allow path
- one-label wildcard host matching
- wildcard apex and nested-subdomain denial
- scheme/host/port mismatch denial
- estimated egress requirement and limit denial
- literal non-public IP denial, including IPv4-mapped IPv6 private literals
- fail-closed empty policy behavior
- crate boundary remains low-level and does not depend on workflow/runtime/secret/observability crates
