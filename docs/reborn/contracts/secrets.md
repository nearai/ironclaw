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
CredentialSlotId
CredentialAccountId
CredentialSecretRef
CredentialAccountRecord
CredentialAccountRepository
InMemoryCredentialAccountRepository
FilesystemCredentialAccountRepository
```

`SecretMaterial` is backed by `secrecy::SecretString`, so access to raw values is explicit through `ExposeSecret`. Metadata, lease records, credential mappings, encrypted records, repository errors, and debug output never contain raw secret values.

Ownership remains:

```text
host_api       -> opaque SecretHandle and Action::UseSecret shapes
filesystem     -> virtual-path storage abstraction plus PostgreSQL/libSQL/local backend implementations
secrets        -> scoped storage, AES-256-GCM/HKDF encryption, metadata, one-shot leases, encrypted-row repository boundary, filesystem-backed encrypted persistence, and credential mapping metadata
authorization  -> whether a caller may use a SecretHandle
capabilities   -> caller-facing workflow; calls host-provided obligation handlers before dispatch/process/approval-lease side effects
host_runtime   -> direct-handle InjectSecretOnce lease/consume composition plus already-resolved credential material in hardened runtime egress
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

All operations receive a `ResourceScope`. Implementations key secrets and leases by tenant/user/agent/project plus `SecretHandle` or `SecretLeaseId`.

Rules:

- no global handle lookup
- the same `SecretHandle` in another tenant/user/agent/project is a distinct secret
- local/single-user deployments should use concrete defaults: tenant `default`, agent `default`, and project `bootstrap`
- `_none` means intentionally absent/shared optional scope; it is not the default local agent or default local project
- cross-scope lease consumption returns `UnknownLease`
- missing secrets return `UnknownSecret` and do not create leases
- consumed leases cannot be consumed again
- revoked leases cannot be consumed
- metadata includes usage counters and timestamps, but never raw material

This is the minimum shape used by the direct-handle `InjectSecretOnce` obligation path.

---

## 5. Filesystem-backed repository

`FilesystemEncryptedSecretRepository<F>` implements `EncryptedSecretRepository` for any `F: RootFilesystem`. It stores redacted JSON records under tenant/user/agent/project-scoped virtual paths:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/projects/{project_id-or-_none}/secrets/{handle}.json
```

For a single local user with one default agent and no selected project, use concrete defaults rather than `_none` for tenant/agent/project identity:

```text
/engine/tenants/default/users/alice/agents/default/projects/bootstrap/secrets/gmail.work.refresh_token.json
```

Use `agents/_none` or `projects/_none` only for intentionally unscoped/shared records.

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

## 7. Credential slots and accounts

Extensions often need a credential *kind* rather than one hard-coded secret. For example, a Gmail extension may need a Google OAuth credential while a user has personal, work, and client Gmail accounts. Reborn models this as metadata-only credential accounts:

```text
Extension declares or implies a credential slot:
  extension_id = gmail
  slot_id      = google_oauth

User stores multiple scoped accounts for that slot:
  account_id   = personal
  label        = Personal Gmail
  subject_hint = me@gmail.com
  secret_refs  = refresh_token -> SecretHandle("gmail.personal.refresh_token")

  account_id   = work
  label        = Work Gmail
  subject_hint = me@company.com
  secret_refs  = refresh_token -> SecretHandle("gmail.work.refresh_token")
```

`ironclaw_secrets` owns only the account metadata and secret-handle references:

```rust
pub struct CredentialAccountRecord {
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub slot_id: CredentialSlotId,
    pub account_id: CredentialAccountId,
    pub label: String,
    pub subject_hint: Option<String>,
    pub secret_refs: Vec<CredentialSecretRef>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

A credential account is **not** an authority grant. It does not contain raw material and does not bypass `SecretStore` lease consumption. Runtime composition must still authorize the capability, resolve the selected credential account, check that every referenced `SecretHandle` is allowed for the invocation, call `lease_once(scope, handle)`, consume each lease exactly once, and inject material only inside the approved runtime/provider path.

Boundary rules:

- `CredentialAccountRepository` is keyed by tenant/user/agent/project + extension + slot + account.
- The same extension and slot may have many accounts under the same scope.
- Account labels and subject hints are UI metadata only; they are not secret material and not authorization decisions.
- `CredentialSecretRef` contains a reference name and `SecretHandle` only, never plaintext, OAuth access tokens, refresh tokens, cookies, or API keys.
- Account selection/defaults are product workflow or settings concerns. The secrets crate may persist account records, but it does not choose an account for a prompt, remember defaults, ask users, or resolve policy.
- OAuth refresh/repair, provider HTTP calls, token exchange, and account verification must go through host/runtime composition and `ironclaw_network`; they do not belong in `ironclaw_secrets`.
- Auditing account use belongs to host/control-plane event sinks. The secrets crate must not emit events or depend on `ironclaw_events`.

Filesystem-backed credential account metadata uses redacted JSON under:

```text
/engine/tenants/{tenant_id}/users/{user_id}/agents/{agent_id-or-_none}/projects/{project_id-or-_none}/credential-accounts/{extension_id}/{slot_id}/{account_id}.json
```

For a single local user with three Gmail accounts, this becomes:

```text
/engine/tenants/default/users/alice/agents/default/projects/bootstrap/credential-accounts/gmail/google_oauth/personal.json
/engine/tenants/default/users/alice/agents/default/projects/bootstrap/credential-accounts/gmail/google_oauth/work.json
/engine/tenants/default/users/alice/agents/default/projects/bootstrap/credential-accounts/gmail/google_oauth/client.json
```

This path stores only labels, subject hints, secret handles, and timestamps. Secret material remains exclusively in encrypted secret records and is only exposed by `SecretStore::consume(...)` after an explicit scoped lease.

V1 direct-handle obligation handling uses `InjectSecretOnce { handle }`. Future obligation handling may add a richer host-api obligation such as:

```rust
InjectCredentialOnce {
    extension_id: ExtensionId,
    slot_id: CredentialSlotId,
    account_id: CredentialAccountId,
}
```

for credential-account lookup. For now, account metadata remains support data and does not grant authority; callers that choose a credential account must resolve its secret handle(s) before authorizing direct `InjectSecretOnce` obligations.

---

## 8. Non-goals

This slice does not implement:

- platform keychain master-key resolution/persistence
- automatic master-key generation or `.env` fallback wiring
- secret rotation/versioning
- secret audit emission
- authorization policy for secret use
- approval prompts for secret use
- account selection UI, account defaults, or per-project remembered choices
- production `InjectCredentialOnce` obligation handling
- generic runtime environment/request injection beyond the one-shot direct secret injection staging store
- OAuth/token refresh flows
- provider account verification
- network policy enforcement

Those should be added as separate service/composition slices without moving runtime or product workflow semantics into this crate. Concrete durable adapters must preserve the same tenant/user/agent/project keying and sanitized error behavior.

---

## 9. Contract tests

The crate tests cover:

- metadata returns no raw secret material
- one-shot leases consume exactly once
- same-handle secrets are tenant/user/agent/project isolated
- missing secrets fail without creating leases
- credential mapping constructors carry handles and host patterns but no secret material
- credential account records allow multiple accounts for the same extension slot without storing material
- credential account repositories isolate tenant/user/agent/project scopes and list only selected extension-slot accounts
- filesystem-backed credential account records persist redacted metadata and survive new repository instances
- encrypted store persists ciphertext rather than plaintext
- identical plaintext uses distinct salts/ciphertext
- a new store instance can read through the same encrypted repository with the same master key
- wrong master keys fail closed without consuming leases
- successful consume records usage metadata
- filesystem-backed repository stores encrypted JSON without plaintext
- filesystem-backed repository survives new store instances over the same root filesystem
- filesystem-backed repository isolates tenant/user/agent/project scopes and lists only visible records
- filesystem-backed repository ignores unrelated engine JSON when scanning for active secret records
- filesystem-backed repository records usage and tombstones deletes without plaintext
- filesystem-backed repository has feature-gated type contracts for libSQL and PostgreSQL `RootFilesystem` backends without direct SQL adapters in this crate
- crate boundary remains low-level and does not depend on workflow/runtime/observability crates


---

## Contract freeze addendum — production source of truth (2026-04-25)

Production secrets use a typed encrypted secret repository as source of truth.

`FilesystemEncryptedSecretRepository` remains a verified reference/projection/backend experiment, but the production contract is structured encrypted records with scoped lease/usage metadata. Generic `/secrets` file listing must not expose source secret records.

V1 must implement `InjectSecretOnce` obligation handling through explicit secret lease consumption:

```text
CapabilityHost obligation handler
  -> authorize/use secret handle
  -> lease_once(scope, handle)
  -> consume exactly once
  -> inject into approved runtime/provider location
  -> redact output/events/audit
```

Master-key/keychain resolution is required before production secret injection is considered complete.
