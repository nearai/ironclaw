# `ironclaw_trust` — cross-crate contract

**Status:** V1 contract
**Crate:** `crates/ironclaw_trust`
**Depends on:** `ironclaw_host_api` (vocabulary)
**Consumed by:** `ironclaw_authorization` (and any future grant-store /
extension-registry crate that needs a policy-validated trust ceiling)

This document is the source-of-truth contract for the trust-policy
substrate. It is co-located with the crate so changes to the contract
are reviewed alongside changes to the implementation. The Reborn-track
docs at `docs/reborn/contracts/host-api.md` and `extensions.md` (in the
staging branch) cover the broader Reborn vocabulary; this file is the
authoritative description of *how trust evaluation works* and how the
`PackageIdentity` / `RequestedTrustClass` / `EffectiveTrustClass` /
`AuthorityCeiling` types compose.

---

## 1. Purpose

`ironclaw_trust` evaluates host trust policy for any manifest-bearing
package and produces the effective trust ceiling that downstream
authorization consumes:

```text
PackageIdentity + RequestedTrustClass + requested authority
  + layered PolicySource entries
  -> TrustPolicy::evaluate(...)
  -> TrustDecision { effective_trust, authority_ceiling, provenance }
```

The crate does not register capabilities, issue grants, dispatch,
hold secrets, or know about runtimes. It is the policy-evaluation
layer that makes `TrustClass` host-determined rather than
manifest-asserted.

`TrustClass` (in `ironclaw_host_api`) names the four ceilings; the
crate-internal `EffectiveTrustClass` is the policy-validated newtype
that downstream authorization is required to consume — its privileged
variants (`FirstParty`, `System`) are constructible only from inside
this crate.

---

## 2. Identity scope (`PackageIdentity`)

`PackageIdentity` is the trust-policy-side identity for any
manifest-bearing package. Its `package_id: PackageId` overlaps with —
and in V1 *equals* — `ExtensionId` for installed extensions, and
extends to other manifest-bearing units that go through trust
evaluation.

| Scope | `PackageSource` | `package_id` overlap | Notes |
|---|---|---|---|
| Installed extensions (WASM, Script, MCP) | `LocalManifest { path }` or `Registry { url }` | `ExtensionId` | Manifest declares `trust = ...` |
| Bundled extensions / loops / skills | `Bundled` | `ExtensionId` | Compiled/bundled with the host binary; matched by `BundledRegistry` |
| Operator declarations | `Admin` | `ExtensionId` (operator-chosen) | Out-of-band trust assertion via `AdminConfig` |
| Built-in tools | `Bundled` (intended; see §9) | host-defined | Migration target — see §9 |

Rules:

- The `package_id` field in `PackageIdentity` is the same `PackageId`
  newtype the rest of Reborn uses for `ExtensionId`. The two names
  describe the same value at different layers — `ExtensionId` when
  the identity reaches the extension registry /
  `CapabilityDescriptor.provider`, `PackageId` when it reaches the
  trust policy.
- A `package_id` collision across `PackageSource` variants is never
  treated as the same package. Trust is bound to
  `(package_id, source)` — see §4.
- Identity drift across re-evaluation (`package_id`, `source`,
  `digest`, `signer`) invalidates retained grants per AC #7. The
  helpers `identity_changed` / `grant_retention_eligible` expose this.

---

## 3. Requested vs effective trust

The crate enforces a strict split between the *declarative manifest
claim* and the *host-validated ceiling*:

| Type | Defined in | Source of truth | Privileged variants |
|---|---|---|---|
| `RequestedTrustClass` | `ironclaw_host_api::trust` | manifest / registry entry | freely deserializable; not authority |
| `TrustClass` | `ironclaw_host_api::runtime` | wire vocabulary | `FirstParty`, `System` reject deserialization (`#[serde(skip_deserializing)]`) |
| `EffectiveTrustClass` | `ironclaw_trust::decision` | `HostTrustPolicy::evaluate` | privileged constructors are crate-private; serializable for audit but **not** deserializable |
| `HostTrustAssignment` | `ironclaw_trust::decision` | host-controlled bundle/admin loaders | production seeding token for policy entries; **not** an authorization input and not deserializable |

Construction-side guarantees:

- A user-installed manifest cannot fabricate `EffectiveTrustClass::FirstParty`
  or `::System` by deserializing into a wire type — both `TrustClass`
  privileged variants and `EffectiveTrustClass` itself reject the
  deserialization path. `EffectiveTrustClass` is wire-asymmetric: it
  serializes for audit envelopes, but is not `DeserializeOwned`.
  A `static_assertions::assert_not_impl_any!` lock pins this at
  compile time.
- The only public `EffectiveTrustClass` constructors are `sandbox()`
  and `user_trusted()`. Privileged effective values flow out of
  `TrustPolicy::evaluate` in non-test builds. Crate-internal tests have
  `#[cfg(test)]` fixtures, but there is deliberately no Cargo feature that
  exposes privileged effective constructors to downstream crates.
- Production bundle/admin loaders seed privileged policy entries through
  `HostTrustAssignment` and the crate-owned `BundledEntry::new` /
  `AdminEntry::for_*` constructors. `HostTrustAssignment` is not
  deserializable and is not accepted by authorization APIs; it is only the
  host-policy configuration token that the trust crate converts into an
  `EffectiveTrustClass` during source evaluation.

Manifest input: the `trust = "..."` field on extension manifests
populates `RequestedTrustClass`, **not** `TrustClass` directly.
Effective trust is established by `TrustPolicy::evaluate` at registry
insertion time, not by parsing the manifest.

`CapabilityDescriptor.trust_ceiling: TrustClass` (defined in
`ironclaw_host_api::capability`) carries the manifest's *declarative
metadata*, not the policy-validated effective ceiling. Two reasons:

- The privileged variants of `TrustClass` reject deserialization, so
  this field can only carry `Sandbox` or `UserTrusted` from manifest
  input. A manifest declaring `trust = "first_party_requested"` parses
  through `RequestedTrustClass` first.
- The *effective* ceiling that authorization consumes is
  `EffectiveTrustClass`, attached to `ExecutionContext.trust` at
  dispatch time, not stored in the descriptor.

Downstream authorization compares grants against
`ExecutionContext.trust` (effective), never against
`descriptor.trust_ceiling` (declarative).

---

## 4. Evaluation matrix

`HostTrustPolicy` composes layered `PolicySource`s in priority order;
the first source returning `Some(SourceMatch)` wins. If no source
matches, the policy falls through to a fail-closed default.

### 4.1 Per-source match keys

| Source | Match keys | Returns on match | On miss |
|---|---|---|---|
| `BundledRegistry` | `package_id` AND `source == Bundled` AND (`digest` if pinned) | `(effective_trust, allowed_effects, max_resource_ceiling)` provenance `Bundled` | `Ok(None)` — fall through |
| `AdminConfig` | `package_id` AND **exact** `PackageSource` match (incl. `LocalManifest { path }` / `Registry { url }`) AND (`digest` if pinned) | provenance `AdminConfig` | `Ok(None)` |
| `SignedRegistry` | (currently inert; see §10) | — | `Ok(None)` always in V1 |
| `LocalDevOverride` | (currently inert; see §10) | — | `Ok(None)` always in V1 |

Match-key rules:

- `package_id` alone is **never** sufficient. Cross-source shadowing —
  a `LocalManifest` package with the same id as an admin-blessed
  `Bundled` package — is rejected at the source level. The
  `AdminEntry::for_local_manifest` constructor exists separately so
  every elevation of a user-writable origin is `rg`-greppable.
- `digest`, when set on the entry, must equal the package's
  `PackageIdentity::digest` exactly. Drift falls through to the next
  source / default. This is the AC #7 grant-reissue trigger.

### 4.2 `RequestedTrustClass` semantics

`RequestedTrustClass` is **audit/claim metadata only** in V1. Sources
do not consult it during matching. It is recorded for audit envelopes
and used by future grant-store wiring to detect manifest-side intent
drift, but does not constrain `EffectiveTrustClass` upward or downward:

- A package requesting `Untrusted` may still receive `FirstParty` if
  host policy assigns that ceiling — the host is the sole authority
  on effective trust, and the manifest's claim does not cap host
  decisions.
- A package requesting `FirstPartyRequested` receives `Sandbox` /
  `UserTrusted` / `Default` provenance if no source assigns the
  privileged ceiling. The request alone is never sufficient.

Rationale: conflating manifest intent with ceiling computation makes
the policy harder to audit and gives manifests a partial say in their
own ceiling. The split is intentional.

### 4.3 `requested_authority` semantics

`requested_authority: BTreeSet<CapabilityId>` is the canonicalized set
of capabilities the package is asking authority over. In V1:

- It is **not** consulted by `PolicySource::evaluate` and does **not**
  cap `AuthorityCeiling`. The ceiling comes purely from policy entries.
- It is used as a stable evaluation input for pre/post mutation probes.
  It is **not** forwarded to `TrustChange` and must not be used as the
  authoritative revocation scope; invalidation listeners derive revocation
  from their grant store plus the decision/ceiling delta.
- It is `BTreeSet`, not `Vec` — capability authority is a set, not a
  multiset, and `[a, a, b]` vs `[a, b]` must never produce different
  invalidation outcomes.

### 4.4 First-match-wins

Source priority is the chain order passed to `HostTrustPolicy::new`.
The first source returning `Ok(Some(...))` is binding; subsequent
sources are not consulted. Duplicate source *types* are rejected at
construction time because mutation APIs target sources by concrete type;
allowing two `AdminConfig` instances would make evaluation and mutation
routing ambiguous. Different source types may still match the same
identity, and first-match-wins resolves that intentionally.

Recommended chain order for production wiring:

```text
[
    BundledRegistry,    // host-controlled, signed-bundle baseline
    SignedRegistry,     // remote signature verification (when live)
    AdminConfig,        // operator overrides
    LocalDevOverride,   // dev-only opt-in (when live)
]
```

### 4.5 Default fallback

When no source matches, `default_decision` returns:

```text
EffectiveTrustClass::sandbox()
+ AuthorityCeiling::empty()
+ TrustProvenance::Default
```

**Uniformly fail-closed across every `PackageSource`.** Earlier draft
shapes granted `UserTrusted` to unmatched `Bundled` / `Registry` /
`Admin` origins, but that was fail-open in two specific ways:

- `Registry { url }` is a remote source; `SignedRegistry` is currently
  inert, so granting `UserTrusted` to an unverified remote package on
  the basis of a self-declared `url` is the textbook fail-open shape.
- `Bundled` reaching the default path means the package isn't in
  `BundledRegistry` — that's a host-config bug (catalog out of sync
  with the binary), not a runtime "this package is trustworthy"
  situation.

Loud detection of "Bundled package missing from registry" belongs in
a startup audit, not in `evaluate()` returning `Err` for a deployment
problem.

---

## 5. Authority ceiling vs grant

`TrustDecision::authority_ceiling: AuthorityCeiling` is an *upper
bound* on what may be granted, not a grant:

```rust
pub struct AuthorityCeiling {
    pub allowed_effects: Vec<EffectKind>,
    pub max_resource_ceiling: Option<ResourceCeiling>,
}
```

Trust class on its own grants nothing. `ironclaw_authorization`
(the consumer crate) must intersect a proposed `CapabilityGrant`'s
declared effects against `allowed_effects`, and any `ResourceProfile`
against `max_resource_ceiling`, before authorizing dispatch.

This matches the broader Reborn rule that registered capabilities are
only possibilities; dispatch still requires grants/leases (see
`docs/reborn/contracts/capability-access.md` §2 in the staging-track
docs).

---

## 6. Mutation and invalidation orchestration

When the host policy revokes or downgrades a package's effective
trust, or shrinks the authority ceiling while keeping the same trust
class, affected grants must be invalidated **before** any subsequent
dispatch runs under the stale ceiling (AC #6).

Computing the *previous* decision requires evaluating the
whole policy chain — not just the source being mutated. The
orchestration therefore lives at the `HostTrustPolicy` layer, not the
per-source layer:

```rust
policy.mutate_with(
    &bus,
    affected_identity,
    requested_authority,
    requested_trust,
    |mutators| {
        mutators.admin_remove(&package_id, &source)?;
        Ok(())
    },
)?;
```

`mutate_with` is the **only public runtime-mutation path**. The
per-source `upsert` / `remove` methods on `BundledRegistry` /
`AdminConfig` / `SignedRegistry` are `pub(crate)` and reachable only
through `SourceMutators` inside a `mutate_with` closure. The
orchestration:

1. Reject duplicate source types at construction time, so mutator routing
   cannot disagree with evaluation about which source instance is active.
2. Acquire a policy-level mutation gate so concurrent `evaluate()` calls
   cannot observe in-flight mutations.
3. Pre-evaluate `affected_identity` to capture the previous full decision.
4. Run the closure with mutator handles that stage operations rather than
   mutating live source state.
5. If the closure returns an error, discard staged operations and return
   the error; no lower decision became visible.
6. Commit staged mutations, post-evaluate, and publish a `TrustChange` on
   the `InvalidationBus` synchronously when the trust class changed or the
   authority ceiling shrank.
7. Release the mutation gate only after synchronous publication completes.
   A listener running on the publishing thread may re-enter `evaluate()` for
   read-only inspection without deadlocking; other threads remain blocked
   until publication completes.

Construction-time population (`with_entries` / `with_signers`)
remains `pub` because no policy state exists for an invalidation to
be meaningful against — those constructors seed the chain *before*
it is wired up to a bus.

`TrustChange::new(...) -> Option<Self>` compares the full previous and
current `TrustDecision`. It returns `None` for no-ops and benign ceiling
expansions; it returns `Some` for trust-class changes and same-class
authority-ceiling reductions. `InvalidationBus::publish` applies a
defense-in-depth filter and `debug_assert!`s on non-invalidating events.
Helper methods `is_downgrade` / `is_upgrade` / `is_kind_change`
(latter for `FirstParty ↔ System`) / `authority_ceiling_reduced` let
listeners be selective — naive "any TrustChange ⇒ revoke" listeners
would over-revoke on benign upgrades.

---

## 7. Set semantics for capability authority

All capability-authority surfaces are typed as
`BTreeSet<CapabilityId>`, not `Vec` or `&[CapabilityId]`:

- `TrustPolicyInput::requested_authority`
- `mutate_with`'s `requested_authority` parameter
- `authority_changed` / `grant_retention_eligible`

Rationale: capability authority is conceptually a set, not a
multiset. A slice-based shape forced length-guarding against
`[a, a, b]` vs `[a, b]` at the cost of false-positive change
detection between two callers that meant the same set but
canonicalized differently. `BTreeSet` makes the multiset literally
inexpressible at the type boundary — duplicates collapse at
construction. `BTreeSet` over `HashSet` for deterministic
iteration, which matters for audit replay and golden-file
comparisons.

---

## 8. Audit serialization

`EffectiveTrustClass` serializes to canonical snake_case wire
strings (audit-stable):

| Variant | Wire string |
|---|---|
| `Sandbox` | `"sandbox"` |
| `UserTrusted` | `"user_trusted"` |
| `FirstParty` | `"first_party"` |
| `System` | `"system"` |

`TrustDecision` serializes the full audit envelope (effective_trust +
authority_ceiling + provenance + evaluated_at). `TrustProvenance` is
a tagged enum with `kind` discriminator: `default`, `bundled`,
`admin_config`, `signed_registry`, `local_manifest`.

The wire-shape contract is locked in by tests T18 and
`trust_decision_serializes_for_audit`. Renames to underlying
`TrustClass` variants or `serde(rename_all)` changes will fire those
tests rather than silently shift audit-envelope contents.

---

## 9. Built-in tool migration intent

Existing built-in tools (shell, http, web_fetch, file, message,
memory, image, etc.) currently reach dispatch through `ToolRegistry`
without going through this trust policy. The intended end state is
that every built-in becomes a `Bundled` `PackageIdentity` entry with
a matching `CapabilityDescriptor`:

| Built-in tool | Eventual `PackageSource` | Eventual `EffectiveTrustClass` | Eventual effects |
|---|---|---|---|
| shell / script execution | `Bundled` | `FirstParty` | `ExecuteCode`, `ReadFilesystem`, `WriteFilesystem` |
| http / web_fetch | `Bundled` | `FirstParty` | `Network` |
| message (channel-bound) | `Bundled` | `System` | `ExternalWrite`, `Network` |
| credential-backed (Gmail / GitHub / Slack) | `Bundled` | `FirstParty` | `Network`, `UseSecret` |
| memory / workspace | `Bundled` | `FirstParty` | `ReadFilesystem`, `WriteFilesystem` |

Migration is **out of scope for V1** — this PR ships the substrate.
Per-built-in PRs migrate them through the unified path so Reborn
ends with one authorization model rather than two parallel ones
(legacy `ToolRegistry` + new policy engine).

### 9.1 Pre-migration path for built-ins

Until migration completes, built-in tools retain their existing
authorization / lifecycle paths. Per-axis mapping so the asymmetry
is reviewable rather than implicit:

| Axis | V1 built-in path (pre-migration) | V1 manifest-bearing package path | Post-migration target |
|---|---|---|---|
| Trust ceiling | implicit (`ToolRegistry` membership = sanctioned) | `EffectiveTrustClass` from this crate | Unified — `EffectiveTrustClass` for both |
| Grant / lease | per-tool `PermissionMode` decision; no formal `CapabilityGrant` | `CapabilityGrant` / `CapabilityLease` (`ironclaw_authorization`, see `capability-access.md`) | Unified — grants for both |
| Approval | per-tool `approval` setting + UnlessAutoApproved gate | gate via `ironclaw_authorization` + obligation handlers | Unified — see `approvals.md` |
| Secret injection | `credential_injector` ad-hoc, scoped per tool | `lease_once` + `consume` (see `secrets.md` / `host-runtime.md`) | Unified — typed encrypted secret repository |
| Network | per-tool allowlist (HTTP tool's `allowed_hosts`) | network obligations (`network.md`) | Unified through `ironclaw_network` |
| Mount / filesystem | ad-hoc `HostPath` access through tool internals | `ScopedPath` / `MountView` per `host-api.md` §7 | Unified — `ScopedPath` for both |
| Invalidation | n/a — `ToolRegistry` is configuration-time stable | `InvalidationBus` synchronous trust-change propagation | Unified — `InvalidationBus` for both |
| Audit | per-tool tracing / `ActionRecord` | `TrustDecision` audit envelope + capability-host audit | Unified envelope |

The asymmetry is intentional during the transition. New tools added
during the migration window should land **directly on the unified
path** (a `Bundled` `PackageIdentity` + `CapabilityDescriptor`),
not the legacy `ToolRegistry` track. The legacy track only exists
to keep already-shipped tools running until each is migrated.

---

## 10. Current limits

This slice intentionally keeps the substrate narrow:

- `SignedRegistry` is structural only — `evaluate` always returns
  `Ok(None)`. Real signature verification (binding `(signer,
  package_id, digest)` so a verified signature on package X does not
  vouch for an unrelated package Y) belongs to a follow-up.
- `LocalDevOverride` is inert — `enabled_for_test` is compiled only for
  this crate's `#[cfg(test)]` targets, so future dev-mode opt-in has a
  stable seam without exposing a production feature. The inert contract is
  pinned by test T17.
- `AdminEntry` fields are `pub` for ergonomics; the `for_*`
  constructors are conventions, not type-enforced. Tightening to
  fully-private fields + accessor methods is a follow-up.
- No durable persistence: policy entries live in `RwLock<HashMap>`s.
  Persistence belongs to the operator-config / bundle-loader layers,
  not here.
- No tracing / metric emission on policy decisions yet — when the
  observability seam lands, `evaluate` should emit a structured event
  per decision (provenance + class + decision_id) for audit replay.

Those should be added as follow-on slices once `ironclaw_authorization`
consumes the substrate end-to-end.
