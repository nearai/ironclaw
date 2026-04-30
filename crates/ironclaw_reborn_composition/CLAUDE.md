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
| Run-state store (`run_state_store`) | `ironclaw_run_state::InMemoryRunStateStore` | Filesystem-backed variant exists in `ironclaw_run_state` (`FilesystemRunStateStore`) but is borrow-only (`&'a F`); no `Arc<F>` constructor yet, so it cannot be stashed as a typed substrate handle. Postgres/libSQL factories not written | `Production` fails closed via `factories::run_state` returning `SubstrateNotImplemented { service: "durable_run_state_backend" }` | TurnCoordinator (#3013) and AgentLoopHost (#3016) drive the run-state contract; durable backends co-land with those. Both build against PR #3095 (`feat(reborn): add host runtime contract facade`) — when that merges, #3013 / #3016 unblock and the run-state durable factory follows |
| Approval request store (`approval_request_store`) | `ironclaw_run_state::InMemoryApprovalRequestStore` | Same as run-state — filesystem variant exists but borrow-only, DB factories pending | Same — `Production` fails closed (paired by validate() rule 3 with run_state) | Same — #3013, #3016 (precursor PR #3095) |
| Resource governor (`resource_governor`) | `ironclaw_resources::InMemoryResourceGovernor` | The reservation governor is intentionally an in-memory service that publishes resource ledger / policy / receipt rows through a separate persistence boundary. The persistence boundary is not yet wired to Reborn | Currently no production gate; if cutover requires a persistent ledger, a `SubstrateNotImplemented { service: "durable_resource_ledger" }` gate will be added | Pending design decision in #2987 epic |
| Filesystem root (`filesystem_root`) | `ironclaw_filesystem::CompositeRootFilesystem` (no mounts) | `LocalFilesystem`, `PostgresRootFilesystem`, and `LibSqlRootFilesystem` all exist in `ironclaw_filesystem`. The composition root currently mounts none of them — `Production` will mount the appropriate backend once it is config-driven | No production gate today, but `Production` should not be considered cutover-ready without configured mounts. A future `factories::filesystem` revision will validate mount presence | Co-lands with the `reborn.filesystem.backend` typed setting and a manifest-based mount catalog |
| Extension registry (`extension_registry`) | Empty `ironclaw_extensions::ExtensionRegistry` | Registry discovery against a real `RootFilesystem` lives in a follow-up. The trust policy engine is wired and merged (PR #3043 / issue #3012 closed 2026-04-29); `factories::trust` uses `HostTrustPolicy::empty()`. Registry population gates on a typed-settings overlay that picks bundled / admin / signed sources | No production gate today, but registry without populated trust sources is conservative-by-default (every manifest gets the Sandbox/UserTrusted default decision). The pairing is enforced by `validate()` rule 6 (`extension_registry` ↔ `trust_policy`) | Co-lands with the `reborn.trust_policy.backend` typed setting; then `factories::extensions` populates the registry against the configured sources |
| Trust policy (`trust_policy`) | `ironclaw_trust::HostTrustPolicy::empty()` | The policy engine substrate landed via PR #3043 (issue #3012). Its `PolicySource` chain is not config-driven yet — that's the typed-settings overlay below. An empty chain returns the default Sandbox/UserTrusted decision for every manifest, which is the safe fail-closed answer | No production gate — an empty policy is conservative by construction. A future `factories::trust` revision will require the chain to be non-empty (or a documented `bundled-only` profile) under `Production` | Co-lands with the `reborn.trust_policy.backend` typed setting; the substrate is merged so the work is purely additive |
| Secret store (`secret_store`) | `ironclaw_secrets::InMemorySecretStore` | Filesystem-encrypted and PG/libSQL-backed `SecretStore` impls don't exist in `ironclaw_secrets` yet | `Production` fails closed via `factories::secrets` returning `SubstrateNotImplemented { service: "durable_secret_store" }`. Material would not survive restart | Co-lands with the durable backend in `ironclaw_secrets` and the `reborn.secrets.backend` typed setting |
| Network policy enforcer (`network_enforcer`) | `ironclaw_network::StaticNetworkPolicyEnforcer` over `NetworkPolicy::default()` | A live `NetworkPolicyStore` (per-scope persisted policies with PG/libSQL backends) is not yet specified in `ironclaw_network` | `Production` fails closed via `factories::network` returning `SubstrateNotImplemented { service: "durable_network_policy_backend" }`. A deny-all default is not a cutover-ready policy | Co-lands with the `reborn.network.policy_backend` typed setting |
| Process services (`process_services`) | In-memory `ProcessStore` + `ProcessResultStore` + shared `ProcessCancellationRegistry` | The substrate exposes a `filesystem(Arc<F>)` preset but no PG/libSQL factory | `Production` fails closed via `factories::processes` returning `SubstrateNotImplemented { service: "durable_process_store" }`. Process records would not survive restart | Co-lands with the `reborn.processes.backend` typed setting; PG/libSQL backends will hang off `ProcessServices::from_parts` once they exist |

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
