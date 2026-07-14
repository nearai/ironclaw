# Train A Roll-Up Hardening Design

**Date:** 2026-07-13  
**PR:** [#6061](https://github.com/nearai/ironclaw/pull/6061)  
**Baseline:** `b45efc42fff4b24e5938762cd36b11870b9664c8`  
**Status:** Approved for implementation

## Goal

Make Train A a self-contained, upgrade-safe prerequisite for Train B:

- one installable extension object;
- manifest-derived tool, channel, and authentication surfaces;
- runtime as execution metadata rather than a second taxonomy;
- one unified Slack extension/provider identity;
- no live legacy extension rails; and
- no Train B runtime machinery.

The result must preserve current `main` security and lifecycle hardening, migrate
persisted Train A predecessor state without data loss, and establish contracts
that Train B can consume without compatibility aliases or unfinished follow-up
work.

## Non-goals

This hardening pass must not introduce:

- manifest v3 or a resolved-manifest architecture;
- `ToolAdapter`, `ChannelAdapter`, or generic extension entrypoint traits;
- a generic extension host, active snapshot, or loader registry;
- a recipe-driven authentication engine;
- generic ingress routing or verification;
- a delivery coordinator;
- vendor extraction or deletion of the current Slack runtime lane;
- a generic specificity allowlist; or
- multi-account product behavior.

## Confirmed blockers

The exact baseline is green in required CI and GitHub reports it mergeable, but
the following acceptance blockers exist:

1. Persisted schema-v2 manifests with formerly valid top-level
   `[[capabilities]]` fail the new strict parser before lifecycle migration can
   replace them.
2. The `slack_bot` installation fold can discard ownership, activation,
   credential bindings, health, timestamps, and manifest hashes.
3. A build without the Slack feature can persist deletion of the retired Slack
   installation.
4. The `slack_personal` credential-provider migration has no production call
   site and some profiles do not provide the root required for enumeration.
5. Installation and credential migration rewrites use unconditional writes and
   can overwrite concurrent state.
6. Provider identity keys use delimiter concatenation and are not injective.
7. Available-extension summaries reparse raw TOML for channel directions and
   maintain duplicate surface truth.
8. Served API and browser E2E scenarios still use the retired wire contract,
   while the zero-taxonomy gate does not cover them.
9. Promised Train A cleanup is incomplete and several comments, docs, and the PR
   body claim guarantees the code does not enforce.

## Design

### 1. Upgrade-safe installation-state loading

`FilesystemExtensionInstallationStore::load_at` remains the owner of its wire
snapshot transition. It will run snapshot decoding, bounded forward migration,
canonicalization, strict validation, and conditional persistence as one
versioned compare-and-swap operation using the shared filesystem CAS helper.

The transition will:

- recognize only the persisted legacy-v2 shape that this strict cutover made
  obsolete;
- translate legacy top-level capability records into the registered
  capability-provider section;
- validate the converted TOML through the same strict public parser used by all
  other records;
- leave already-current records byte-stable;
- leave malformed persisted bytes untouched and fail startup with a typed
  error; and
- initialize the in-memory store from the snapshot that actually won CAS.

This is a persistence-format transition, not a second public manifest parser.
New input with top-level capabilities remains invalid.

Some legacy local-development mounts explicitly do not implement versioned
CAS. Those mounts must never receive a blind-write fallback: startup may use a
fully validated normalized snapshot in memory while leaving the persisted bytes
untouched and warning that the migration will repeat. CAS-capable production
backends must persist the transition and become byte-stable. Expanding the
legacy local backend into a versioned database is outside Train A.

### 2. Canonical `slack_bot` installation fold

The Slack identity transition will be fallible and feature-safe. It will first
construct a complete prospective snapshot, validate it, and only then allow the
CAS operation to commit.

Each retired row will be retyped to the unified `slack` extension and matching
manifest reference while preserving its persisted fields. Existing unified and
retyped rows will then flow through `canonicalize_installation_rows`, which
remains the single owner of:

- tenant versus member ownership;
- member union;
- activation conflict handling;
- credential-binding union and conflict detection;
- health selection;
- timestamps; and
- canonical installation identity.

The explicit Train A rule is enabled-wins for the complete Slack group. The
migration must preserve a valid existing unified manifest hash. When only the
retired host-bundled manifest exists, the transition supplies the current
host-bundled unified manifest. If the build cannot supply that manifest, it
fails without modifying persisted state; it never deletes user state because a
feature is disabled.

### 3. Durable `slack_personal` credential transition

Production composition will run the credential-provider transition before
publishing product-auth services. Every durable profile that can own the record
must provide the root enumeration seam.

The transition will use a strict migration-specific enumeration path that
returns traversal and read failures. Each candidate record will be reread with
its version and rewritten with bounded versioned CAS. Conflicts cause reread and
retry; they never overwrite refresh, revoke, status, or secret changes.

The migration will be idempotent and fail startup when it cannot prove the
complete transition. A later restart must converge without duplicate accounts
or lost credential data. The periodic Google refresh sweep will retain its
early filtering behavior rather than collecting every account globally.

### 4. One typed surface projection

The validated manifest projection is the sole authority for surface kinds,
channel directions, and connection requirements. Available-extension summaries
will not reparse `raw_toml` or maintain manually synchronized taxonomy caches.

If the existing neutral projection lacks detailed channel attributes, it will
be extended at the owning contract layer with the smallest typed representation
needed by Train A. No Train B resolved-manifest object or adapter abstraction is
introduced.

### 5. Provider identity compatibility and security

New installation-scoped provider keys will use an injective length-prefixed
encoding. Parsing validates byte boundaries, the separator, a non-empty actor
identifier, and a valid typed installation identifier.

Lookups will preserve compatibility with already persisted delimiter-form keys
without making the retired encoding the write format. Binding epoch and
revocation behavior remain unchanged. Internal resolver/store types stay
crate-private until a documented external facade consumer exists.

The identity module remains product-blind as required by Train A, but its
documentation will describe the actual explicit composition wiring rather than
claiming a generic manifest-driven registry that does not exist.

### 6. Residual Train A cleanup

Remove only shims made obsolete by the completed Train A model, including:

- duplicate Slack legacy setup/resolver machinery where the current unified
  path has an equivalent owner;
- no-op internal-extension and deprecated WebUI route helpers;
- unused public identity internals;
- obsolete connect-action vocabulary with no production producer;
- stale gateway queries, split-Slack frontend names, and retired MCP navigation;
- comments pointing at deleted modules or the old companion model; and
- source gates that miss relevant Reborn Python E2E contract files.

Every deletion must be preceded by a repository-wide consumer search and must
preserve current `main` behavior, especially per-user OAuth lifecycle and exact
conversation lookup.

## Acceptance checklist

### Scope and architecture

- [ ] Extension is the only installable object.
- [ ] Tool, channel, and auth surfaces are derived from its manifest.
- [ ] Runtime is execution metadata, not taxonomy.
- [ ] Current `main` security and Slack lifecycle behavior is preserved.
- [ ] No Train B abstraction or unrelated refactor is added.

### Manifest and projection

- [ ] New manifests have one strict v2 parse path.
- [ ] New top-level `[[capabilities]]` input remains rejected.
- [ ] Persisted pre-cutover rows migrate before strict parsing.
- [ ] Malformed state fails without modifying persisted bytes.
- [ ] Converted state is strict-parser validated before commit.
- [ ] Surface kinds, directions, and connection come from one typed projection.
- [ ] No raw-TOML semantic reparse or parallel surface cache remains.

### Slack installation migration

- [ ] Tenant ownership and member ownership are preserved.
- [ ] Enabled-wins is deterministic.
- [ ] Credentials merge and conflicts fail explicitly.
- [ ] Health, timestamp, installation id, and manifest hash policies are
      canonical.
- [ ] Feature-disabled startup never deletes state.
- [ ] Typed errors propagate.
- [ ] Restart is logically idempotent everywhere and byte-stable on CAS-capable
      backends.
- [ ] Concurrent updates cannot be overwritten.

### Slack credential migration

- [ ] All production durable profiles execute it before service publication.
- [ ] Enumeration errors are returned.
- [ ] Writes use bounded versioned CAS.
- [ ] Partial failure cannot be reported as success.
- [ ] Restart converges without lost secrets, status, or accounts.
- [ ] The periodic Google sweep filters during traversal.

### Identity and security

- [ ] New provider keys are injective.
- [ ] Existing persisted keys remain resolvable.
- [ ] Cross-installation and cross-tenant collisions are impossible.
- [ ] Scope, epoch, and revocation checks remain intact.
- [ ] Internal identity machinery is not publicly exported.

### Wire, UI, and lifecycle

- [ ] Wire uses `runtime` plus typed `surfaces`; `kind` is absent.
- [ ] Channel/tool grouping is surface-derived.
- [ ] Unified Slack activation and caller OAuth readiness remain distinct.
- [ ] Host-owned final delivery remains unchanged.
- [ ] Relevant served API and browser E2E use the current contract.
- [ ] Retired routes, names, queries, and helpers are removed.

### Code quality

- [ ] Strong types, typed errors, canonical reducers, and shared CAS are used.
- [ ] No production `.unwrap()` or `.expect()` is introduced.
- [ ] No warn-and-continue migration failure.
- [ ] No `CasExpectation::Any` read-modify-write.
- [ ] No delete-before-validation transition.
- [ ] No duplicate reducer, resolver, DTO, parser, or taxonomy cache.
- [ ] Comments describe enforced behavior.

### Verification

- [ ] Each repair begins with a failing regression test.
- [ ] Installation migration covers mixed, legacy-only, multi-owner, conflict,
      feature-off, malformed, restart, idempotency, and concurrent-write cases.
- [ ] Credential migration is verified through production composition on
      libSQL and PostgreSQL, including restart and conflict behavior.
- [ ] Identity collision and compatibility tests pass.
- [ ] Manifest and projection contract tests pass.
- [ ] Served API and browser E2E pass.
- [ ] Frontend unit, type, and lint checks pass.
- [ ] Owning crates, architecture, retired-taxonomy, and full Reborn E2E pass.
- [ ] Workspace formatting, zero-warning clippy, and pre-commit safety pass.
- [ ] A fresh eight-lens review has no unresolved actionable findings.

### PR readiness

- [ ] Changelog, parity docs, skill guidance, comments, and PR body are accurate.
- [ ] Compatibility and rollback behavior are documented.
- [ ] No unresolved review thread remains.
- [ ] Every required check passes on the exact final SHA.
- [ ] The final diff contains only Train A implementation, tests, cleanup, and
      directly relevant documentation.

## Execution strategy

Implementation is split into six test-driven slices:

1. installation snapshot compatibility and bounded CAS;
2. canonical Slack installation fold;
3. durable credential-provider migration;
4. typed surface projection and provider-key compatibility;
5. residual cleanup and served/browser E2E repair; and
6. documentation, PR metadata, and the complete verification matrix.

Each behavioral slice receives an independent implementation review before the
next slice is considered complete. The final branch is pushed to
`nea25-rollup` only after the exact final commit passes the required matrix.
