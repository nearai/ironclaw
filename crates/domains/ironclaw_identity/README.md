# ironclaw_identity

The canonical identity layer: it maps every external identity — browser OAuth
logins and external channel actors — to a stable `UserId` *before* any runtime
state is touched, is the durable home of the minimal user profile and the
admin user directory, and (as its `projects` module) owns the Project entity,
membership ACL, and the project access-gating service. Bottom-of-stack, with a
machine-enforced never-reach-upstream dependency rule.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_identity` · **Manifest:** `crates/domains/ironclaw_identity/Cargo.toml`
- **Use this when:** resolving or minting a user for an OAuth login, looking
  up/administering user profiles, or working with project records, membership,
  or project access gating.
- **Don't use this when:** binding a channel conversation to a thread →
  `ironclaw_conversations` (it consumes an already-resolved `UserId`);
  post-OAuth *channel* binding `(provider, provider_user_id) → user` → the
  channel identity store in `ironclaw_extension_host` (see `CONTRACT.md`,
  "Two external-identity stores"); browsing project *files* → the
  composition-resident browse reader (deliberately not here).

## Public surface

- `RebornIdentityResolver` (`resolve_or_create` — the only user-minting path
  in the stack; channel actors fail closed with `ChannelActorNotMintable`;
  `adopt_migrated_identity` for legacy seeds) and `RebornIdentityStore`.
- `RebornUserDirectory` — the separate admin enumeration/management trait,
  kept apart so admin CRUD cannot perturb minting invariants.
- Key newtypes: `ProviderKind`, `ProviderInstanceId`, `ExternalSubjectId`;
  `SurfaceKind`; `RebornIdentityError`.
- `projects` module: `ProjectRecord`, `ProjectMemberRecord`, `ProjectRole`,
  `ProjectRepository` + `FilesystemProjectRepository`, and
  `projects::service::RebornProjectService` — the gating service implementing
  `ironclaw_product_contracts::project_service::ProjectService`, with access
  resolved live on every request (never cached).

## Depends on / consumed by

- **Normal deps (measured):** exactly `ironclaw_host_api`,
  `ironclaw_filesystem`, `ironclaw_product_contracts` — the armed three-entry
  allowlist (the third arrived 2026-08-05 via PROPOSAL §12.13 D-Q, exactly one
  entry, contracts-layer and downward).
- **Consumed by (2):** `ironclaw_composition` (curated facade) and
  `ironclaw_assistant` (admin directory + project surfaces).

## Invariants

- **Never reach upstream:** the allowlist is enforced in
  `reborn_crate_dependency_boundaries_hold`
  (`reborn_dependency_boundaries.rs`) — every other workspace crate is
  forbidden, so a fourth entry is a documented decision, not a drift.
- Channel actors never mint; verified-email linking is gated to the
  browser-OAuth surface — the security boundary that stops cross-surface
  account collapse (`CONTRACT.md`, invariants 1 and 4).
- Project access is resolved live — a revoke and a re-grant are each visible
  on the next call; pinned in both directions by
  `tests/project_repository_contract.rs`.
- The `projects` record half persists; the service half authorizes; a role
  check must never move into the store (`CONTRACT.md`, "The `projects`
  module").

## Tests

```bash
cargo test -p ironclaw_identity
cargo test -p ironclaw_identity --test project_repository_contract
```

## See also

- **The module spec is [`CONTRACT.md`](./CONTRACT.md)** (named in the root
  `CLAUDE.md` Module Specs table) — read it before changing anything here;
  this README only orients.
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.11–§6.4.12, §12.13
  D-P/D-Q.
