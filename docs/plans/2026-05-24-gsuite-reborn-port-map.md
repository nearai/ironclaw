# GSuite Reborn Port Map

This plan supersedes the old Google phase worktrees and PRs #3893-#3898. The
replacement stack starts from `origin/reborn-integration` at `38b8a9210` and
ports only the pieces that still fit the current Reborn auth and first-party
extension boundaries.

## Current Base

- `crates/ironclaw_auth` owns product auth flows, callback claim/fail/complete,
  credential accounts, provider exchange contracts, and in-memory fakes.
- `crates/ironclaw_first_party_extensions` owns concrete first-party userland
  extension behavior, package descriptors, and host-bundled handlers.
- `crates/ironclaw_first_party_extension_ports` owns loop-facing adapter and
  port contracts for first-party extension activation/execution surfaces.
- `crates/ironclaw_reborn_composition` wires built-in first-party packages,
  handler registries, product auth ports, secret stores, and runtime HTTP
  egress.
- `crates/ironclaw_host_runtime` owns host runtime authority,
  `FirstPartyCapabilityHandler`, `FirstPartyCapabilityRegistry`, and built-in
  first-party dispatch.

## Phase Source Decisions

| Old source | Decision | Replacement owner |
| --- | --- | --- |
| Phase 1 `crates/ironclaw_oauth` | Rewrite, do not port as a crate | `crates/ironclaw_auth::oauth` helpers |
| Phase 2 blocked-auth resume path | Drop for this stack | Already covered by current product auth continuation flow |
| Phase 3 Google provider scaffold | Rewrite | `ironclaw_auth` provider helper plus `ironclaw_first_party_extensions::gsuite` |
| Phase 4 composition wiring | Defer | GitHub issue blocked on composition/manifest PRs |
| Phase 5 Calendar package | Port/adapt | `crates/ironclaw_first_party_extensions::gsuite::calendar` |
| Phase 6 Gmail package | Port/adapt | `crates/ironclaw_first_party_extensions::gsuite::gmail` |
| Phase 7 UI prompts | Defer | Follow-up UI issue only if backend needs it |
| Phase 8 live harness | Defer | GitHub issue for ignored/manual live tests |

## Port Rules

- Do not recreate `ironclaw_oauth` or `ironclaw_native_extensions`.
- Keep OAuth callback authority on:
  `RebornProductAuthServices::handle_oauth_callback -> AuthFlowManager`.
- Validate callback flow, scope, state, provider, and PKCE before provider
  exchange.
- Store and select Google credentials through `CredentialAccountService`.
- GSuite handlers must use `RuntimeHttpEgress`; they must not own direct HTTP
  transports.
- Credential injection must use the selected account `SecretHandle`, not a
  process-wide `google_oauth_token` constant.
- Keep Calendar and Gmail package descriptors under first-party extension code;
  composition decides when to register them into the host package/handler
  registries.

## Deferred Issue Hooks

- Composition wiring should reference #3967 and blocking PRs #3939, #3944,
  #3955, #3904, and #3949.
- Shim/live harness should reference #3968 and blocking PRs #3944, #3955, and
  #3903.
- Closeout should reference #3969 and old GSuite PRs #3893-#3898.

## Target Verification

- Phase 2: `cargo test -p ironclaw_auth`
- Phase 3/4: `cargo test -p ironclaw_first_party_extensions`
- Port contract changes: `cargo test -p ironclaw_first_party_extension_ports`
- Dependency changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
