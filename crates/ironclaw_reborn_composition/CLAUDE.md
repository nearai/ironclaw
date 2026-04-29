# ironclaw_reborn_composition guardrails

- This crate is the production composition root called by `src/app.rs::AppBuilder`. It composes module-owned substrate factories — it does not own substrate logic.
- Do not move substrate state, contract types, or business logic into this crate. Module-owned factories live in their owning crates and are *called* from here.
- Architecture rule (enforced by `crates/ironclaw_architecture/tests/reborn_composition_boundaries.rs`): no substrate crate may depend on `ironclaw_reborn_composition`. The composition root composes substrate, never the other way around.
- Every successful build must call `RebornProductionServices::validate()` before returning. Coupling rules expand as substrate lands.
- All `RebornBuildError` variants must keep `Display` redaction-safe. No host paths, connection strings, raw secrets, approval reasons, lease IDs, or invocation fingerprints in operator-visible output.
- New factory module = new gate file in `src/factories/` plus a row in the `RebornProductionServices` struct. Gate factories return `SubstrateNotImplemented` under `RebornProfile::Production` until their substrate crate exists.

## Backend exclusions (issue #3026 acceptance criterion #10)

Acceptance criterion #10 of #3026 requires Postgres/libSQL backend factories for every required persistent service, *or* documented exclusions. The exclusions below cover the gap between what's mergeable today and what cutover requires.

| Service | Current backend | Why no Postgres/libSQL factory yet | Production impact | Migration path |
|---|---|---|---|---|
| Durable event log (`event_log`) | `ironclaw_events::InMemoryDurableEventLog` | Postgres/libSQL backends for `DurableEventLog` are deliberately deferred per `crates/ironclaw_events/src/lib.rs` module docs — they depend on `ironclaw_filesystem` (merged) plus the database substrates not yet wired into the events crate | `Production` profile fails closed via `factories::events` returning `SubstrateNotImplemented { service: "durable_event_backend" }`. No production traffic can be served | Issue #3022 (event substrate integration tests) gates the cutover; durable backends ship alongside that work |
| Durable audit log (`audit_log`) | `ironclaw_events::InMemoryDurableAuditLog` | Same as event log — deferred to the same grouped Reborn PR | Same — `Production` fails closed | Same — #3022 |
| Run-state store (`run_state_store`) | `ironclaw_run_state::InMemoryRunStateStore` | Filesystem-backed variant exists in `ironclaw_run_state` (`FilesystemRunStateStore`). Postgres/libSQL factories for it are not yet written | `Production` fails closed via `factories::run_state` returning `SubstrateNotImplemented { service: "durable_run_state_backend" }` | TurnCoordinator (#3013) and AgentLoopHost (#3016) drive the run-state contract; durable backends co-land with those |
| Approval request store (`approval_request_store`) | `ironclaw_run_state::InMemoryApprovalRequestStore` | Same as run-state — filesystem variant exists, DB factories pending | Same — `Production` fails closed (paired by validate() rule 3 with run_state) | Same — #3013, #3016 |
| Resource governor (`resource_governor`) | `ironclaw_resources::InMemoryResourceGovernor` | The reservation governor is intentionally an in-memory service that publishes resource ledger / policy / receipt rows through a separate persistence boundary. The persistence boundary is not yet wired to Reborn | Currently no production gate; if cutover requires a persistent ledger, a `SubstrateNotImplemented { service: "durable_resource_ledger" }` gate will be added | Pending design decision in #2987 epic |
| Filesystem root (`filesystem_root`) | `ironclaw_filesystem::CompositeRootFilesystem` (no mounts) | `LocalFilesystem`, `PostgresRootFilesystem`, and `LibSqlRootFilesystem` all exist in `ironclaw_filesystem`. The composition root currently mounts none of them — `Production` will mount the appropriate backend once it is config-driven | No production gate today, but `Production` should not be considered cutover-ready without configured mounts. A future `factories::filesystem` revision will validate mount presence | Co-lands with the `reborn.filesystem.backend` typed setting and a manifest-based mount catalog |
| Extension registry (`extension_registry`) | Empty `ironclaw_extensions::ExtensionRegistry` | Registry discovery against a real `RootFilesystem` lives in a follow-up that co-lands with the trust-class policy engine (#3012/#3043). Until trust class is host-controlled, manifest discovery cannot safely populate the registry under `Production` | No production gate today, but registry without trust assignment is unsafe — `Production` should add a `SubstrateNotImplemented { service: "trust_class_policy" }` gate (already present in `factories::trust`) | #3043 (trust-class engine) lands first; then `factories::extensions` populates the registry under that gate |

Removing an exclusion from this table requires a Postgres/libSQL backend factory in the corresponding `factories::*` module, paired tests for both backends (per `.claude/rules/database.md`), and updates to the matching `validate()` coupling rule when the contract changes.

## Verification commands

Issue #3026's "Suggested verification" block names crate targets that don't exist yet. The actual verification command set for what's in the workspace today:

```bash
# Substrate + composition crate tests
cargo test -p ironclaw_reborn_composition -p ironclaw_architecture -p ironclaw_events

# Binary-side Reborn surfaces (config + AppBuilder branch + bootstrap)
cargo test -p ironclaw --lib --features libsql -- config::reborn
cargo test -p ironclaw --lib --features libsql -- app::tests::reborn_branch
cargo test -p ironclaw --lib --features libsql -- bootstrap

# Pre-commit safety regression tests (covers the REBORN_BRIDGE rule)
bash scripts/test-pre-commit-safety.sh

# Workspace gates
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

When new substrate crates land (`ironclaw_capabilities`, `ironclaw_processes`, `ironclaw_dispatcher`, `ironclaw_secrets`, `ironclaw_network`, `ironclaw_memory`, `ironclaw_host_runtime`), add them to the first command. The substrate-level integration tests required by issue #3022 will live alongside the durable event backend factory and should be added to the `cargo test -p ironclaw_events` invocation when they land.
