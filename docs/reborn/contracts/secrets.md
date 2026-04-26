# IronClaw Reborn secrets service contract

**Date:** 2026-04-26
**Status:** V1 encrypted store + filesystem-backed durability + credential mapping slice
**Crate:** `crates/ironclaw_secrets`
**Depends on:** `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

`ironclaw_secrets` is the scoped secret metadata, encrypted storage, and lease service for Reborn.

It turns opaque host API handles into explicit, short-lived access leases:

```text
ResourceScope + SecretHandle
  -> SecretStore::lease_once(...)
  -> SecretLease
  -> SecretStore::consume(...)
  -> SecretMaterial exactly once
```

The crate owns scoped storage mechanics, AES-256-GCM/HKDF encryption, encrypted-row repository contracts, filesystem-backed durable persistence over `RootFilesystem`, one-shot lease state, redacted metadata, and credential mapping shapes. It does not decide authorization, run approval flows, inject secrets into runtime requests, contact networks, emit audit events, or execute product workflows.

---

## 2. Boundary

The public contract includes:

```rust
SecretMaterial
SecretId
SecretMetadata
SecretLeaseId
SecretLeaseStatus
SecretLease
SecretStoreError
SecretStore
InMemorySecretStore
SecretsCrypto
EncryptedSecretRecord
EncryptedSecretRepository
EncryptedSecretStore
InMemoryEncryptedSecretRepository
FilesystemEncryptedSecretRepository
CredentialLocation
CredentialMapping
```

`SecretMaterial` is backed by `secrecy::SecretString`, so access to raw values is explicit through `ExposeSecret`. Metadata, lease records, credential mappings, encrypted records, repository errors, and debug output never contain raw secret values.

Ownership remains:

```text
host_api       -> opaque SecretHandle and Action::UseSecret shapes
filesystem     -> virtual-path storage abstraction plus PostgreSQL/libSQL/local backend implementations
secrets        -> scoped storage, AES-256-GCM/HKDF encryption, metadata, one-shot leases, encrypted-row repository boundary, filesystem-backed encrypted persistence, and credential mapping metadata
authorization  -> whether a caller may use a SecretHandle
capabilities   -> caller-facing workflow; currently fails closed on InjectSecretOnce obligations
host_runtime   -> composition of already-resolved credential material into hardened runtime egress; future secret lease consumption in obligation handlers
runtimes        -> consume injected values only after host-side authorization and lease handling
```

`ironclaw_secrets` intentionally stays independent from workflow/runtime/event/authorization/process crates and from concrete runtime crates. Durable secret persistence is implemented through the Reborn `RootFilesystem` abstraction, so PostgreSQL/libSQL durability comes from filesystem backend composition rather than direct SQL adapters in this crate.

---

## 3. Encryption model

The Reborn crypto port follows the existing production secrets design:

```text
master key (SecretMaterial)
  + per-secret random salt
  -> HKDF-SHA256 derived key
  -> AES-256-GCM encrypted value
```

Rules:

- master keys must be at least 32 bytes
- every stored secret gets a new 32-byte random salt
- encrypted rows store `nonce || ciphertext || tag` plus the salt
- identical plaintext values produce different ciphertexts because salts/nonces differ
- tampered ciphertext, wrong master keys, and invalid UTF-8 fail closed with sanitized `SecretStoreError` variants
- `SecretsCrypto` and `EncryptedSecretRecord` debug output redacts master keys, ciphertext, and salts

`EncryptedSecretStore<R>` implements `SecretStore` over an `EncryptedSecretRepository`. It encrypts on `put`, decrypts only after a scoped one-shot lease is consumed, records usage after successful decrypt, and leaves a lease active if decryption fails.

---

## 4. Scope and isolation

All operations receive a `ResourceScope`. Implementations key secrets and leases by tenant/user/project plus `SecretHandle` or `SecretLeaseId`.

Rules:

- no global handle lookup
- the same `SecretHandle` in another tenant/user/project is a distinct secret
- cross-scope lease consumption returns `UnknownLease`
- missing secrets return `UnknownSecret` and do not create leases
- consumed leases cannot be consumed again
- revoked leases cannot be consumed
- metadata includes usage counters and timestamps, but never raw material

This is the minimum shape needed before wiring secret lease consumption into obligation handling.

---

## 5. Filesystem-backed repository

`FilesystemEncryptedSecretRepository<F>` implements `EncryptedSecretRepository` for any `F: RootFilesystem`. It stores redacted JSON records under tenant/user/project-scoped virtual paths:

```text
/engine/tenants/{tenant_id}/users/{user_id}/projects/{project_id-or-_none}/secrets/{handle}.json
```

Repository records contain only metadata, ciphertext, salt, and a small tombstone flag used for delete semantics on filesystems that do not expose physical deletion yet. `list`, `get`, `record_usage`, `delete`, and `any_exist` operate through `RootFilesystem` only. There are no direct PostgreSQL/libSQL dependencies in the repository; DB durability is supplied by composing this repository with `PostgresRootFilesystem` or `LibSqlRootFilesystem` from `ironclaw_filesystem`.

## 6. Current API flow

```rust
let root_filesystem = Arc::new(/* LocalFilesystem, LibSqlRootFilesystem, or PostgresRootFilesystem */);
let repository = Arc::new(FilesystemEncryptedSecretRepository::new(root_filesystem));
let crypto = SecretsCrypto::new(SecretMaterial::from(master_key))?;
let secrets = EncryptedSecretStore::new(repository.clone(), crypto);

let metadata = secrets
    .put(scope.clone(), handle.clone(), SecretMaterial::from("token"))
    .await?;

let encrypted_record = repository.get(&scope, &handle).await?;
let lease = secrets.lease_once(&scope, &handle).await?;
let material = secrets.consume(&scope, lease.id).await?;
```

`metadata`, `lease`, `CredentialMapping`, and `EncryptedSecretRecord` are safe to log only as metadata/redacted structs; they do not include secret values. `material` is the only raw-value carrier and should stay inside the narrow injection path that requested it.

Credential mappings describe where an already-authorized secret should be placed, without carrying material:

```rust
let mapping = CredentialMapping::bearer(
    SecretHandle::new("github_token")?,
    "api.github.com",
);
```

Runtime composition must obtain material through explicit scoped secret access before creating an injection-time value such as `RuntimeHttpCredential`; this crate does not inject into requests itself.

---

## 7. Non-goals

This slice does not implement:

- platform keychain master-key resolution/persistence
- automatic master-key generation or `.env` fallback wiring
- secret rotation/versioning
- secret audit emission
- authorization policy for secret use
- approval prompts for secret use
- production `InjectSecretOnce` obligation handling
- runtime environment/request injection
- OAuth/token refresh flows
- network policy enforcement

Those should be added as separate service/composition slices without moving runtime or product workflow semantics into this crate. Concrete durable adapters must preserve the same tenant/user/project keying and sanitized error behavior.

---

## 8. Contract tests

The crate tests cover:

- metadata returns no raw secret material
- one-shot leases consume exactly once
- same-handle secrets are tenant/user/project isolated
- missing secrets fail without creating leases
- credential mapping constructors carry handles and host patterns but no secret material
- encrypted store persists ciphertext rather than plaintext
- identical plaintext uses distinct salts/ciphertext
- a new store instance can read through the same encrypted repository with the same master key
- wrong master keys fail closed without consuming leases
- successful consume records usage metadata
- filesystem-backed repository stores encrypted JSON without plaintext
- filesystem-backed repository survives new store instances over the same root filesystem
- filesystem-backed repository isolates tenant/user/project scopes and lists only visible records
- filesystem-backed repository ignores unrelated engine JSON when scanning for active secret records
- filesystem-backed repository records usage and tombstones deletes without plaintext
- filesystem-backed repository has feature-gated type contracts for libSQL and PostgreSQL `RootFilesystem` backends without direct SQL adapters in this crate
- crate boundary remains low-level and does not depend on workflow/runtime/observability crates
