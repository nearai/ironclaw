# ironclaw_secrets

Scoped, encrypted secret custody: secret metadata and storage over the
filesystem fabric, one-shot leases, the credential broker built on them, the
cryptography (authenticated encryption, AAD derivation, master-key validation),
and the operating-system keychain integration that protects the master key.
The invariant that raw material is readable exactly once per lease is this
crate's entire reason to exist, and keeping the crypto/keychain dependency cone
out of every other crate is why it is a separate crate.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_secrets` · **Manifest:**
  `crates/substrates/ironclaw_secrets/Cargo.toml`
- **Use this when:** secret material needs storing, leasing, or brokering —
  and you are the auth engine, the kernel's staging path, or
  composition wiring them.
- **Don't use this when:** you need a secret *injected* into a runtime call →
  that is the kernel's obligation handling (`ironclaw_host_runtime` staging),
  not a direct store read; you're products-tier → go through the
  `ironclaw_product_contracts::operator_secrets` port (deliberately narrower
  than this crate's shape); you're detecting secrets in text →
  `ironclaw_safety`.

## Public surface

- `SecretStore` trait — `lease_once`/`consume`, the CAS one-shot primitive
  (raw material readable exactly once per lease); `SecretMetadata`,
  `SecretLease`/`SecretLeaseId`/`SecretLeaseStatus`, `SecretStoreError`,
  `DecryptedSecret`, `SecretMaterial` (a `secrecy::SecretString` re-export).
  Volatile construction via `SecretStore::ephemeral()`.
- Credential broker: `CredentialAccount`/`CredentialSession` (+ stores
  `CredentialAccountStore`/`CredentialSessionStore`),
  `CredentialTargetPolicy`/`CredentialPathPolicy`, `InMemoryCredentialBroker`,
  `CredentialBrokerError`, `RedactedJson`.
- Crypto: `SecretsCrypto` and the AAD constructors (`crypto`); OS keychain
  master-key integration (`keychain`).

One production implementation of each store, built on the filesystem fabric.

## Depends on / consumed by

- **Depends on (workspace):** `ironclaw_filesystem` (custody over the fabric —
  a charted same-layer edge), `ironclaw_host_api`. External: `aes-gcm`,
  `hkdf`, `sha2`, `subtle`, `secrecy`, `secret-service`/`security-framework`
  (keychains), among others — the cone this crate quarantines.
- **Consumed by (measured 2026-08-05):** 8 normal — `ironclaw_auth` (the
  chartered direct consumer: it owns token-custody flows), `ironclaw_host_runtime`
  (kernel staging), `ironclaw_composition` (constructs), plus
  `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`
  (tracked in #7095), `ironclaw_sandbox`, `ironclaw_stress`. The target set is
  smaller (PROPOSAL §6.2.2 removed the `webui`/`operator` edges already); the
  surplus is standing narrowing work, not permission to add more.

## Invariants

- **One-shot consumption:** a secret's raw material is readable exactly once
  per lease — `SecretStore::consume` after an explicit scoped lease.
- **No raw material in any output:** metadata, errors, debug, events,
  snapshots, and docs never carry secret values.
- **`put` and atomic `put_if_absent` are trusted primitives** for
  setup/composition/storage code, not runtime/plugin APIs.
- **Isolation is scoped:** tenant/user/agent/project isolation is preserved;
  no global handle lookup unless an explicit admin-scoped API is introduced
  later.
- **Custody only:** no authorization, approval, run-state, runtime injection,
  network access, process lifecycle, or product workflow semantics here.
- **Upward edges forbidden by name:** the `ironclaw_secrets` `BoundaryRule` in
  `reborn_dependency_boundaries.rs` (`reborn_crate_dependency_boundaries_hold`);
  the `secrets → filesystem` same-layer edge is inventoried in
  `reborn_same_layer_edge_inventory.rs`.

## Tests

```bash
cargo test -p ironclaw_secrets
cargo test -p ironclaw_architecture_tests   # boundary rules
```

## See also

Family boundary: [`crates/substrates/AGENTS.md`](../AGENTS.md). Contracts:
`docs/reborn/contracts/secrets.md`,
`docs/reborn/contracts/storage-placement.md`,
`docs/reborn/contracts/kernel-boundary.md`. (The crate's working rules were
folded into the Invariants above on 2026-08-06; this README is the crate-local
home.)
