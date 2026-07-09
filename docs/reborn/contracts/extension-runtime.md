# Extension Runtime Contract

**Status:** Target normative contract

**Detailed design:** `docs/superpowers/specs/2026-07-09-unified-extension-runtime-design.md`

**Verification ledger:** `docs/reborn/extension-runtime-verification.md`

This contract defines the invariants the Reborn extension runtime must enforce.
“Must” and “must not” are normative.

## 1. Product model

1. Extension is the only installable product object.
2. One `ExtensionId` owns all of its tool, channel, auth, trigger, and file
   surfaces.
3. Capability-surface kind is product taxonomy. Runtime kind is an
   implementation/loading choice only.
4. `ProviderId` identifies a credential authority referenced by an auth
   surface. It must not identify an installed product.
5. Package ID must not be used as an implicit channel-surface ID.

## 2. Manifest authority

1. An extension package must contain exactly one installable root manifest.
2. The root may explicitly import typed leaf fragments, producing one logical
   manifest compilation unit.
3. A fragment must not be independently installable, trusted, activated, or
   addressed.
4. The root exclusively owns identity, version, trust request, runtime,
   dependencies, host-API membership, and fragment membership/order.
5. Imports must be explicit, package-relative, non-recursive, bounded, and
   containment checked. Globs, URLs, absolute paths, parent traversal,
   backslashes, symlink escape, duplicate paths, and nested imports must fail.
6. One host-API reference must use either one inline section or fragments, never
   both.
7. The generic compiler must not deep-merge or override TOML. The host-API
   contract owns typed fragment validation and aggregation.
8. One invalid leaf must invalidate the entire extension contract. No partial
   surface may publish.
9. v3 static tool declarations must have one typed leaf per capability. v3 auth
   requirements must resolve to an explicit auth surface or digest-pinned
   provider dependency.
10. Runtime-discovered tools must be represented by an explicit dynamic-provider
    Tool source group with host-enforced ceilings, not a sixth surface kind.
11. v3 hooks must resolve to typed hook declarations and activate atomically;
    v3 System runtime must be rejected.

## 3. Resolution and integrity

1. The compiler must emit one immutable `ResolvedExtensionManifest`, source
   map, closure snapshot, package digest, and contract digest.
2. Runtime, lifecycle, trust, discovery, workflow, auth, and frontend projection
   must consume the resolved record. They must not reparse raw root/fragment
   TOML.
3. The persisted closure must contain root and ordered fragment bytes so restart
   does not read mutable package sources.
4. `PackageDigest` must cover every immutable named package file and the
   dependency lock.
5. `ContractDigest` must cover canonical resolved authority and be insensitive
   to source whitespace/comments.
6. Digest framing must be domain separated, versioned, length prefixed, and
   ambiguity safe.
7. Package mutation must revalidate package signature/trust. Contract authority
   widening must require renewed approval.
8. Compiler/runtime must open the same immutable indexed package snapshot;
   manifest-only persistence is insufficient.
9. Content-addressed package storage must retain every indexed byte,
   authenticity/dependency data, generation leases, rollback refs, and
   crash-safe GC on both databases.

## 4. Runtime binding

1. Each runtime loader must return one `ExtensionEntrypoint`.
2. The entrypoint must return implementations keyed by resolved `SurfaceKey`.
3. Operational behavior must use narrow tool/channel/auth/trigger/file adapter
   interfaces; the entrypoint must not become a cross-capability God trait.
4. Runtime implementations must not redeclare IDs, schemas, effects,
   permissions, scopes, routes, directions, credentials, egress, host ports, or
   trust.
5. `BoundExtension::try_new` must be the only manifest-to-runtime join.
6. Binding must reject missing, extra, duplicate, wrong-kind, wrong-direction,
   wrong-owner, ABI-incompatible, conflict-producing, or authority-widening
   implementations.
7. A runtime adapter must be reachable only through a bound handle containing
   its resolved contract, installation context, generation, and effective
   policy.
8. A channel implementation must expose only the ingress, outbound,
   connection, target, and action sub-adapters required by its resolved channel
   declaration.
9. Runtime loading, construction, and binding must be side-effect-free and must
   receive no authority-bearing host ports.
10. Loader-issued provenance must bind each implementation to package,
    dependency/export, and ABI; an extension cannot mint it.
11. Local join validates keys/kinds/directions/provenance/ABI. Active-set
    construction validates global conflicts. Scoped ports enforce runtime
    authority.
12. Trigger/File are reserved unsupported and must not expose runtime bindings
    in this ABI.
13. Native first-party code is TCB and architecturally constrained; only
    sandbox runtimes receive hard authority isolation.

## 5. Host and adapter responsibilities

### Host must own

- package verification and effective trust;
- authorization, approvals, obligations, and resource policy;
- route/body/rate/concurrency/candidate limits;
- signature recipe execution, freshness, replay defense, and sealed evidence;
- secret storage and credential injection;
- OAuth state, CSRF, PKCE, replay, flow/account storage, and callback scope;
- tenant/caller/installation scope and generic identity/conversation binding;
- idempotency, target authorization, delivery attempts, retry, and drain;
- lifecycle, persistence, audit, and active snapshot publication.

### Extension adapters must own

- vendor payload and response parsing;
- untrusted protocol installation hints;
- challenge/ack semantics;
- external actor/conversation/event normalization;
- vendor target formats and provisioning;
- outbound rendering, multipart/vendor API semantics, and safe error mapping;
- provider endpoints/parameter quirks/token parsing/refresh/revoke/identity
  extraction;
- tool behavior.

Adapters may exercise host authority only through scoped ports derived from the
resolved contract and effective host policy.

## 6. Tool contract

1. Dispatch must resolve a prebound `ToolAdapter` by capability ID.
2. Generic dispatch must retain authorization, approval, obligation, resource,
   credential, event, and audit behavior.
3. Generic dispatch must not select a package/runtime implementation by a
   concrete extension or provider at invocation time.
4. An explicit send-message tool is a tool side effect and must not be used to
   deliver final replies.

## 7. Channel ingress contract

1. One generic host router must match manifest-declared active route entries.
2. The host must enforce route policy and bounds before adapter parsing.
3. Adapter inspection may emit only untrusted hints.
4. The host must intersect hints with host-owned candidates and enforce a
   verification budget.
5. The host must execute declarative verification without exposing signing
   secret bytes to the adapter.
6. Only sealed verified input may reach `ingest_verified`.
7. Adapter output must be a normalized message/no-op/bounded immediate response.
8. Product workflow must own identity, conversation, dedupe, admission, and turn
   submission.
9. Extensions must not mount arbitrary Axum routers.
10. Routes must use the reserved canonical extension-webhook namespace and must
    not collide with fixed host routes.
11. Connection-created opaque routing claims must be host-indexed/scoped and
    hint/candidate/parser-group work must be bounded.
12. Normal/no-op 2xx acknowledgement requires a durable dedupe/admission commit;
    persistence failure returns retryable failure.
13. Protocol reply metadata must be a bounded opaque seed that the host signs,
    scopes, persists, and generation-leases.

## 8. Channel outbound contract

1. Final replies, progress, gate/auth prompts, failures, connection notices,
   busy/working state, trigger delivery, and cleanup must enter one generic
   semantic delivery coordinator.
2. Outbound policy must validate target/preferences/privacy and persist an
   attempt before vendor egress.
3. The bound channel adapter must own vendor rendering/send/update/delete/
   multipart semantics.
4. Restricted host egress must enforce declared authority and inject secrets.
5. The generic coordinator is the sole durable delivery-state writer; the
   adapter returns structured part/protocol outcomes and receives no store.
6. No direct product send path may bypass the coordinator.
7. A crash after vendor egress and before result persistence must become
   `Unknown`; blind retry is allowed only with a tested vendor idempotency key,
   otherwise reconcile or terminate unknown.

## 9. Auth contract

1. One generic start/status/callback/revoke route family must resolve a bound
   auth adapter.
2. Requests must not carry provider identity for a runtime string switch after
   binding.
3. Host-owned state must bind caller, extension, installation, auth surface,
   provider contract/digest, scopes, TTL, and continuation.
4. Adapter plans/requests must remain inside manifest host/scope/credential
   ceilings.
5. The host must store encrypted normalized grants and validate identity claims.
6. No provider-specific callback handler or core provider switch may remain.
7. Manual-secret auth is an explicit host-managed surface; optional remote
   validation requires a declared validator binding.
8. Adapter authorization plans cannot override host-reserved OAuth parameters
   or issuer binding.

## 10. Connection, target, and action contract

1. Connection/status/begin/complete/disconnect must resolve by full surface key.
2. Target metadata must be opaque, signed, versioned, size bounded, and scoped
   by host-owned claims.
3. Protocol target IDs/provisioning must remain adapter-owned.
4. Protected setup/configuration must use fixed generic host routes and
   manifest-declared schemas/actions.
5. The host must authenticate, authorize, validate, persist secrets, and audit
   before invoking an adapter action.
6. Frontend behavior must derive from generic surface/action DTOs, never package
   ID branches.

## 11. Lifecycle contract

1. `ExtensionHost` must be the only active-set writer.
2. Activation must stage and validate a complete next generation before any
   durable/live publication.
3. Durable activation state must use revision compare-and-swap and include
   generation/package/contract/trust digests.
4. Live publication must be one immutable snapshot swap.
5. Failure before CAS must leave durable and live state unchanged.
6. Startup must rebuild enabled generations and publish once; invalid
   installations must quarantine without partial surfaces.
7. Upgrade/deactivate/remove must reject new work, swap atomically, and drain
   old `Arc` generations safely.
8. Rollback must use a persisted immutable prior package/manifest snapshot.
9. Persistent behavior must support libSQL and PostgreSQL.
10. Resumable auth/delivery/ingress/cleanup/target work must pin exact package,
    contract, dependencies, generation, and ABI until terminal/TTL.
11. The first implementation must enforce one extension-serving leader/fencing
    lease per deployment/tenant partition; it must not imply process-local
    snapshot swaps are multi-replica atomic.

## 12. Generic-core prohibition

Outside concrete extension/provider crates, versioned migrations, tests,
fixtures, docs, and generated catalog data, production generic code must not:

- contain a concrete extension/channel/provider literal in control flow;
- import a concrete adapter/provider crate;
- construct a concrete adapter;
- mount a concrete route;
- expose a concrete compile feature/config type;
- parse a concrete protocol payload;
- call a concrete vendor API;
- render a concrete product UI component.

Permanent architecture tests must enforce both source and dependency
directions. Generic host must compile/test with Slack absent, and Slack plus
Telegram must run through the same production interfaces.
