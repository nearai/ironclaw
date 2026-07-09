# Unified Extension Runtime — Verification Ledger

**Baseline:** `900d435ee4d8496fb0d711fcf2f52807f1d414d3`

**Design:** `docs/superpowers/specs/2026-07-09-unified-extension-runtime-design.md`

**Plan:** `docs/superpowers/plans/2026-07-09-unified-extension-runtime-implementation-plan.md`

**Contract:** `docs/reborn/contracts/extension-runtime.md`

This is the release gate for the unified extension runtime. It is not a list of
aspirations. The implementation agent must check an item only after recording
specific evidence in the final traceability table or next to the item.

## 1. Evidence rules

Acceptable evidence:

- an exact unit/contract test name for a pure invariant;
- a caller-level test that drives the real dispatcher, route, manager, or store
  for behavior that gates a side effect;
- a libSQL and PostgreSQL result for persistent behavior;
- a frontend component/integration/E2E test for rendered behavior;
- an architecture source/dependency test for absence/prohibition;
- an exact command result for workspace-wide quality gates;
- an inspected migration fixture and round-trip test for compatibility;
- a safe operator/manual observation only when no deterministic test can reach
  the behavior, with the reason recorded.

Not sufficient:

- “code looks generic”;
- a helper unit test when a wrapper computes different inputs;
- `wait_for_status(Completed)` without the required side-effect assertion;
- one database backend for a shared persistent contract;
- a test-only registry not used by production;
- docs claiming a path was removed;
- an allowlist that hides an entire generic directory;
- `cargo test --workspace --all-features` as proof an external PostgreSQL lane
  actually ran.

Status convention:

- `[ ]` not proven;
- `[x]` proven and evidence recorded;
- `[!]` temporarily superseded by an approved ADR while this ledger is being
  updated. It is never a final state: the ADR must update/remove the old
  requirement, and release mode permits only `[x]` atomic items.

## 2. Release blockers

- [ ] **REL-001** Every required atomic item below is `[x]`; none is pending,
  blocked, or superseded.
- [ ] **REL-002** No production caller remains on the pre-binding extension,
  channel, tool, or auth path.
- [ ] **REL-003** Temporary concrete-product source allowlist is empty.
- [ ] **REL-004** Generic crates build/test with Slack absent.
- [ ] **REL-005** Slack and Telegram use the same production binding interfaces.
- [ ] **REL-006** The arbitrary fixture channel completes backend + frontend
  connect/inbound/outbound flows without product-specific source changes.
- [ ] **REL-007** libSQL and PostgreSQL durable lanes pass.
- [ ] **REL-008** Full workspace tests and clippy pass with zero warnings.
- [ ] **REL-009** All migration fixtures pass first run, restart, second run,
  malformed-record, and cleanup-version cases.
- [ ] **REL-010** `FEATURE_PARITY.md`, contract docs, skill map, and changelog
  match shipped behavior.

---

## 3. Product/domain model

- [ ] **MODEL-001** Extension is the only installable product record.
- [ ] **MODEL-002** Tool/channel/auth/trigger/file cannot be installed,
  trusted, enabled, disabled, upgraded, or removed independently.
- [ ] **MODEL-003** One `ExtensionId` owns all surfaces in the resolved contract.
- [ ] **MODEL-004** Runtime kind cannot affect surface kind/discovery.
- [ ] **MODEL-005** `ProviderId` cannot be used where `ExtensionId` is required.
- [ ] **MODEL-006** Package ID is not an implicit channel ID.
- [ ] **MODEL-007** Multiple channel surfaces in one extension remain distinct
  through lifecycle, wire, connection, target, ingress, and outbound flows.
- [ ] **MODEL-008** Every durable surface reference includes full `SurfaceKey`.
- [ ] **MODEL-009** Tool surface keys validate capability prefix against owner.
- [ ] **MODEL-010** Trigger/File remain reserved typed kinds without accidental
  runtime publication.
- [ ] **MODEL-011** Product vocabulary contains no retired `slack_bot`,
  `slack_personal`, channel-as-product, or tool-as-product identity.
- [ ] **MODEL-012** Shared provider implementation dependencies do not appear as
  installable products.

Suggested evidence: domain type contract tests, retired-taxonomy architecture
test, lifecycle API tests, multi-channel arbitrary fixture test.

## 4. Root manifest and fragment compilation

### Source-of-truth behavior

- [ ] **MAN-001** Exactly one installable root `manifest.toml` exists per package.
- [ ] **MAN-002** Root explicitly owns every fragment path and declaration order.
- [ ] **MAN-003** Unimported package leaf grants no surface/authority.
- [ ] **MAN-004** Fragment is not independently parseable as an extension.
- [ ] **MAN-005** Runtime/lifecycle/trust/frontend consume one resolved record,
  not individual source files.
- [ ] **MAN-006** One invalid leaf rejects the entire extension atomically.
- [ ] **MAN-007** No partial surfaces publish after compile failure.
- [ ] **MAN-008** v2 inline input normalizes to the same resolved domain types.
- [ ] **MAN-009** v3 rejects inline section plus fragments for one host-API ref.
- [ ] **MAN-010** v3 does not perform generic deep merge/override/last-wins.

### Path/envelope security

- [ ] **MAN-011** Empty path rejected.
- [ ] **MAN-012** Absolute path rejected.
- [ ] **MAN-013** URL/URI path rejected.
- [ ] **MAN-014** Windows drive path rejected.
- [ ] **MAN-015** Backslash path rejected.
- [ ] **MAN-016** NUL/control path rejected.
- [ ] **MAN-017** Empty, `.`, and `..` segments rejected.
- [ ] **MAN-018** Glob/wildcard rejected.
- [ ] **MAN-019** Duplicate normalized path within/across sections rejected.
- [ ] **MAN-020** Symlink/mount/cross-package escape rejected.
- [ ] **MAN-021** Fragment-to-fragment import/nested depth rejected.
- [ ] **MAN-022** Missing fragment reports package-relative path.
- [ ] **MAN-023** Non-UTF-8 fragment rejected safely.
- [ ] **MAN-024** Non-table/empty body rejected.
- [ ] **MAN-025** Wrong fragment schema rejected.
- [ ] **MAN-026** Wrong/unsupported fragment kind rejected by owning contract.
- [ ] **MAN-027** Unknown fragment envelope field rejected.
- [ ] **MAN-028** Fragment root identity/trust/runtime/host-API/import fields
  rejected.
- [ ] **MAN-029** Diagnostics include safe package-relative line/column.
- [ ] **MAN-030** Diagnostics omit host absolute path/body/secret.

### Bounds

- [ ] **MAN-031** Root >256 KiB rejected before unbounded parse/allocation.
- [ ] **MAN-032** Leaf >64 KiB rejected before unbounded parse/allocation.
- [ ] **MAN-033** More than 512 leaves rejected before surplus reads.
- [ ] **MAN-034** Closure >2 MiB rejected before unbounded accumulation.
- [ ] **MAN-035** Import depth is exactly one.
- [ ] **MAN-036** Existing total extension-count discovery bound still prevents
  surplus package reads.

### Contract aggregation

- [ ] **MAN-037** Generic compiler knows envelope/path/provenance, not Slack or
  host-API body fields.
- [ ] **MAN-038** Capability-provider v2 preserves leaf order.
- [ ] **MAN-039** One static capability leaf yields exactly one capability and
  Tool surface.
- [ ] **MAN-040** Duplicate static capability ID fails globally.
- [ ] **MAN-041** Channel contract accepts exactly one channel leaf per host-API
  instance.
- [ ] **MAN-042** Auth contract accepts exactly one auth leaf per host-API
  instance.
- [ ] **MAN-043** Multiple channels/auth surfaces require multiple explicit refs.
- [ ] **MAN-044** v3 product-auth credential reference resolves to explicit auth
  surface or pinned dependency.
- [ ] **MAN-045** Dangling/wrong auth-surface reference fails.
- [ ] **MAN-046** Existing channel credential/egress/route coherence validation
  remains effective on fragments.
- [ ] **MAN-047** Asset refs remain package-root-relative.
- [ ] **MAN-048** Unknown/unreferenced operational sections fail.
- [ ] **MAN-049** Channel/auth root section local name matches fragment body ID.
- [ ] **MAN-050** v3 hooks resolve to typed `HookManifestEntry` without raw TOML
  reprojection and participate in contract/package digests.
- [ ] **MAN-051** Hook install/uninstall is atomic with extension generation.
- [ ] **MAN-052** v3 rejects System runtime; historical v2 System records are
  explicitly migrated/rejected.

Primary evidence target:
`ironclaw_extensions/tests/manifest_fragment_contract.rs`, discovery caller
tests, and product-adapter/channel/auth contract ingestion tests.

## 5. Digests, package inventory, and persistence

- [ ] **DIG-001** Closure digest uses versioned domain-separated framing.
- [ ] **DIG-002** Package digest uses normalized path+length+bytes framing.
- [ ] **DIG-003** Contract digest uses versioned canonical typed DTO.
- [ ] **DIG-004** Concatenation/path ambiguity test passes.
- [ ] **DIG-005** Fragment byte mutation changes closure/package digest.
- [ ] **DIG-006** Runtime artifact mutation changes package digest.
- [ ] **DIG-007** Schema/prompt/localization/asset mutation changes package digest.
- [ ] **DIG-008** Whitespace/comment-only manifest edit leaves contract digest.
- [ ] **DIG-009** Semantic authority edit changes contract digest.
- [ ] **DIG-010** Semantically ordered import reorder changes contract digest.
- [ ] **DIG-011** Unordered set/map ordering is canonical.
- [ ] **DIG-012** Golden canonical bytes are stable.
- [ ] **DIG-013** Package signature/trust consumes full package digest.
- [ ] **DIG-014** Dependency lock is covered by package integrity.
- [ ] **DIG-015** Stored digest mismatch fails closed on reopen.
- [ ] **DIG-016** Resolved record persists root + ordered leaf bytes.
- [ ] **DIG-017** Resolved record persists typed contract + source map + digests.
- [ ] **DIG-018** Restart succeeds with mutable package source unavailable.
- [ ] **DIG-019** No production domain projection reparses raw root/leaf TOML.
- [ ] **DIG-020** Legacy raw-root record migrates to versioned root-only closure.
- [ ] **DIG-021** Record constructor, not caller, computes authoritative digests.
- [ ] **DIG-022** Byte-only change revalidates signature/reloads code.
- [ ] **DIG-023** Authority widening requires approval; no unconditional bundled
  hash migration.
- [ ] **DIG-024** Narrowing/equivalent delta follows explicit tested policy.
- [ ] **DIG-025** Generated first-party package inventory is sorted,
  deterministic, symlink-safe, and excludes source-only files.
- [ ] **DIG-026** Every imported leaf is packaged.
- [ ] **DIG-027** Every packaged `manifests/**/*.toml` leaf is imported exactly
  once.
- [ ] **DIG-028** CI path filters run relevant lanes for fragment edits.
- [ ] **DIG-029** Package index rejects missing/duplicate/unlisted payload files.
- [ ] **DIG-030** Detached signature exclusion/signing tuple avoids circularity
  and covers identity/version/package digest.

## 6. Durable package store and authenticity

- [ ] **PKG-001** Compiler consumes one immutable indexed package snapshot.
- [ ] **PKG-002** Stage streams and bounds files before visibility.
- [ ] **PKG-003** Failed stage leaves no install/package record.
- [ ] **PKG-004** Commit atomically publishes blob metadata and manifest/install
  reference with revision CAS.
- [ ] **PKG-005** Open returns exact content-addressed bytes.
- [ ] **PKG-006** Missing/corrupt blob quarantines; no mutable-source refetch.
- [ ] **PKG-007** File count/path/per-file/runtime/aggregate/archive/ratio limits
  are enforced while streaming.
- [ ] **PKG-008** Host-bundled catalog attestation verifies.
- [ ] **PKG-009** Detached Ed25519 registry signature verifies against host trust
  store and rejects wrong key/message/signature.
- [ ] **PKG-010** Local unsigned package remains sandboxed and approval-gated.
- [ ] **PKG-011** Active generation lease pins root/dependency blobs.
- [ ] **PKG-012** Auth/delivery/ingress/cleanup/target/rollback leases pin exact
  package/contract/dependency/ABI.
- [ ] **PKG-013** Terminal/TTL paths release leases safely.
- [ ] **PKG-014** GC skips installed/active/pending/rollback/leased blobs.
- [ ] **PKG-015** GC is crash-idempotent and quota-aware.
- [ ] **PKG-016** Old root-only hash maps only through known bundled catalog or
  requires rematerialization/reapproval; no fabricated digest.
- [ ] **PKG-017** libSQL package store/lease/GC contract passes.
- [ ] **PKG-018** PostgreSQL package store/lease/GC contract passes.
- [ ] **PKG-019** Ed25519 message/wire validates key-ID/base64/length and rejects
  unknown/revoked/expired/wrong-source signers.

## 7. First-party manifest parity

- [ ] **PAR-001** All 11 bundled roots use v3 logical compilation units.
- [ ] **PAR-002** All 137 static capability declarations moved one-per-leaf.
- [ ] **PAR-003** Public capability IDs unchanged exactly.
- [ ] **PAR-004** Descriptions unchanged unless separately reviewed.
- [ ] **PAR-005** Effects/default permissions/visibility unchanged exactly.
- [ ] **PAR-006** Schema/prompt refs unchanged exactly.
- [ ] **PAR-007** Required host ports/resource profiles unchanged exactly.
- [ ] **PAR-008** Credential handles/sources/scopes/audiences/targets unchanged
  exactly unless the auth design intentionally normalizes them with approved
  snapshot update.
- [ ] **PAR-009** Derived Tool/Auth/Channel surfaces match expected migration.
- [ ] **PAR-010** Slack resolves exactly 5 Tool + 1 Channel + 1 Auth.
- [ ] **PAR-011** Slack channel directions/route/credentials/egress preserved.
- [ ] **PAR-012** Hosted MCP uses dynamic-provider declaration, not fake static
  leaves for live tools.
- [ ] **PAR-013** Exact semantic snapshot tests compare fields, not counts only.
- [ ] **PAR-014** Auth migration matrix proves 63 Google OAuth, 18 Notion OAuth,
  5 Slack OAuth, 47 GitHub manual, and 1 NEAR AI manual references resolve.
- [ ] **PAR-015** Real Telegram v3 package resolves 1 Channel + 1 host-managed
  manual Auth and activates through the same host.

## 8. Entrypoint, binding, and loaders

- [ ] **BIND-001** Every runtime technology loads one `ExtensionEntrypoint`.
- [ ] **BIND-002** Entrypoint returns a private-construction binding map keyed by
  full `SurfaceKey`.
- [ ] **BIND-003** Duplicate binding rejected during map construction.
- [ ] **BIND-004** Missing expected binding rejects extension.
- [ ] **BIND-005** Unexpected binding rejects extension.
- [ ] **BIND-006** Wrong implementation variant rejects extension.
- [ ] **BIND-007** Missing declared channel direction/action sub-adapter rejects.
- [ ] **BIND-008** Extra undeclared channel direction/action sub-adapter rejects.
- [ ] **BIND-009** Root/dependency owner mismatch rejects.
- [ ] **BIND-010** Dependency version/digest/export mismatch rejects.
- [ ] **BIND-011** Runtime ABI/interface version mismatch rejects.
- [ ] **BIND-012** Binding cannot request undeclared host port.
- [ ] **BIND-013** Binding cannot request undeclared credential handle.
- [ ] **BIND-014** Binding cannot widen route/egress/effect/scope/direction.
- [ ] **BIND-015** Duplicate active capability conflict rejects publication.
- [ ] **BIND-016** Duplicate active ingress route conflict rejects publication.
- [ ] **BIND-017** Incompatible provider export/digest conflict rejects.
- [ ] **BIND-018** `BoundExtension::try_new` is the only join constructor.
- [ ] **BIND-019** No raw unbound adapter is exposed to runtime callers.
- [ ] **BIND-020** Adapter traits contain no manifest-authority metadata getters.
- [ ] **BIND-021** One entrypoint returns narrow traits; no cross-capability God
  operational trait exists.
- [ ] **BIND-022** Native loader accepts only host-bundled/trust-approved packages.
- [ ] **BIND-023** Unknown native service fails safely.
- [ ] **BIND-024** WASM loader enforces world/ABI/memory/deadline/concurrency bounds.
- [ ] **BIND-025** Generic extension host has no concrete extension dependency.
- [ ] **BIND-026** Generated first-party catalog is data-driven; no handwritten
  product switch in composition.
- [ ] **BIND-027** Resolver lookup includes tenant/caller/agent/project scope and
  authorized explicit installation selection.
- [ ] **BIND-028** Same capability in different scopes is allowed; ambiguity in
  one scope fails.
- [ ] **BIND-029** Load/construction/bind are side-effect-free and receive no
  authority-bearing ports.
- [ ] **BIND-030** Loader-issued origin provenance cannot be minted/altered by
  extension code.
- [ ] **BIND-031** Local join validates set/kind/direction/provenance/ABI only;
  global conflicts are active-set validation.
- [ ] **BIND-032** Optional readiness runs after local join through read-only
  bounded ports and before activation CAS.
- [ ] **BIND-033** Trigger/File remain reserved unsupported and cannot bind.
- [ ] **BIND-034** Dynamic provider is an internal Tool source group, not a sixth
  capability surface kind.
- [ ] **BIND-035** Native first-party code is explicitly TCB; hard isolation is
  claimed only for sandbox runtimes.

## 9. Active generation and lifecycle

- [ ] **LIFE-001** `ExtensionHost` is the only active-snapshot writer.
- [ ] **LIFE-002** Active snapshot is immutable to readers.
- [ ] **LIFE-003** Load failure leaves durable/live state unchanged.
- [ ] **LIFE-004** Bind failure leaves durable/live state unchanged.
- [ ] **LIFE-005** Readiness failure leaves durable/live state unchanged.
- [ ] **LIFE-006** Global conflict leaves durable/live state unchanged.
- [ ] **LIFE-007** Store/CAS failure leaves live state unchanged.
- [ ] **LIFE-008** Successful activation publishes all expected resolver views
  in one generation.
- [ ] **LIFE-009** Activation increments generation/revision exactly once.
- [ ] **LIFE-010** Concurrent stale activation loses CAS without partial state.
- [ ] **LIFE-011** Durable commit includes tenant/install/generation/package/
  contract/trust decision digests.
- [ ] **LIFE-012** Crash after CAS/before swap restores committed generation once.
- [ ] **LIFE-013** Startup stages all enabled installs before one initial publish.
- [ ] **LIFE-014** Invalid independent install quarantines without partial surfaces.
- [ ] **LIFE-015** Shared-dependency conflict fails/quarantines deterministically.
- [ ] **LIFE-016** Upgrade publishes complete new generation atomically.
- [ ] **LIFE-017** In-flight work completes against retained old generation `Arc`.
- [ ] **LIFE-018** New work after swap resolves only new generation.
- [ ] **LIFE-019** Deactivate rejects new work and drains old work.
- [ ] **LIFE-020** Drain deadline/cancellation/unresolved record behavior tested.
- [ ] **LIFE-021** Remove cleans generic auth/connection/identity/target/state in
  specified idempotent order.
- [ ] **LIFE-022** Partial remove/cleanup is durable retryable state, not success.
- [ ] **LIFE-023** Rollback uses persisted prior immutable snapshot.
- [ ] **LIFE-024** Byte-equivalent/narrowing/widening authority delta paths tested.
- [ ] **LIFE-025** Approval denial leaves old generation active.
- [ ] **LIFE-026** Lifecycle/audit after-commit delivery is idempotent/outboxed or
  explicitly non-authoritative.
- [ ] **LIFE-027** Tenant is part of active indexes/resolver/route/store scope.
- [ ] **LIFE-028** Same surface/capability in different tenants coexists, while
  cross-tenant resolution never returns a handle.
- [ ] **LIFE-029** libSQL lifecycle contract passes.
- [ ] **LIFE-030** PostgreSQL lifecycle contract passes.
- [ ] **LIFE-031** Resumable work acquires exact generation lease transactionally.
- [ ] **LIFE-032** Restart rehydrates exact leased generation, never latest.
- [ ] **LIFE-033** Serving-leader lease/fencing permits one extension-serving
  process per partition and rejects stale holder.
- [ ] **LIFE-034** PostgreSQL two-host lease contention/failover passes.
- [ ] **LIFE-035** Nonholder reports not-ready and cannot serve/mutate extensions.

## 10. Tool dispatch

- [ ] **TOOL-001** Dispatcher resolves prebound tool by `CapabilityId`.
- [ ] **TOOL-002** Dispatcher no longer loads package/selects runtime kind per
  invocation.
- [ ] **TOOL-003** Unknown capability fails before adapter work.
- [ ] **TOOL-004** Authorization still runs through actual caller.
- [ ] **TOOL-005** Approval/lease/obligation behavior still runs.
- [ ] **TOOL-006** Resource reservation/governor behavior still runs.
- [ ] **TOOL-007** Credential requirement/gate/injection uses resolved contract.
- [ ] **TOOL-008** Adapter cannot access undeclared credential/egress/host port.
- [ ] **TOOL-009** Runtime result/events/audit remain caller-visible.
- [ ] **TOOL-010** WASM lane works as `ToolAdapter`.
- [ ] **TOOL-011** MCP lane works as `ToolAdapter`/dynamic provider.
- [ ] **TOOL-012** Script/native lanes work behind same interface where supported.
- [ ] **TOOL-013** Slack five tools activate/invoke through generic dispatcher.
- [ ] **TOOL-014** `slack.send_message` remains explicit tool, never final-reply
  delivery shortcut.
- [ ] **TOOL-015** Dynamic child tool cannot exceed provider ceiling.

## 11. Runtime-discovered tool providers

- [ ] **DYN-001** Manifest explicitly declares one dynamic provider surface.
- [ ] **DYN-002** Exact binding is at provider-surface level.
- [ ] **DYN-003** Discovered child names remain inside declared namespace.
- [ ] **DYN-004** Tool count ceiling enforced.
- [ ] **DYN-005** Input/output schema size/shape ceiling enforced.
- [ ] **DYN-006** Effect ceiling enforced conservatively.
- [ ] **DYN-007** Credential/host-port/egress ceiling enforced.
- [ ] **DYN-008** Invalid child rejects refresh without partial publication.
- [ ] **DYN-009** Discovery generation/digest is immutable and observable.
- [ ] **DYN-010** Refresh atomically replaces complete child set.
- [ ] **DYN-011** Runtime invocation still runs normal dispatcher policy.
- [ ] **DYN-012** Existing MCP annotation conservatism preserved.

## 12. Channel ingress

### Generic router/policy

- [ ] **ING-001** One generic router/table mounts active manifest routes.
- [ ] **ING-002** Route activation rejects method/path conflicts.
- [ ] **ING-003** Method mismatch rejected before adapter work.
- [ ] **ING-004** Body limit enforced before adapter work.
- [ ] **ING-005** Rate limit enforced before adapter work.
- [ ] **ING-006** Candidate limit enforced before expensive verification.
- [ ] **ING-007** Deadline and shutdown drain enforced.
- [ ] **ING-008** CORS/origin/stream/audit/listener policy preserved.
- [ ] **ING-009** No arbitrary extension-owned Axum router is mountable.

### Hints and verification

- [ ] **ING-010** Adapter inspection sees bounded input only.
- [ ] **ING-011** Inspection hints are typed untrusted data.
- [ ] **ING-012** Hints cannot establish tenant/actor/install/identity/trust.
- [ ] **ING-013** Host intersects hints with host-owned candidate scope.
- [ ] **ING-014** HMAC recipe exact byte construction tested.
- [ ] **ING-015** Constant-time signature comparison used.
- [ ] **ING-016** Missing/bad signature rejected.
- [ ] **ING-017** Stale timestamp rejected.
- [ ] **ING-018** Replay rejected.
- [ ] **ING-019** Ambiguous verified installation rejected.
- [ ] **ING-020** Sealed evidence can be minted only by host verifier.
- [ ] **ING-021** Signing secret bytes never reach adapter recorder/log.

### Adapter/workflow outcome

- [ ] **ING-022** Challenge/URL verification returns bounded immediate response.
- [ ] **ING-023** Authenticated ignored event returns explicit no-op.
- [ ] **ING-024** Normal event maps actor/conversation/event/message/attachments.
- [ ] **ING-025** Generic workflow performs identity/conversation binding.
- [ ] **ING-026** Generic workflow performs idempotency/admission/turn submission.
- [ ] **ING-027** Immediate ack plus async failure semantics are explicitly tested.
- [ ] **ING-028** Slack real package passes mounted-route caller test.
- [ ] **ING-029** Telegram passes same mounted-route caller test.
- [ ] **ING-030** Arbitrary fixture passes without source branch.
- [ ] **ING-031** No Slack envelope/route/signature type remains in generic code.
- [ ] **ING-032** Verifier segment/literal/body/header ceilings enforced.
- [ ] **ING-033** Immediate response status/header/body/deadline ceilings enforced.
- [ ] **ING-034** Identical route descriptors may group installations; an
  incompatible descriptor at the same method/path fails activation.
- [ ] **ING-035** Canonical extension-webhook namespace/one-segment suffix and
  percent-decoding grammar reject protected/catch-all/ambiguous routes.
- [ ] **ING-036** Activation checks collisions against all fixed host routes.
- [ ] **ING-037** Header count/bytes and duplicate security-header limits hold.
- [ ] **ING-038** Adapter inspection panic/CPU/deadline isolation holds.
- [ ] **ING-039** Connection completion stores bounded HMAC-indexed routing
  claims scoped by surface/install/parser compatibility.
- [ ] **ING-040** No-hint/multi-parser-group matching follows 4-group/10-ms/
  32-candidate total budget and ambiguity rules.
- [ ] **ING-041** Normal/no-op 2xx occurs only after durable dedupe/admission
  commit; persistence failure returns retryable 5xx.
- [ ] **ING-042** Crash-before/after enqueue, concurrent replay, and restart
  replay converge exactly once on both DBs.
- [ ] **ING-043** Challenge-only response may skip enqueue only after verification.
- [ ] **ING-044** Opaque reply-target seed is bounded, signed, scoped, persisted,
  generation-leased, and returned only to a compatible adapter.
- [ ] **ING-045** Immediate response cannot redirect/set cookie/override security
  headers.

## 13. Channel outbound and delivery

### Semantic coordinator

- [ ] **OUT-001** Final reply enters generic coordinator.
- [ ] **OUT-002** Progress enters generic coordinator.
- [ ] **OUT-003** Gate prompt enters generic coordinator.
- [ ] **OUT-004** Auth prompt enters generic coordinator.
- [ ] **OUT-005** Failure notice enters generic coordinator.
- [ ] **OUT-006** Working/busy notice enters generic coordinator.
- [ ] **OUT-007** Connect-required notice enters generic coordinator.
- [ ] **OUT-008** Triggered delivery enters generic coordinator.
- [ ] **OUT-009** Cleanup/delete intent enters generic coordinator.
- [ ] **OUT-010** Capability activity/display behavior is explicit.
- [ ] **OUT-011** Source-route final reply policy preserved.
- [ ] **OUT-012** Trigger/preference policy preserved.
- [ ] **OUT-013** Unauthorized/unavailable/stale target fails closed.
- [ ] **OUT-014** Direct-message privacy is generic and enforced.
- [ ] **OUT-015** Delivery attempt persists before vendor egress.
- [ ] **OUT-016** Production construction cannot use no-op sink.

### Adapter and egress

- [ ] **OUT-017** Bound adapter owns protocol rendering.
- [ ] **OUT-018** Bound adapter owns vendor target formatting.
- [ ] **OUT-019** Bound adapter owns multipart/update/delete semantics.
- [ ] **OUT-020** Host restricted egress injects declared credential.
- [ ] **OUT-021** Adapter-supplied authorization header rejected.
- [ ] **OUT-022** Undeclared host rejected before network.
- [ ] **OUT-023** Wrong credential handle rejected before network.
- [ ] **OUT-024** Forbidden method/oversized body rejected.
- [ ] **OUT-025** Redirect/private-IP/DNS escape rejected.
- [ ] **OUT-026** Cross-tenant/install egress rejected.
- [ ] **OUT-027** Vendor response/body/error is bounded/redacted.

### Reliability

- [ ] **OUT-028** Coordinator alone persists delivery state; adapter receives no
  store/sink.
- [ ] **OUT-029** Retryable failure schedules bounded backoff.
- [ ] **OUT-030** Permanent failure does not retry.
- [ ] **OUT-031** Dedupe/idempotency prevents duplicate attempt.
- [ ] **OUT-032** Single-flight behavior is generic.
- [ ] **OUT-033** Partial multipart rule prevents duplicate prior parts.
- [ ] **OUT-034** Shutdown drain handles pending delivery.
- [ ] **OUT-035** Generation swap behavior for pending delivery is explicit.
- [ ] **OUT-036** Slack passes all semantic intent cases.
- [ ] **OUT-037** Telegram passes representative same-interface cases.
- [ ] **OUT-038** No direct Slack/vendor send remains in generic source.
- [ ] **OUT-039** Explicit Slack send tool remains separate from final delivery.
- [ ] **OUT-040** Prepared/Sending are persisted before vendor call.
- [ ] **OUT-041** Crash after possible vendor success becomes `Unknown`.
- [ ] **OUT-042** Unknown retries only with tested vendor idempotency key;
  otherwise reconcile or terminal unknown without blind resend.

## 14. Auth provider

### Generic flow/security

- [ ] **AUTH-001** One generic start route resolves full auth surface key.
- [ ] **AUTH-002** One generic status route is caller/surface scoped.
- [ ] **AUTH-003** One generic callback route handles providers.
- [ ] **AUTH-004** One generic revoke/disconnect route handles providers.
- [ ] **AUTH-005** Flow binds caller/tenant/extension/install/surface/provider
  contract digest/generation.
- [ ] **AUTH-006** TTL enforced.
- [ ] **AUTH-007** OAuth state/CSRF validated.
- [ ] **AUTH-008** PKCE validated as declared.
- [ ] **AUTH-009** Callback replay rejected.
- [ ] **AUTH-010** Cross-caller/install/tenant callback rejected.
- [ ] **AUTH-011** Requested scopes intersect capability need and manifest ceiling.
- [ ] **AUTH-012** Scope widening rejected before provider call.
- [ ] **AUTH-013** Redirect URI/host validated.
- [ ] **AUTH-014** Client secret remains host-side/restricted egress.
- [ ] **AUTH-015** Adapter provider plan remains inside manifest host ceiling.
- [ ] **AUTH-016** Adapter request has no provider selector for string switching.

### Provider behavior/storage

- [ ] **AUTH-017** Adapter owns endpoint/parameter quirks.
- [ ] **AUTH-018** Adapter owns token response parsing.
- [ ] **AUTH-019** Adapter owns refresh/revoke quirks.
- [ ] **AUTH-020** Adapter returns normalized identity claim.
- [ ] **AUTH-021** Host validates identity claim against flow/surface scope.
- [ ] **AUTH-022** Access/refresh secrets encrypted and stored atomically.
- [ ] **AUTH-023** Refresh rotation handles old/new secret safely.
- [ ] **AUTH-024** Revoke idempotency tested.
- [ ] **AUTH-025** Malformed/oversized provider response redacted.
- [ ] **AUTH-026** Flow consumes and continuation resumes exactly once.
- [ ] **AUTH-027** Implementation upgrade between begin/callback cannot silently
  switch adapter.
- [ ] **AUTH-028** Slack personal OAuth passes generic real-route flow.
- [ ] **AUTH-029** Missing Slack tool credential gates, authenticates, stores,
  resumes, and invokes through caller-level test.
- [ ] **AUTH-030** No `if provider == ...` or provider string multiplexor remains.
- [ ] **AUTH-031** libSQL auth state/account flow passes.
- [ ] **AUTH-032** PostgreSQL auth state/account flow passes.
- [ ] **AUTH-033** Host constructs and prevents duplicate/override of state,
  redirect_uri, PKCE, client_id, scope, response_type reserved parameters.
- [ ] **AUTH-034** Issuer/authorization-server mix-up defense is tested where
  issuer identity exists.
- [ ] **AUTH-035** Manual-secret auth surface stores encrypted fields with no
  runtime binding by default.
- [ ] **AUTH-036** Adapter remote validation is required only when declared and
  receives injected secret through restricted egress, not raw storage access.
- [ ] **AUTH-037** Google/Notion/Slack OAuth and GitHub/NEAR AI manual baseline
  references all resolve to explicit v3 auth ownership.

## 15. Connection, targets, configuration, and actions

### Connection

- [ ] **CONN-001** Status resolves by full surface key.
- [ ] **CONN-002** Begin/complete/disconnect resolve bound connection adapter.
- [ ] **CONN-003** Multiple channel surfaces maintain separate connection state.
- [ ] **CONN-004** Host owns authenticated caller and schema validation.
- [ ] **CONN-005** Host owns secret persistence/rollback.
- [ ] **CONN-006** Adapter owns vendor validation/connect completion.
- [ ] **CONN-007** Disconnect CAS/order matches normative design.
- [ ] **CONN-008** Retryable cleanup persists `disconnecting` state.
- [ ] **CONN-009** Generic identity/grant/target cleanup is idempotent.
- [ ] **CONN-010** Adapter state cleanup cannot cross namespace.

### Targets

- [ ] **TGT-001** Target list/search resolves bound target adapter.
- [ ] **TGT-002** Target resolve/provision resolves bound target adapter.
- [ ] **TGT-003** Protocol IDs remain opaque to generic policy.
- [ ] **TGT-004** Host wrapper signs/version-bounds/size-bounds metadata.
- [ ] **TGT-005** Target ref binds tenant/caller/install/surface/scope.
- [ ] **TGT-006** Forged/cross-scope/stale/unknown-version target rejected.
- [ ] **TGT-007** Slack ID validation and DM provisioning exist only in Slack
  extension/compat migration.
- [ ] **TGT-008** Old Slack target codec migrates and is time-bounded.

### Configuration/actions

- [ ] **ACT-001** Fixed generic surface/action routes exist.
- [ ] **ACT-002** Arbitrary extension-owned protected routes are impossible.
- [ ] **ACT-003** Action ID/schema/effects/connection requirements come from
  resolved contract.
- [ ] **ACT-004** Host auth/body/rate/CORS/schema/audit policy enforced.
- [ ] **ACT-005** Secret form fields never return raw value to UI/adapter.
- [ ] **ACT-006** Remote protocol validation is explicit adapter action.
- [ ] **ACT-007** Action failure rolls back staged config/secrets atomically.
- [ ] **ACT-008** Slack setup/routes/allowed/subjects behavior runs through
  generic actions.
- [ ] **ACT-009** Dead pairing fallback is wired generically or deleted.

## 16. Frontend and wire

- [ ] **UI-001** Every wire surface contains a full stable surface key.
- [ ] **UI-002** Tool view includes capability identity/display generically.
- [ ] **UI-003** Auth view includes provider/display/actions generically.
- [ ] **UI-004** Channel view includes direction/connection/actions/targets.
- [ ] **UI-005** One extension may return/render multiple channels.
- [ ] **UI-006** Channels tab derives only from surface wire.
- [ ] **UI-007** No separate channel registry is queried.
- [ ] **UI-008** Arbitrary fixture channel renders in same component.
- [ ] **UI-009** Second fixture channel renders without frontend source change.
- [ ] **UI-010** Generic config form handles non-secret/secret/schema errors.
- [ ] **UI-011** Generic action form handles connect/disconnect/remote validate.
- [ ] **UI-012** Generic target picker handles list/search/provision.
- [ ] **UI-013** Auth OAuth card has no provider-specific display map.
- [ ] **UI-014** Connection event labels are returned data/fallback, not product
  branch.
- [ ] **UI-015** Automation delivery copy derives from target capabilities.
- [ ] **UI-016** Product localization loads from validated package resources with
  safe host fallback.
- [ ] **UI-017** Slack setup/channel picker/API files deleted.
- [ ] **UI-018** Static asset tests use arbitrary and second-channel fixtures.
- [ ] **UI-019** Source scan finds no concrete package-ID UI condition.
- [ ] **UI-020** Real Slack backend/UI surface flow passes without bespoke API.
- [ ] **UI-021** Serialized Tool/Auth/Channel/multi-channel wire golden fixtures
  are stable and versioned.
- [ ] **UI-022** Unknown future surface kind does not crash extension list and is
  rendered/ignored through explicit unsupported behavior.
- [ ] **UI-023** Previous frontend bundle compatibility in release N is tested.
- [ ] **UI-024** Playwright arbitrary + Slack end-to-end flow passes against real
  backend with hermetic vendor edges.

## 17. Shared provider implementations

- [ ] **PROV-001** Root pins dependency ID/exact version/digest/export.
- [ ] **PROV-002** Package signature covers dependency lock.
- [ ] **PROV-003** Bound owner matches resolved dependency declaration.
- [ ] **PROV-004** Same export/version/digest instance may be reference-counted.
- [ ] **PROV-005** Exact export/version/digest/ABI versions coexist for tenants,
  rolling generations, and callback leases without last-wins behavior.
- [ ] **PROV-006** No last-wins provider registration.
- [ ] **PROV-007** Accounts/grants remain tenant/caller/owning-extension scoped.
- [ ] **PROV-008** Removing one consumer preserves other consumer/refcount.
- [ ] **PROV-009** Dependency cannot widen root surface authority.
- [ ] **PROV-010** Gmail/Drive/Calendar/Docs/Sheets/Slides use one Google
  implementation proof.
- [ ] **PROV-011** Shared provider never appears as independently installable UI
  product.
- [ ] **PROV-012** Callback remains pinned to dependency generation/digest.
- [ ] **PROV-013** Only explicitly declared incompatible singleton keys require
  coordinated activation/conflict.

## 18. Legacy migration and rollback

### Manifest/lifecycle

- [ ] **MIG-001** v2 raw-root stored record migrates.
- [ ] **MIG-002** Old `ManifestHash` is translated without weakening new digest
  verification.
- [ ] **MIG-003** Old enabled state restores equivalent new generation.
- [ ] **MIG-004** Migration state wire/version marker is durable.

### Slack identity/state

- [ ] **MIG-005** `slack_bot` identity migrates to `slack` channel surface.
- [ ] **MIG-006** `slack_personal` identity migrates to `slack` auth/tool surfaces.
- [ ] **MIG-007** Split records deduplicate deterministically.
- [ ] **MIG-008** Setup records and secret handles migrate.
- [ ] **MIG-009** External identity/actor bindings migrate.
- [ ] **MIG-010** Shared-channel subject routes migrate.
- [ ] **MIG-011** Allowed-channel lists migrate.
- [ ] **MIG-012** DM targets/provisioned IDs migrate.
- [ ] **MIG-013** Outbound preferences/reply target refs migrate.
- [ ] **MIG-014** Conversation/source bindings migrate.
- [ ] **MIG-015** Idempotency/single-flight/delivery state migrates where durable.
- [ ] **MIG-016** Old config/env imports once into generic installation config.
- [ ] **MIG-017** Old webhook URL alias forwards to generic ingress.
- [ ] **MIG-018** Old OAuth callback alias forwards to generic auth host.
- [ ] **MIG-019** Old target codec decodes only in compatibility code.

### Migration quality

- [ ] **MIG-020** Dry-run reports safe counts/actions without mutation.
- [ ] **MIG-021** First run succeeds on real old-wire fixture.
- [ ] **MIG-022** Crash/restart resumes safely.
- [ ] **MIG-023** Second run is idempotent.
- [ ] **MIG-024** Malformed record quarantines safely.
- [ ] **MIG-025** Cross-tenant record cannot migrate into another tenant.
- [ ] **MIG-026** Dual-read/one-write window writes only new shape.
- [ ] **MIG-027** Cleanup version removes old aliases/records after success.
- [ ] **MIG-028** Rollback republishes prior immutable generation.
- [ ] **MIG-029** Rollback never rereads mutable source.
- [ ] **MIG-030** libSQL migration passes.
- [ ] **MIG-031** PostgreSQL migration passes.
- [ ] **MIG-032** Root-only legacy hash maps through known bundled catalog or
  requires rematerialization/reapproval; no full digest fabrication.
- [ ] **MIG-033** Cutover release writes only new shape and uses only new runtime
  implementation path.
- [ ] **MIG-034** Remaining aliases/readers are forwarding/read-only, versioned,
  isolated to migration crate, and telemetry-backed.
- [ ] **MIG-035** Cleanup release gate requires N+2, 30 days, 14 zero-observation
  days, audit, operator acknowledgement, and rollback expiry.

## 19. Database schema and transaction parity

- [ ] **DB-001** Fresh libSQL schema creates all package/manifest/install/lease/
  admission/delivery/migration tables and indexes.
- [ ] **DB-002** Fresh PostgreSQL schema creates equivalent constraints/indexes.
- [ ] **DB-003** Exact NEA-25 baseline schema upgrades to new schema on libSQL.
- [ ] **DB-004** Exact NEA-25 baseline schema upgrades on PostgreSQL.
- [ ] **DB-005** Backfills preserve IDs/digests/state or quarantine explicitly.
- [ ] **DB-006** Unique/foreign-key/CAS constraints match on both backends.
- [ ] **DB-007** Failure injection at every multi-record transition rolls back.
- [ ] **DB-008** Concurrent activation/lease/admission/delivery CAS parity passes.
- [ ] **DB-009** Rollback-window package/state rows survive cleanup jobs.
- [ ] **DB-010** Supported old/new binary schema compatibility is explicit and
  tested; unsupported downgrade fails safely.
- [ ] **DB-011** Migration lock/fencing prevents concurrent schema application.
- [ ] **DB-012** PostgreSQL test uses a real testcontainer/service, not a mock.

## 20. Security-negative verification

- [ ] **SEC-001** Package signature tamper rejected.
- [ ] **SEC-002** Fragment/asset/runtime tamper rejected.
- [ ] **SEC-003** Manifest trust request cannot self-grant privileged trust.
- [ ] **SEC-004** Installed third-party package cannot use native/system runtime.
- [ ] **SEC-005** Extra runtime binding cannot hide undeclared behavior.
- [ ] **SEC-006** Host-policy attenuation is narrower/equal to manifest ceiling.
- [ ] **SEC-007** Cross-tenant/install secret access rejected.
- [ ] **SEC-008** Cross-tenant/install state access rejected.
- [ ] **SEC-009** Adapter-supplied auth header rejected where host injects.
- [ ] **SEC-010** Egress redirect/private/reserved IP escape rejected.
- [ ] **SEC-011** Ingress candidate amplification bounded.
- [ ] **SEC-012** Ingress body/rate/deadline abuse bounded.
- [ ] **SEC-013** Signature secret absent from adapter/log/error.
- [ ] **SEC-014** OAuth state/code/token/client secret absent from logs/errors.
- [ ] **SEC-015** Provider raw body absent from user-visible errors.
- [ ] **SEC-016** Target metadata forge/oversize/version attacks rejected.
- [ ] **SEC-017** Dynamic MCP count/schema/effect/authority attacks rejected.
- [ ] **SEC-018** WASM ABI/memory/concurrency/reentrancy/deadline abuse bounded.
- [ ] **SEC-019** Surface action cannot mount arbitrary handler or bypass auth.
- [ ] **SEC-020** Operator inspection is authenticated/read-only/redacted.
- [ ] **SEC-021** Unbounded external IDs are not metric labels.

## 21. Observability and failure semantics

- [ ] **OBS-001** Manifest errors have stable category and safe source span.
- [ ] **OBS-002** Load/bind/readiness/conflict errors have stable categories.
- [ ] **OBS-003** Activation generation/revision/digest/trust delta observable.
- [ ] **OBS-004** Ingress route/candidate/verification outcome/latency observable.
- [ ] **OBS-005** Tool policy/adapter/result outcome observable.
- [ ] **OBS-006** Auth phase observable without sensitive values.
- [ ] **OBS-007** Outbound attempt/part/retry/final status observable.
- [ ] **OBS-008** Drain/quarantine/migration/rollback state observable.
- [ ] **OBS-009** User errors omit raw path/body/token/provider response/backtrace.
- [ ] **OBS-010** Operator view shows resolved surfaces/source/digests/expected vs
  bound/generation/health/ceilings/dependencies safely.
- [ ] **OBS-011** Audit/outbox duplication behavior tested across restart.

## 22. Performance and concurrency

- [ ] **PERF-001** Tool/channel/auth request lookup performs no TOML parse.
- [ ] **PERF-002** Request lookup performs no installation/package store read.
- [ ] **PERF-003** Active lookup uses immutable indexed snapshot.
- [ ] **PERF-004** Snapshot publication is one pointer swap after staged build.
- [ ] **PERF-005** Route lookup uses compiled method/path index.
- [ ] **PERF-006** Ingress parser groups/candidates/inspection time are bounded.
- [ ] **PERF-007** Package extraction/hash limits stop file-count/size/ratio DoS.
- [ ] **PERF-008** WASM wrapper pool/serialization enforces concurrency and
  reentrancy without exposing non-`Sync` instances.
- [ ] **PERF-009** All adapter network calls have deadline/cancellation/body cap.
- [ ] **PERF-010** Generation guard acquisition/release is bounded and leak-free.
- [ ] **PERF-011** Package/provider refcounts remain correct under concurrency.
- [ ] **PERF-012** Delivery scheduling has no unbounded product-specific polling.
- [ ] **PERF-013** Two-process serving-lease contention/fencing/failover passes.
- [ ] **PERF-014** Concurrent activation/upgrade/resolve stress exposes no mixed
  generation.
- [ ] **PERF-015** Metrics use only low-cardinality labels; high-cardinality
  install/surface IDs are restricted to traces/audit.

## 23. Architecture and deletion gates

- [ ] **ARCH-001** Generic crates cannot depend on concrete extension crates.
- [ ] **ARCH-002** `ironclaw_extension_host` cannot depend on concrete extensions.
- [ ] **ARCH-003** `ironclaw_extension_ingress` cannot depend on concrete channels.
- [ ] **ARCH-004** `ironclaw_auth_host` cannot depend on concrete providers.
- [ ] **ARCH-005** Concrete extension cannot depend on composition/auth-host/
  ingress implementation crates.
- [ ] **ARCH-006** Composition implements no Tool/Channel/Auth/Entrypoint adapter.
- [ ] **ARCH-007** Composition constructs no concrete extension.
- [ ] **ARCH-008** Composition mounts no concrete extension route.
- [ ] **ARCH-009** Product workflow contains no concrete product/provider branch.
- [ ] **ARCH-010** Auth core/host contains no concrete provider branch.
- [ ] **ARCH-011** Dispatcher contains no concrete extension/runtime selector.
- [ ] **ARCH-012** Config/CLI contain no channel-specific production type/feature.
- [ ] **ARCH-013** WebUI source contains no product-specific behavior branch.
- [ ] **ARCH-014** Generic production source contains no vendor API method/route/
  protocol payload type outside sanctioned paths.
- [ ] **ARCH-015** `raw_toml()` has no production authority consumer.
- [ ] **ARCH-016** Active snapshot mutation exists only in ExtensionHost.
- [ ] **ARCH-017** Runtime metadata getters duplicating manifest are deleted.
- [ ] **ARCH-018** `ProductAdapterRuntimeEntry`/unused raw projection deleted.
- [ ] **ARCH-019** Provider string multiplexor deleted.
- [ ] **ARCH-020** Slack composition directory deleted.
- [ ] **ARCH-021** `ironclaw_slack_v2_adapter` retired/folded as designed.
- [ ] **ARCH-022** `serve_slack.rs` deleted.
- [ ] **ARCH-023** `slack-v2-host-beta` generic-core feature/dependencies deleted.
- [ ] **ARCH-024** Slack-specific frontend components/APIs deleted.
- [ ] **ARCH-025** Concrete-source temporary allowlist empty.
- [ ] **ARCH-026** Generic crates compile/test with Slack absent.
- [ ] **ARCH-027** Adding second arbitrary fixture required no generic code change.
- [ ] **ARCH-028** Scanner covers all production crates/new generic crates and
  derives forbidden extension/provider/vendor data from package inventory.
- [ ] **ARCH-029** Negative invented-product fixture proves future IDs are caught.
- [ ] **ARCH-030** Host-login SSO is explicitly scoped and cannot serve as an
  extension-provider switch.
- [ ] **ARCH-031** `ironclaw_llm` has no concrete channel formatting branch;
  presentation is typed contract data.
- [ ] **ARCH-032** Trace contribution origin uses generic extension/surface IDs,
  not concrete channel variants.
- [ ] **ARCH-033** `ironclaw_first_party_extension_catalog` is sole concrete
  native factory aggregation edge.
- [ ] **ARCH-034** Lower `ironclaw_first_party_extensions` does not depend on
  product-layer concrete extensions.
- [ ] **ARCH-035** `ironclaw_outbound` does not depend on product-adapter traits;
  coordinator lives in product workflow.
- [ ] **ARCH-036** `ironclaw_extension_egress` owns generic restricted auth/
  channel egress implementations.

## 24. Documentation and CI

- [ ] **DOC-001** Domain glossary matches implemented names.
- [ ] **DOC-002** ADR status updated to Accepted after merge.
- [ ] **DOC-003** This design updated for any approved implementation deviation.
- [ ] **DOC-004** Normative extension-runtime contract matches code.
- [ ] **DOC-005** Existing extensions contract updated.
- [ ] **DOC-006** Product-adapters contract updated.
- [ ] **DOC-007** Host-API/auth/kernel/migration contracts updated.
- [ ] **DOC-008** `reborn-extension-surfaces` skill updated.
- [ ] **DOC-009** New/touched crate AGENTS/CLAUDE guardrails updated.
- [ ] **DOC-010** `FEATURE_PARITY.md` reflects generic runtime, not host-beta.
- [ ] **DOC-011** `CHANGELOG.md` records manifest/runtime/migration behavior.
- [ ] **DOC-012** CI path filters include root/fragments/assets/runtime changes.
- [ ] **DOC-013** No generated `openwiki/` hand edits.
- [ ] **DOC-014** Compatibility window/removal version/operator steps documented.
- [ ] **DOC-015** Rollback instructions tested and documented.

---

## 25. Required command matrix

Record date, commit SHA, environment, exit code, and relevant test count for
each command. A different crate/test name requires an approved design-doc
change, and the traceability table must record the mapping.

### Focused contracts

```bash
cargo test -p ironclaw_extension_contracts
cargo test -p ironclaw_extensions --test manifest_fragment_contract
cargo test -p ironclaw_extensions --test manifest_digest_contract
cargo test -p ironclaw_extensions --test discovery_manifest_bound
cargo test -p ironclaw_extensions --test installations_contract
cargo test -p ironclaw_product_adapter_registry --test manifest_ingestion
cargo test -p ironclaw_product_adapter_registry --test registry_contract
cargo test -p ironclaw_extension_host
cargo test -p ironclaw_extension_host --test performance_contract
cargo test -p ironclaw_extension_ingress
cargo test -p ironclaw_extension_ingress --test bounded_ingress_stress
cargo test -p ironclaw_extension_egress
cargo test -p ironclaw_auth_host
cargo test -p ironclaw_product_adapters
cargo test -p ironclaw_dispatcher
cargo test -p ironclaw_product_workflow
cargo test -p ironclaw_outbound
cargo test -p ironclaw_slack_extension
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_google_auth_provider
cargo test -p ironclaw_notion_auth_provider
cargo test -p ironclaw_first_party_extension_catalog
cargo test -p ironclaw_reborn_migration
```

### Whole-path integration

```bash
cargo test --test reborn_integration_extension_runtime
cargo test --test reborn_integration_extension_migration
cargo test --test reborn_group_extensions
cargo test --test reborn_integration_durable
cargo test --test reborn_integration_oauth_connect
bash scripts/reborn-e2e-rust.sh
bash scripts/ci/check-reborn-qa-fixtures.sh
```

### Architecture and absence

```bash
cargo test -p ironclaw_architecture
bash scripts/ci/check-generic-extension-host-without-concrete.sh
git grep -n 'raw_toml()' -- crates ':!crates/ironclaw_extensions' ':!crates/ironclaw_reborn_migration'
```

Also run the checked-in concrete-source/dependency gates; do not rely on ad hoc
grep alone.

### Frontend

```bash
corepack pnpm --dir crates/ironclaw_webui_v2/frontend test
corepack pnpm --dir crates/ironclaw_webui_v2/frontend lint
corepack pnpm --dir crates/ironclaw_webui_v2/frontend e2e
cargo test -p ironclaw_webui_v2 --all-features
```

### Full quality

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git diff --check
git status --short
```

### Database parity

The new `database_*` cases start a real PostgreSQL testcontainer (Docker is a
declared prerequisite) and create an embedded libSQL database. Run exactly:

```bash
cargo test --features postgres,libsql --test reborn_integration_extension_runtime -- database_ --nocapture
cargo test --features postgres,libsql --test reborn_integration_extension_migration -- database_ --nocapture
```

They include fresh schema, exact-baseline upgrade, failure injection,
activation CAS, serving/generation lease, restore, auth account, admission,
scoped state, target, package GC, and delivery persistence. A skip because
Docker/PostgreSQL is unavailable is a release failure, not a pass.

---

## 26. Machine-readable traceability

`docs/reborn/extension-runtime-evidence.toml` contains exactly one entry for
every atomic requirement ID. `scripts/ci/check-extension-runtime-verification.sh`
rejects duplicate/missing IDs, checked boxes without passing evidence, stale
requirement text hashes, missing SHA/backend/date/reviewer, and non-final state
in release mode. Aggregate REL/SIGN rows reference their atomic IDs.

| Requirement ID | Evidence type | Test/command/path | Backend/feature | Commit SHA | Result/date | Reviewer |
| --- | --- | --- | --- | --- | --- | --- |
| _example: MAN-011..018_ | unit table cases | `manifest_fragment_contract::rejects_unsafe_paths` | default | _sha_ | pass / date | _name_ |
| _populate during implementation_ | | | | | | |

## 27. Final sign-off

- [ ] **SIGN-001 Product model reviewer:** no split product or hidden registry.
- [ ] **SIGN-002 Manifest/security reviewer:** closure/digests/path/trust proven.
- [ ] **SIGN-003 Runtime reviewer:** exact binding and atomic generations proven.
- [ ] **SIGN-004 Tool reviewer:** all lanes use bound dispatcher.
- [ ] **SIGN-005 Channel reviewer:** ingress/outbound/target/connection generic.
- [ ] **SIGN-006 Auth reviewer:** no provider-specific core behavior.
- [ ] **SIGN-007 Persistence/migration reviewer:** both DBs and rollback proven.
- [ ] **SIGN-008 Frontend reviewer:** arbitrary/multiple channels, no product
  branch.
- [ ] **SIGN-009 Architecture reviewer:** allowlists empty; Slack deletion test.
- [ ] **SIGN-010 Release owner:** command matrix and traceability complete.
