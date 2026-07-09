# ADR 0001: Extension Manifest Graph and Adapter Seams

- Status: Accepted for implementation
- Date: 2026-07-09
- Deciders: IronClaw Reborn maintainers
- Related design: `docs/superpowers/specs/2026-07-09-unified-extension-runtime-design.md`

## Context

NEA-25 establishes one extension identity and projects product-facing tool,
channel, and auth surfaces from its manifest. It does not yet make the runtime
generic. The checked-out tip still manually constructs Slack in Reborn
composition, mounts Slack-specific routes, selects Slack-specific OAuth code,
and repeats manifest-owned channel metadata inside the runtime adapter.

We need one architecture that supports all of the following without returning
to separate tool, channel, and auth products:

- a single extension may expose several capability kinds;
- a large extension may author those declarations in small files;
- the installed contract remains one immutable source of authority;
- runtime implementations cannot silently add or widen declared authority;
- generic IronClaw subsystems never branch on an extension, channel, or
  provider name;
- protocol-specific behavior is implemented once, in extension-owned code;
- activation, upgrade, restore, and removal cannot expose a partially wired
  extension.

## Decision

### 1. Extension is the only installable product object

One `ExtensionId` owns every surface in the package. Tool, channel, auth,
trigger, and file are capability-surface kinds, not independently installable
product types. Runtime kind is an implementation detail and never determines
product taxonomy.

`ProviderId` remains a credential-account authority referenced by an auth
surface. It is not an alias for `ExtensionId` and cannot identify an installed
product.

### 2. One logical manifest compilation unit may use typed leaf files

The package root contains the only installable `manifest.toml`. The root owns
identity, version, requested trust, runtime selection, host-API membership, and
an explicit ordered list of fragment paths for each fragmented host-API
section.

Fragments use `reborn.extension_fragment.v1`. They are typed, non-installable,
non-recursive leaves. They cannot declare identity, version, trust, runtime,
host-API membership, or further imports. Imports are local, explicit, bounded,
and package-contained; globs, URLs, absolute paths, parent traversal,
backslashes, symlink escape, duplicate paths, and nested imports are rejected.

The first cutover uses `reborn.extension_manifest.v3` roots. The v2 inline
reader remains only as a compatibility input while existing packages migrate.
All first-party packages migrate to v3 so fragments are a generic facility,
not a Slack exception.

Each host-API contract owns fragment-body validation and aggregation. The
generic resolver owns only bounded file loading, path containment, envelope
validation, ordering, provenance, and digest framing. There is no generic TOML
deep merge, override, or last-writer-wins behavior.

The compiler produces a canonical `ResolvedExtensionManifest` plus an exact
source map. Discovery, trust, lifecycle, frontend projection, and runtime
binding consume that immutable result. They do not reparse root TOML or reread
fragment files.

Compilation reads one immutable `InstalledPackageSnapshot` containing the full
indexed package, authenticity envelope, and dependency lock. A
content-addressed package store retains the exact runtime/assets for restart,
rollback, and generation-leased asynchronous work.

### 3. Package and contract integrity are distinct

`PackageDigest` covers every named file in the immutable package, using a
version-framed path-and-bytes hash. `ContractDigest` covers the canonical
resolved manifest contract. Both are persisted with the resolved snapshot and
installation generation.

A byte change to a fragment or asset changes `PackageDigest`. A semantic
authority change changes `ContractDigest` and requires trust/approval
reevaluation. Formatting-only manifest changes may leave `ContractDigest`
unchanged but can never leave `PackageDigest` unchanged.

### 4. One extension entrypoint binds all executable surfaces

Every runtime kind loads one `ExtensionEntrypoint`. Given an installation
context and the resolved contract, the entrypoint returns an `ExtensionBindings`
map keyed by declared surfaces.

Loading/construction/binding are side-effect-free and receive no authority
ports. The loader attaches unforgeable origin/package/export/ABI provenance;
optional read-only readiness runs only after the local join.

The runtime returns implementations only. It does not redeclare IDs, effects,
schemas, directions, scopes, routes, auth policy, credential handles, or egress
authority.

`BoundExtension::try_new` is the only local join point. Loader-issued,
unforgeable provenance identifies which root/dependency entrypoint supplied
each implementation. The join requires a bijection:
every executable declared surface has exactly one correctly typed binding, and
there are no missing, extra, duplicate, wrong-kind, wrong-owner, or
ABI-incompatible bindings. Global conflicts are checked against the staged
active snapshot; runtime authority is enforced by scoped ports and restricted
egress.

### 5. Use one entrypoint, not one God adapter

`ExtensionEntrypoint` is an assembly boundary, not a massive operational
trait. It returns narrow adapters owned by the subsystem that consumes them:

- a tool binding invokes one declared capability;
- channel ingress inspects and normalizes bounded protocol input;
- channel outbound renders normalized semantic output;
- channel connection manages provider-specific connect actions behind generic
  host routes;
- channel target resolution understands provider-specific target identifiers;
- auth handles provider request/response quirks behind the host's generic OAuth
  state machine;
- future trigger and file bindings use their own narrow interfaces.

The current `ProductAdapter` becomes channel-focused operational interfaces and
loses metadata getters that duplicate the manifest. It is not expanded into a
cross-capability mega-trait.

### 6. The host owns authority; extensions own protocol semantics

The generic host owns trust evaluation, signature and package verification,
body/rate/concurrency limits, route matching, secret storage and injection,
OAuth state/PKCE/replay protection, caller and tenant scope, identity and
conversation binding, idempotency, authorization, approvals, retry scheduling,
delivery attempts, persistence, audit, and lifecycle.

Extension-owned adapters own vendor payload parsing, installation hints,
challenge responses, vendor target formats, message rendering, provider OAuth
endpoints and response parsing, refresh/revoke quirks, and tool behavior. They
can exercise host authority only through scoped host ports derived from the
resolved manifest and effective trust decision.

Native first-party implementations are trusted computing-base code and are
enforced architecturally to use ports; only sandbox runtimes receive hard
authority isolation.

### 7. Publish immutable active generations atomically

A new `ironclaw_extension_host` deep module owns install, bind, activate,
deactivate, upgrade, restore, and the immutable active-extension snapshot.
Composition supplies stores, runtime loaders, and host ports; it does not
construct a concrete extension.

Activation stages and validates the entire next generation, commits durable
activation state with compare-and-swap, then swaps one immutable in-memory
snapshot. Runtime readers receive narrow resolver ports rather than depending
on `ironclaw_extension_host` directly. In-flight work retains its old
generation while new work sees the new generation. Startup reconstructs all
enabled generations before one publication step; a failed extension is
quarantined without partially publishing its surfaces.

## Consequences

### Positive

- Adding an ordinary extension requires no concrete product branch in host,
  workflow, auth, lifecycle, CLI, composition, or frontend code.
- Tool, channel, and auth declarations have one authority source and one
  runtime implementation each.
- Large packages stay reviewable without making fragments independently
  installable or authoritative.
- Trust, activation, and restore operate on immutable digest-pinned inputs.
- The architecture can prove completeness mechanically through exact binding
  and zero-specificity tests.

### Costs

- Manifest v3, closure persistence, runtime loading, activation state, and
  first-party package inventory must migrate together in staged vertical
  slices.
- Current Slack code must move out of composition, not merely be wrapped there.
- Generic ingress, OAuth, connection, target, and delivery coordinators need
  deeper interfaces than the present first Slack slice.
- Active-generation draining and shared-provider ownership introduce explicit
  concurrency and version-conflict rules.

### Compatibility

- v2 inline manifests are accepted only by a compatibility compiler and are
  normalized into the same resolved model.
- Legacy Slack split IDs, state roots, callback aliases, target encodings, and
  connection records are migrated by versioned, idempotent migration code.
- Compatibility shims may read old state or accept old callback aliases during
  a bounded window; they may not remain alternate runtime implementations.

## Rejected alternatives

### Expand `ProductAdapter` into one cross-capability trait

Rejected because it couples unrelated subsystem protocols, creates a God
interface, and encourages implementations to repeat manifest authority.

### Keep one physical manifest file

Rejected because it makes multi-surface packages difficult to own and review.
Logical singularity does not require physical inlining.

### Allow generic recursive includes or deep merge

Rejected because ordering, overrides, provenance, conflict behavior, digest
semantics, and security become implicit. Host-API contracts already know how to
validate and aggregate their own typed declarations.

### Register each tool, channel, and auth provider independently

Rejected because it recreates the retired split-product taxonomy and permits
partially activated extensions.

### Keep concrete Slack wiring in composition behind helper functions

Rejected because the host would still reason about one channel and adding the
next channel would require another concrete branch. The implementation must
move behind the same interfaces every extension uses.

### Trust runtime-reported capabilities

Rejected because runtime code could widen authority after install review.
Bindings implement an already validated contract; they never author it.

## Enforcement

This decision is complete only when all acceptance items in the related design
are checked, the caller-level integration flows pass, and architecture tests
prove that generic production crates contain no concrete extension/channel/
provider wiring outside explicitly bounded migration code.
