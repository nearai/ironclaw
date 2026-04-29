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

The crate owns storage mechanics and one-shot lease state. It does not decide authorization, run approval flows, inject secrets into runtime requests, contact networks, emit audit events, or execute product workflows. It only provides the lease/consume primitive; runtime injection is not enforced until an obligation-handler/runtime composition slice wires it in.

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
capabilities   -> caller-facing workflow; currently fails closed on InjectSecretOnce obligations
host_runtime   -> future composition of secret services into runtime adapters/obligation handlers
runtimes        -> consume injected values only after host-side authorization and lease handling
```

---

## 3. Scope and isolation

All operations receive a `ResourceScope`. The in-memory V1 implementation keys stored secrets by tenant/user/agent/project plus `SecretHandle`, and keys one-shot leases by the full resource scope plus `SecretLeaseId`.

Rules:

- no global handle lookup
- the same `SecretHandle` in another tenant/user/agent/project is a distinct secret
- cross-scope lease consumption returns `UnknownLease`
- missing secrets return `UnknownSecret` and do not create leases
- consumed leases cannot be consumed again
- revoked leases cannot be consumed
- consumed and revoked lease tombstones drop retained cloned secret material

This is the minimum shape needed before wiring secret injection into obligation handling.

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

---

## 5. Non-goals

This slice does not implement:

- durable encrypted secret persistence
- platform keychain integration
- secret rotation/versioning
- secret audit emission
- authorization policy for secret use
- approval prompts for secret use
- runtime environment/request injection
- OAuth/token refresh flows
- network policy enforcement

Those should be added as separate service/composition slices without moving runtime or product workflow semantics into this crate.

---

## 6. Contract tests

The crate tests cover:

- metadata returns no raw secret material
- one-shot leases consume exactly once
- same-handle secrets are tenant/user/agent/project isolated
- revoked leases cannot be consumed
- missing secrets fail without creating leases
- consumed and revoked lease records do not retain cloned secret material
- crate boundary remains low-level and does not depend on workflow/runtime/observability crates
