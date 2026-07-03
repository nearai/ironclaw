# Reborn Production Cutover Readiness Closeout

Issues: #3026, #4621

This note is the final closeout map for the Reborn production wiring and
cutover-readiness epic. It does not make Reborn default-on and it does not
perform a v1 data migration. It records the current production-readiness source
of truth, the rollback/default-off story, and the evidence that production
traffic cannot reach a partially wired Reborn graph.

## Source Of Truth

Production cutover is controlled by `RebornCompositionProfile` plus typed
`RebornBuildInput` / `RebornRuntimeInput`:

| Profile | Runtime behavior |
| --- | --- |
| `disabled` | Default-off. `build_reborn_services` returns no runtime facades and a blocking disabled readiness diagnostic. `build_reborn_runtime` rejects live traffic. |
| `local-dev` | Local-only development profile. Readiness is `DevOnly` and carries a blocking non-production diagnostic. |
| `local-dev-yolo` | Explicit trusted-laptop profile. Readiness is `DevOnly`; host access requires an explicit disclosure/confirmation path. |
| `migration-dry-run` | Builds and validates the production-shaped graph, reports readiness/diagnostics, and rejects live runtime traffic. |
| `production` | Builds production-shaped storage/runtime services and starts live runtime traffic only when readiness is `ProductionValidated` with no blocking diagnostics. |

The live runtime boundary is `build_reborn_runtime`. CLI/WebUI entrypoints reach
that boundary through `ironclaw_reborn_composition`; they do not reconstruct
lower-level stores, `TurnCoordinator`, or host-runtime handles at the route
layer.

## Completed Slice Map

| Slice | Issue | PR | Evidence |
| --- | --- | --- | --- |
| Stable readiness diagnostic vocabulary | #4617 | #4626 | `readiness_serializes_diagnostics_with_stable_redacted_vocabulary`, `readiness_diagnostic_unknown_wire_variants_round_trip_losslessly` |
| Production wiring report -> readiness mapping | #4618 | #4627 | `production_wiring_report_maps_through_public_readiness_entrypoint`, stable component/reason mapping tests |
| Production cutover gate before serving traffic | #4619 | #4682 | Runtime cutover-gate tests and `build_reborn_runtime` caller-path coverage |
| PostgreSQL production storage config | #4551 | #4631 | Production Postgres factory tests in `facade_factory.rs` |
| Production `build_reborn_runtime` launch | #4615 | #4645 | Production runtime launch path and readiness checks |
| Backend-parity readiness coverage | #4620 | #4713 | `libsql_substrate_readiness_diagnostics_cover_required_backend_gaps`, `postgres_substrate_readiness_diagnostics_cover_required_backend_gaps`, `build_reborn_runtime_allows_validated_production_readiness` |
| Operator status/config surfaces consuming readiness | #4595, #4593 | #4737, #4736 | `docs/reborn/contracts/operator-observability-backends.md`, `docs/reborn/contracts/operator-effective-config.md` |

## Acceptance Criteria Mapping

| #3026 criterion | Closeout evidence |
| --- | --- |
| Explicit typed production profile/config path | `RebornCompositionProfile`, `RebornBuildInput`, `RebornRuntimeInput`, profile parsing tests, and `run_honors_boot_profile_from_config_file`. |
| Disabled mode exposes no partial Reborn production services | `RebornServices::disabled`, `disabled_returns_empty_services`, `disabled_readiness_is_redaction_safe`, and `runtime_rejects_disabled_profile_before_local_substrate_lookup`. |
| Local-dev/local-yolo are visibly non-production | `dev_only_profiles_are_visible_non_production_in_readiness`, `local_dev_factory_readiness_includes_non_production_diagnostic`, and `local_dev_yolo_factory_readiness_includes_non_production_diagnostic`. |
| Migration-dry-run validates but does not switch live traffic | `migration_dry_run_validates_libsql_shape`, process-port fail-closed tests, and `runtime_rejects_migration_dry_run_before_live_traffic`. |
| Production fails closed on missing/local-only/unverified/unsupported services | `build_production_shaped` wiring validation, `ProductionWiringReport` mapping tests, and required-backend parity tests for libSQL/PostgreSQL. |
| Redacted stable readiness diagnostics | `readiness_diagnostics_do_not_carry_sensitive_detail_fields`, backend URL/secret redaction assertions, and operator observability backend contract requirements. |
| AppBuilder/default startup stays clear | Reborn production composition remains in `ironclaw_reborn_composition`; legacy `src/main.rs` is covered by `legacy_main_does_not_compose_reborn_runtime`. |
| Reborn binary remains thin bootstrap | `ironclaw-reborn` delegates to command modules and Reborn-owned factories; `reborn_binary_main_is_thin_bootstrap` guards this mechanically. |
| WebUI/Product Workflow consume facade APIs | `RebornWebuiBundle`, `build_webui_services`, product-live adapter tests, and crate guardrails in `crates/ironclaw_reborn_composition/CLAUDE.md`. |
| Required graph components included or diagnosed | Host-runtime `ProductionWiringReport`, Reborn readiness diagnostic component mapping, and #4620 backend-parity readiness tests. |
| PostgreSQL/libSQL parity evidence | #4620 tests plus #4551/#4615 production storage/runtime launch work. |
| No hidden legacy/Reborn dual writers | Reborn startup is separate from legacy `main.rs`; migration/compatibility writes remain under #3029 and are not silently performed by production readiness. |
| Rollback/default-off behavior before final default-on | Default profile is `disabled`; operators can stop the Reborn binary or switch profile back to `disabled` before any future irreversible migration/default-on cutover. |

## Rollback And Default-Off Contract

Before Reborn becomes default-on, rollback is profile/deployment based:

1. Switch Reborn profile to `disabled`, or stop the standalone Reborn binary.
2. Keep the legacy v1 path as the serving path.
3. Do not run irreversible migration/backfill as part of this readiness gate.
4. If `migration-dry-run` is used, treat its result as validation evidence only;
   it must not start the live turn runner or accept product traffic.

Any future bridge that writes both legacy and Reborn state must define its own
idempotent migration/backfill and rollback behavior under #3029. This closeout
does not grant a hidden dual-writer mode.

## External Disposition

These related issues remain valid, but they are no longer blockers for closing
#3026's production-composition readiness story:

| Issue | Disposition |
| --- | --- |
| #3029 migration/compatibility bridges | Still owns actual v1/Reborn migration and bridge behavior. #3026 only proves production readiness stays default-off and does not create hidden dual writers. |
| #3032 no-exposure safeguards | Still owns broad no-exposure product/runtime coverage. #3026 covers readiness/status redaction and production-wiring diagnostics. |
| #3333 production wiring gaps | Used as detailed evidence input. Remaining domain-specific gaps should become component diagnostics or narrower implementation issues, not a second owner-level cutover epic. |
| #3045 runtime presets/effective runtime policy | Runtime policy is required by production composition and tested here; broader profile/preset evolution remains under #3045. |
| #4539 approvals parity | Product approval behavior remains an approvals-parity epic. #3026 covers production wiring for approval request/lease stores and fail-closed readiness. |

## Validation Commands

Recommended closeout validation:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_reborn_composition runtime_rejects_disabled_profile_before_local_substrate_lookup
cargo test -p ironclaw_reborn_composition runtime_rejects_migration_dry_run_before_live_traffic --features libsql --locked
cargo test -p ironclaw_architecture legacy_main_does_not_compose_reborn_runtime
cargo test -p ironclaw_architecture reborn_binary_main_is_thin_bootstrap
```
