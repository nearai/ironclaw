# IronClaw Reborn secrets service contract

**Date:** 2026-04-26
**Status:** V1 service-boundary slice
**Crate:** `crates/ironclaw_secrets`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_secrets` is the scoped secret metadata and lease service for Reborn.

It turns opaque host API handles into explicit, short-lived access leases:

```text
ResourceScope + SecretHandle
  -> SecretStore::lease_once(...)
  -> SecretLease
  -> SecretStore::consume(...)
  -> SecretMaterial exactly once
```

The crate owns storage mechanics and one-shot lease state. It does not decide authorization, run approval flows, contact networks, emit audit events, or execute product workflows. It only provides the metadata and lease/consume primitive; host-runtime composition owns any concrete injection into runtime requests.

---

## 2. Boundary

The public contract is intentionally small:

```rust
SecretMaterial
SecretMetadata
SecretLeaseId
SecretLeaseStatus
SecretLease
SecretStoreError
SecretStore
InMemorySecretStore
```

`SecretMaterial` is backed by `secrecy::SecretString`, so access to raw values is explicit through `ExposeSecret`. Metadata, lease records, and errors never contain raw values.

Ownership remains:

```text
host_api       -> opaque SecretHandle and Action::UseSecret shapes
secrets        -> scoped storage, metadata, and one-shot leases
authorization  -> whether a caller may use a SecretHandle
capabilities   -> caller-facing workflow; fails closed on InjectSecretOnce unless an obligation handler is configured
host_runtime   -> built-in obligation handler leases/stages one-shot secret material and shared runtime HTTP egress injects/redacts secrets for host-mediated requests
runtimes        -> consume injected values only after host-side authorization and lease handling
```

---

## 3. Scope and isolation

All operations receive a `ResourceScope`. The in-memory V1 implementation keys secrets by tenant/user/agent/project plus `SecretHandle`; leases are scoped by the full invocation context plus `SecretLeaseId`.

Rules:

- no global handle lookup
- the same `SecretHandle` in another tenant/user/agent/project is a distinct secret
- cross-scope lease consumption returns `UnknownLease`
- missing secrets return `UnknownSecret` and do not create leases
- consumed leases cannot be consumed again
- revoked leases cannot be consumed

This is the minimum shape needed for host-runtime composition to wire secret injection into obligation handling without exposing raw values to runtime crates.

---

## 4. Current API flow

```rust
let metadata = secrets
    .put(scope.clone(), handle.clone(), SecretMaterial::from("token"))
    .await?;

let lease = secrets.lease_once(&scope, &handle).await?;
let material = secrets.consume(&scope, lease.id).await?;
```

`metadata` and `lease` are safe to log only as metadata; they do not include secret values. `material` is the only raw-value carrier and should stay inside the narrow injection path that requested it.

`SecretStore::put(...)` is for trusted setup, composition, migration, or storage-code paths that are already allowed to manage secret material. It is not a runtime/plugin API, and it intentionally does not perform authorization itself.

The shared Reborn runtime HTTP egress service uses this surface to:

- check metadata for required or optional credential handles
- create one-shot leases scoped to the request
- consume the lease exactly once inside the host process
- reject runtime-supplied sensitive headers, auth-like headers, credential query parameters, credential-shaped request content, and credential-shaped raw or percent-decoded URL content before network dispatch
- inject material into the outgoing request shape
- scrub leased values from runtime-visible network errors and response headers/bodies
- strip sensitive response headers and block credential-shaped response bodies before they reach runtime callers

Runtime HTTP credential injection is authority-bearing and must be host-derived.
`RuntimeCredentialInjection` is not a permission request supplied by guest code,
runtime code, or an extension process. The upstream capability/obligation owner
must derive it only after proving:

- the extension or capability declared the secret handle
- the caller is authorized or approved to use the secret
- the destination URL matches the capability or secret destination policy
- the injection target and prefix are host-approved
- the final request still passes the network policy boundary

The shared egress service intentionally does not perform that authorization
decision; it consumes the already-approved injection plan, injects it, redacts
it, and fails closed when a required credential is unavailable. Injection plans
also declare a material source: `SecretStoreLease` leases and consumes directly
from `SecretStore`, while `StagedObligation { capability_id }` consumes material
that `BuiltinObligationHandler` already leased, consumed, and staged in
`RuntimeSecretInjectionStore`. Runtime adapters that use the staged source must
not lease the same handle independently; `HostHttpEgressService` removes staged
material with `take(scope, capability_id, handle)` before outbound transport so
the value cannot be reused after success, failure, or runtime-visible errors.
Staged entries also expire after the store TTL (five minutes by default) and
expired material is pruned during insertion, `take(...)`, and explicit
`prune_expired(...)` calls. If one approved request plan injects the same
source+handle into multiple targets, the egress service consumes or leases the
material once and reuses it only within that request. Runtime callers must not
supply their own `Authorization`, cookie, or API-key-style headers; those values
must come from the host-approved injection plan. WASM host-mediated HTTP
composition can construct staged plans with `WasmStagedRuntimeCredentials` after
attaching the invoking capability id to the adapter; exact-url rules should be
preferred when a credential is only valid for specific destinations.

---

## 5. Non-goals

This slice does not implement:

- durable encrypted secret persistence
- platform keychain integration
- secret rotation/versioning
- secret audit emission
- authorization policy for secret use
- approval prompts for secret use
- direct runtime environment/request injection from this crate
- OAuth/token refresh flows
- network policy enforcement

Those should be added as separate service/composition slices without moving runtime or product workflow semantics into this crate.

---

## 6. Contract tests

The crate tests cover:

- metadata returns no raw secret material
- one-shot leases consume exactly once
- same-handle secrets are tenant/user/agent/project isolated
- consumed and revoked lease records drop retained secret material
- revoked leases cannot be consumed
- missing secrets fail without creating leases
- crate boundary remains low-level and does not depend on workflow/runtime/observability crates
