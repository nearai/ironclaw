# ironclaw_reborn_migration

Standalone migration crate for Reborn operator migrations. The default build is
legacy-free and ships the Reborn extension ownership migration binary. The full
**IronClaw v1 / engine-v2 persisted state** importer remains available behind
the explicit `full-migration` feature and ships as `ironclaw-reborn-migration`;
the conversion engine is a library (`run_migration`) so it can later be wired
into `ironclaw-reborn` startup.

- **Read side** = the root `ironclaw` crate (`ironclaw::db::connect_with_handles`)
  — one v1 database (PostgreSQL **or** libSQL). Engine-v2 state is **not** a
  separate DB: missions/projects/threads were persisted by the v2 bridge as JSON
  blobs inside the v1 `memory_documents` table under `engine/…` /
  `.system/engine/…` paths. Parsed via the serde mirrors in `v2_model.rs` (the
  engine-v2 types were deleted; they survive only at git tag `old_engine_v2`).
- **Write side** = Reborn domain stores built directly over a `RootFilesystem` /
  triggers DB in `target.rs`, without booting a `RebornRuntime`. Threads /
  secrets / identity force a concrete filesystem type, so they are built inside
  the backend match arm and stored as `#[async_trait]` trait objects.
- **Philosophy: nothing is silently dropped.** Infrastructure errors abort; a
  value with no Reborn representation is recorded as a `LossyItem` on the
  `MigrationReport` (the manifest), with the reason and the Reborn gap named.

```
cargo run -p ironclaw_reborn_migration --features full-migration,libsql -- \
  --source-libsql ~/.ironclaw/ironclaw.db \
  --target-libsql ./reborn-local-dev.db \
  --tenant-id default --agent-id default --dry-run
```

## What converts, and where losses go

| v1 / engine-v2 source | Reborn target | Status |
|---|---|---|
| `conversations` + `conversation_messages` | `SessionThreadRecord` + transcript (orig id preserved via `EnsureThreadRequest.thread_id`; per-message role/ts/id in `metadata_json.legacy_v1`) | **full** |
| routine `Trigger::Cron` | `TriggerRecord` (`TriggerSchedule::Cron`) via `TriggerRepository::upsert_trigger` | **full** |
| engine-v2 mission `Cadence::Cron` | `TriggerRecord`; `thread_history` → threads under `ThreadScope.mission_id` | **full** |
| `memory_documents` (non-engine) | `ironclaw_memory` documents (`MemoryService::write`) | **full** |
| `secrets` | decrypt via v1 `SecretsStore` → re-encrypt via Reborn `SecretStore::put` (needs `--secret-master-key`) | **full** |
| `user_identities` (OAuth) + `channel_identities` | `RebornIdentityResolver::adopt_migrated_identity` (`SurfaceKind::Oauth` / `ChannelActor`) | **full** |
| `wasm_tools` / `wasm_channels` installs | `ExtensionInstallation` (+ synthesized `capability_provider` manifest) via composition's `migration-support` seam; `tool_capabilities.allowed_secrets` → credential bindings | **full (manifest is a placeholder — see below)** |
| routine `Trigger::{Event,SystemEvent,Webhook,Manual}` | — (Reborn `TriggerSourceKind` = `Schedule` only) | **gap → report** |
| mission `Cadence::{OnEvent,OnSystemEvent,Webhook,Manual}` | — | **gap → report** |
| routine guardrails / notify / run counters; mission focus / approach / success-criteria / notify | — (no trigger field / no durable mission entity) | **gap → report** |
| `routine_runs` history | — (`TriggerRepository` has no public run-history insert) | **gap → report** |
| routine/mission `Failed` status | `TriggerState::Paused` | **degraded → report** |
| non-user/assistant transcript messages (system/tool) | retained in thread `metadata_json.legacy_v1`, not a standalone row | **degraded → report** |
| `settings` (key/value) | — (Reborn config is typed `config.toml`/`providers.json`/`LlmKeyStore`, no generic KV store) | **gap → report** |
| `memory_document_versions` | — (no per-doc version history in Reborn) | **gap → report** |
| `agent_jobs` / `job_actions` / `job_events` | — (Reborn has no general job store) | **gap → report** |
| `heartbeat_state` | — (re-establish as a scheduled trigger) | **gap → report** |
| extension manifest fidelity + WASM binary; tool capability config; channel→secret binding; `pairing_requests` | — | **degraded/gap → report** |

`Domain` + `LossReason` on each `LossyItem` make the manifest greppable; the
acceptance test asserts the **exact** gap set so a regression that silently drops
a domain fails the build.

## Notes on the "full" deferred converters

- **Secrets** — needs `--secret-master-key` (used verbatim as the HKDF IKM, as
  in v1). The v1 store is built from the raw `DatabaseHandles`; each secret is
  listed, decrypted (`get_decrypted`), and re-encrypted through
  `RebornTarget::secret_store` (`FilesystemSecretStore`). Expiry is preserved; a
  secret that fails to decrypt (expired / wrong key) is a per-secret loss, not a
  run abort. Without a key, secrets are skipped with a recorded loss.
- **Identities** — `user_identities` read via the `Database` trait,
  `channel_identities` via raw SQL (no trait accessor). Adoption preserves the v1
  `UserId` and seeds the verified-email index. Idempotent (safe to re-run).
- **Extensions** — installed tools/channels become `ExtensionInstallation`s with
  activation from the v1 `status` and credential bindings from
  `tool_capabilities.allowed_secrets`. The synthesized manifest declares
  `ironclaw.capability_provider/v1` + one `ask`-permission placeholder capability
  (a non-first-party manifest must declare a host API or capability). **The v1
  capability contract and WASM binary are NOT carried over** — the manifest is a
  migration placeholder, recorded as a `manifest_fidelity` loss per installation.
  The store is opened through the composition `migration-support` seam
  `extension_installation_store_for_migration` (mirrors composition's
  `*_for_test` accessors; ships zero bytes without the feature).

## Remaining follow-up — wire into `ironclaw-reborn` startup

Call `run_migration` in `crates/ironclaw_reborn_cli/src/runtime/mod.rs` after the
storage root is resolved and before `build_reborn_runtime`, mirroring
`with_run_local_trigger_fire_access_checker`; or add a `Command::Migrate`
subcommand. The `run --dry-run` output already reserves a `v1_state:` line.
Deferred per the original PR scope.

## Mount layout caveat

`mounts.rs` reproduces the production alias→path layout (memory `/memory`;
threads/secrets tenant/user-scoped) because the canonical resolver is **private**
in `ironclaw_reborn_composition`. It MUST be reconciled with composition when the
startup wiring lands so the runtime reads back exactly what was migrated. The
acceptance test verifies round-trip through the **same** services the migration
writes with, pinning conversion correctness independently of that reconciliation
(end-to-end runtime-readback is the wiring follow-up).

## Tests

`tests/migration_roundtrip.rs` (`required-features = ["full-migration",
"libsql"]`, Docker-free):
seeds a rich v1+engine-v2 fixture (conversations, every routine trigger variant,
cron + non-cron missions with a mission thread, memory docs, settings, a secret,
an OAuth + a channel identity, an installed WASM tool), runs the migration, and
asserts converted counts (including secrets/identities/extensions), the exact gap
set, triggers read back through the **public** `LibSqlTriggerRepository`, and
on-disk durability of thread / secret / extension-installation documents via a
fresh connection. A second case asserts `--dry-run` reports fully but writes
nothing. Add a Postgres variant with the `postgres_pool_or_skip()`
skip-if-no-Docker helper (see `crates/ironclaw_reborn_composition/tests/postgres_substrate.rs`).

```
cargo test -p ironclaw_reborn_migration --features full-migration,libsql --test migration_roundtrip
```
