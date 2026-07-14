# Train A Roll-Up Hardening Design

**Date:** 2026-07-13
**PR:** [#6061](https://github.com/nearai/ironclaw/pull/6061)
**Baseline:** `b45efc42fff4b24e5938762cd36b11870b9664c8`
**Status:** Implementation and local acceptance complete; exact-head GitHub verification pending

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
CAS. Startup migration on those mounts must never receive a blind-write
fallback: startup may use a fully validated normalized snapshot in memory while
leaving the persisted bytes untouched and warning that the migration will
repeat. Explicit local-development lifecycle mutations remain durable through a
bounded, single-process compatibility worker; that exception is never selected
for hosted or CAS-capable profiles. CAS-capable production backends must persist
the transition and become byte-stable. Expanding the legacy local backend into
a versioned database is outside Train A.

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

New installation-scoped provider keys use the versioned, injective
`ic1.<installation-byte-length>.<base64url-no-pad(installation)>.<base64url-no-pad(actor)>`
encoding. Parsing validates segment count, the advertised decoded byte length,
base64url canonicality, bounds, a non-empty actor identifier, and a valid typed
installation identifier.

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

### 7. Explicit Train A inbound authority

Train A does not introduce a generic inbound adapter registry. Composition may
own an inbound OAuth connection only for the host-bundled unified `slack`
package whose typed surfaces declare inbound channel delivery and the `slack`
OAuth provider. Install, restore, and activation all enforce that exact
allowlist before publishing or mutating lifecycle state. An unrelated package
cannot gain Slack connection authority merely by combining an inbound surface
with a Slack OAuth requirement.

The available-extension projection exposes a connection requirement only for
that same supported package/provider combination. Other inbound combinations
fail closed until Train B supplies the runtime abstraction that can execute
them; the UI must not present an unusable setup action.

### 8. Retired personal-installation cleanup

The one-time `slack_user` cleanup is authorized only by one strictly valid,
host-bundled retired manifest and installation rows whose manifest references
match both its identifier and hash. Missing, duplicate, malformed,
non-host-bundled, or mismatched provenance fails before mutation. Builds
without the Slack host feature return a typed error and preserve the entire
snapshot byte-for-byte.

### 9. OAuth UI completion fencing

The WebUI treats every OAuth launch and callback as a generation-scoped
operation. Replayed callbacks are ignored while the matching follow-up is in
flight, superseded promise completions cannot close or mutate the current
modal, and failure of the current launch clears its busy state. Activation,
catalog invalidation, success notification, and modal close all re-check the
current generation so a stale request cannot complete a newer setup flow.

### 10. Authoritative installation-store mutations

Every installation-store mutation on a CAS-capable backend rereads the current
winning snapshot, applies one typed mutation, validates the complete candidate,
and commits through the shared bounded root-CAS helper. A successful write is
published to the process-local read projection only after persistence wins; a
failed write cannot leak uncommitted state into memory. Generation fencing
prevents an older successful caller from publishing over a newer successful
caller when their post-write work completes out of order.

Fresh installation commits the manifest and installation in one store
transition. It must never leave an orphan manifest that poisons retry after an
interruption. The explicitly opted-in non-CAS local-development path serializes
normal lifecycle mutations through one bounded worker and preserves durable
install, activate, and remove behavior. It is a compatibility boundary, not a
production multi-writer guarantee.

### 11. OAuth rollback generation preservation

Rollback of an active Slack connection generation fences that generation's
identity without erasing a newer pending replacement. The exact sequence
active A, begin B, roll back A, complete B must leave B eligible to activate.
Existing disconnect behavior and rollback of an unsuperseded generation remain
unchanged.

### 12. Bounded credential traversal and deployment fence

The strict `slack_personal` migration has aggregate budgets in addition to the
per-directory limits: 8,192 retained owners, 65,536 base/session scopes, and
65,536 account-directory entries. Every retained owner, every base or session
scope (including empty roots), and every directory entry (including malformed
or nonmatching entries) consumes budget before filtering. Exceeding a budget
fails startup with the existing sanitized backend-unavailable result before any
migration write.

This migration is a one-time stop-the-world cutover. All pre-Train-A writers
must be stopped before the new binary starts migration; a rolling overlap can
otherwise create a new retired row after enumeration. Rollback after the data
transition requires restoring the pre-cutover state backup or rolling forward,
because the old binary does not recognize the unified provider identity.

## Merge-readiness contract

### The PR must do

- Expose one install/search/list/activate/remove lifecycle for extension
  package references; channel and tool views are projections of that object.
- Parse every new manifest through the registered strict-v2 contract path and
  reject legacy top-level capabilities at public ingestion boundaries.
- Upgrade only recognized persisted predecessor shapes, validate the complete
  candidate, and commit it with bounded CAS before publishing services.
- Apply every CAS-capable installation mutation to the latest persisted
  snapshot, publish only the winning result, and install manifest plus
  installation atomically.
- Preserve ownership, activation, credential bindings, health, timestamps,
  manifest authority, caller scope, binding epoch, and revocation semantics
  across migration and compatibility reads.
- Derive runtime, surfaces, channel direction, connection strategy, and UI
  grouping from typed manifest/product projections.
- Authorize the Train A inbound connection only for the exact host-bundled
  unified Slack package/provider combination at install, restore, activation,
  and available-extension projection boundaries.
- Fail closed before side effects when Train A has no authenticated owner for
  an inbound connection workflow.
- Keep final outbound delivery host-owned and Slack conversation resolution
  exact; extension tools do not become a second delivery path.
- Report exact verification evidence, including real PostgreSQL and served
  browser lanes, with environmental skips stated rather than implied green.
- Bound the complete credential-migration traversal and document/enforce the
  quiesced one-time deployment procedure operationally.

### The PR must not do

- Add a second installable channel/MCP/tool object, legacy runtime alias, or
  package-name classifier.
- Add manifest v3, generic adapters/ingress/delivery, a recipe auth engine, or
  another Train B abstraction to make this patch appear complete.
- Blind-write a migrated snapshot, use `CasExpectation::Any` for a
  read-modify-write transition, delete before validation, or silently continue
  after a migration error.
- Deploy old and new writers concurrently during the one-time credential
  migration, or claim that new-binary CAS can fence a pre-Train-A writer.
- Let a failed canonical activation write shadow a still-authoritative legacy
  predecessor, or trust payload identity without validating its physical key.
- Treat arbitrary inbound plus arbitrary OAuth surfaces as composition-owned,
  or delete retired personal state without strict manifest/ref/hash authority.
- Present fabricated connection/gateway state, infer pairing from onboarding
  prose, expose a route that is guaranteed to 404, or swallow post-OAuth
  activation failure.
- Introduce public identity internals, mirror DTOs, raw-string domain enums,
  production `.unwrap()`/`.expect()`, hardcoded secrets, or unrelated churn.

### Reject these code smells

- Two parsers, reducers, provider resolvers, surface caches, connection
  classifiers, or cleanup paths for the same semantic decision.
- `starts_with` tenant/installation matching, delimiter-concatenated compound
  identities, unbounded retry loops, or process mutexes held across backend I/O.
- Best-effort success after a durable or side-effecting step failed; generic
  logs in place of typed propagation; comments claiming guarantees with no
  caller-level test.
- Frontend branches keyed on `slack`, `wasm`, `mcp`, or a retired `kind` when a
  typed surface/connection field owns the decision.
- Async OAuth handlers that lack generation checks across every awaited step,
  allow callback replay during follow-up, or leave the current flow busy after
  launch failure.
- A green unit helper with no production caller test, a route test that never
  boots the served binary, or a taxonomy gate that scans the wrong enclave.

### Positive evidence expected in the final diff

- One canonical reducer and one shared bounded-CAS primitive are reused.
- Compatibility is narrow, directional, idempotent, and absent from normal
  writes; feature-disabled builds preserve bytes and fail closed.
- Unsupported inbound package/provider combinations and unauthoritative
  retired-state cleanup attempts are rejected before lifecycle mutation.
- Error types retain useful causes without leaking secrets, paths, tokens, or
  provider payloads.
- Tests cover malformed state, conflict/retry, restart, rollback, legacy-only,
  mixed-generation, cross-user/cross-installation, and corrupt physical-path
  cases through production callers.
- Deletions reduce live vocabulary and branches; any retained exception is a
  named migration/test boundary with a machine-enforced scope.

## Acceptance checklist

This checklist records the code and local acceptance state of the final
pre-push worktree. The two GitHub-only gates remain deliberately unchecked
until the pushed commit has completed review-thread and required-check
verification; their live result belongs in the PR body because checking either
box in this commit would necessarily create a new, unverified commit.

### Scope and architecture

- [x] Extension is the only installable object.
- [x] Tool, channel, and auth surfaces are derived from its manifest.
- [x] Runtime is execution metadata, not taxonomy.
- [x] Current `main` security and Slack lifecycle behavior is preserved.
- [x] No Train B abstraction or unrelated refactor is added.

### Manifest and projection

- [x] New manifests have one strict v2 parse path.
- [x] New top-level `[[capabilities]]` input remains rejected.
- [x] Persisted pre-cutover rows migrate before strict parsing.
- [x] Malformed state fails without modifying persisted bytes.
- [x] Converted state is strict-parser validated before commit.
- [x] Surface kinds, directions, and connection come from one typed projection.
- [x] No raw-TOML semantic reparse or parallel surface cache remains.

### Slack installation migration

- [x] Tenant ownership and member ownership are preserved.
- [x] Enabled-wins is deterministic.
- [x] Credentials merge and conflicts fail explicitly.
- [x] Health, timestamp, installation id, and manifest hash policies are
      canonical.
- [x] Feature-disabled startup never deletes state.
- [x] Typed errors propagate.
- [x] Restart is logically idempotent everywhere and byte-stable on CAS-capable
      backends.
- [x] Concurrent updates cannot be overwritten.
- [x] Failed writes do not publish uncommitted in-memory state.
- [x] Fresh manifest plus installation persistence is one atomic transition.
- [x] Explicit non-CAS local development still persists install, activate, and
      remove through its bounded single-process compatibility path.
- [x] Retired personal cleanup requires one strict host-bundled manifest and
      exact installation manifest-ref/hash provenance.
- [x] Missing, duplicate, malformed, non-bundled, feature-off, and mismatched
      provenance preserve the snapshot without partial deletion.

### Slack credential migration

- [x] All production durable profiles execute it before service publication.
- [x] Enumeration errors are returned.
- [x] Writes use bounded versioned CAS.
- [x] Partial failure cannot be reported as success.
- [x] Restart converges without lost secrets, status, or accounts.
- [x] Aggregate owner, scope, and account-entry budgets fail closed before
      writes; empty and nonmatching traversal still consumes budget.
- [x] The one-time deployment requires pre-Train-A writers to be quiesced, and
      rollback requires pre-cutover restore or roll-forward.
- [x] The periodic Google sweep filters during traversal.

### Identity and security

- [x] New provider keys are injective.
- [x] Existing persisted keys remain resolvable.
- [x] Cross-installation and cross-tenant collisions are impossible.
- [x] Scope, epoch, and revocation checks remain intact.
- [x] Internal identity machinery is not publicly exported.

### Wire, UI, and lifecycle

- [x] Wire uses `runtime` plus typed `surfaces`; `kind` is absent.
- [x] Channel/tool grouping is surface-derived.
- [x] Unified Slack activation and caller OAuth readiness remain distinct.
- [x] Install, restore, activation, and projection accept only the explicit
      host-bundled Slack inbound/OAuth combination owned by Train A.
- [x] OAuth replay and stale async completions are generation-fenced, and a
      current launch failure clears the busy state.
- [x] Rollback of active generation A preserves a newer pending generation B,
      while fencing A's identity and allowing B to activate.
- [x] Host-owned final delivery remains unchanged.
- [x] Relevant served API and browser E2E use the current contract.
- [x] Retired routes, names, queries, and helpers are removed.

### Code quality

- [x] Strong types, typed errors, canonical reducers, and shared CAS are used.
- [x] No production `.unwrap()` or `.expect()` is introduced.
- [x] No warn-and-continue migration failure.
- [x] No `CasExpectation::Any` read-modify-write.
- [x] No delete-before-validation transition.
- [x] No duplicate reducer, resolver, DTO, parser, or taxonomy cache.
- [x] Comments describe enforced behavior.

### Verification

- [x] Each repair begins with a failing regression test.
- [x] Installation migration covers mixed, legacy-only, multi-owner, conflict,
      feature-off, malformed, restart, idempotency, and concurrent-write cases.
- [x] Credential migration is verified through production composition on
      libSQL and PostgreSQL, including restart and conflict behavior.
- [x] Identity collision and compatibility tests pass.
- [x] Manifest and projection contract tests pass.
- [x] Served API and browser E2E pass.
- [x] Frontend unit, type, and lint checks pass.
- [x] Owning crates, architecture, retired-taxonomy, and full Reborn E2E pass.
- [x] Workspace formatting, zero-warning clippy, and pre-commit safety pass.
- [x] A fresh eight-lens review has no unresolved actionable findings.

### PR readiness

- [x] Changelog, parity docs, skill guidance, and comments are accurate; the PR
      body is updated as part of the push operation.
- [x] Compatibility and rollback behavior are documented.
- [ ] No unresolved review thread remains.
- [ ] Every required check passes on the exact final SHA.
- [x] The final diff contains only Train A implementation, tests, cleanup, and
      directly relevant documentation.

## Verification evidence

The final pre-push worktree passed:

- `cargo test -p ironclaw_filesystem --all-features`, including 9/9 real-backend
  CAS storm tests across in-memory, libSQL, and PostgreSQL;
- `cargo test -p ironclaw_extensions --all-features`;
- `cargo test -p ironclaw_product_workflow --all-features`;
- `cargo test -p ironclaw_webui_v2 --all-features`;
- `cargo test -p ironclaw_reborn_composition --all-features` and a final focused
  1,776/1,776 library-test rerun after the last migration hardening;
- the ignored production PostgreSQL migration test against
  `postgresql://benjaminkurrek@localhost/ironclaw_6061_test` (1/1);
- `cargo test -p ironclaw_architecture` (8 composition-boundary, 34
  dependency-boundary, and 4 retired-taxonomy tests);
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `bash scripts/reborn-e2e-rust.sh`;
- frontend lint and 722/722 unit tests across 89 files under Node 22;
- the served WebUI browser lane (42/42);
- `PYTHONTRACEMALLOC=10 tests/e2e/.venv/bin/python -W
  always::ResourceWarning -m unittest
  scripts.reborn_webui_v2_live_qa.test_run_live_qa` (179/179, no skips; Python
  3.14 reported pre-existing SQLite `ResourceWarning`s at unchanged test
  helper lines);
- `scripts/ci/check-reborn-qa-fixtures.sh` (13 files);
- `cargo fmt --all -- --check`, `git diff --check`, and
  `scripts/pre-commit-safety.sh`; and
- fresh architecture/scope, security/concurrency, and tests/docs reviewers,
  with no remaining actionable finding after the accepted fixes were rerun.

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
