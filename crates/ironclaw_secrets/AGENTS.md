# Agent Map — ironclaw_secrets

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/secrets.md`
- `docs/reborn/contracts/storage-placement.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Scoped secret storage and credential brokering, currently:
- Secret metadata, leases, and one-shot consumption: `SecretMetadata`, `SecretLease`/`SecretLeaseId`/`SecretLeaseStatus`, the `SecretStore` trait + `ScopedSecretsStoreAdapter`/`InMemorySecretStore`, the legacy `SecretsStore` (`consume`, `CreateSecretParams`, `SecretConsumeResult`), and `SecretStoreError`/`SecretError`.
- Credential-broker subsystem: `CredentialAccount`/`CredentialSession` (+ IDs and `CredentialAccountStatus`), `CredentialTargetPolicy`/`CredentialPathPolicy`, `CredentialAccountStore`/`CredentialSessionStore` traits, `InMemoryCredentialBroker`, `CredentialSessionRequest`, `CredentialBrokerError`, `RedactedJson`.
- Encryption helpers (`crypto`): `SecretsCrypto` and the AAD constructors (`secret_record_aad`, `filesystem_secret_aad`, `credential_account_aad`, `credential_session_aad`).
- Filesystem-backed stores: `FilesystemSecretStore`, `FilesystemCredentialBroker`; and the `SecretMaterial` (`secrecy::SecretString`) re-export.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- raw secret material in errors/events/debug/snapshots/docs or provider HTTP/injection beyond mediated handoff.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_secrets`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
