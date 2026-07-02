# North star: one unified tool-lifecycle pipeline (Reborn)

**Status:** north-star design + phased plan (pre-implementation)
**Motivating issue:** nearai/ironclaw#5416 (false "Gmail already connected") — a *symptom* of the sprawl below.
**Interim fix:** `docs/plans/2026-07-01-reborn-google-connection-state-5416.md` — designed as a strict subset (Phase 0).
**Scope:** Reborn stack only. Retiring the legacy v1/v2 extension stack is explicitly out of scope.

## 1. The vision

There is **one** question the whole tool lifecycle keeps re-answering in different
places: *"what is the connection state of this capability/extension for this
caller — can the model use it right now, and if not, what unblocks it?"* Today that
question is answered by **8 disjoint state representations** and **3 separate
capability-listing passes**, reconciled by hand at each surface. #5416 is one of the
contradictions that fall out.

North star: **one canonical connection-state, computed once, consumed by every
surface** — search, settings list, the model tool surface, and the pre-dispatch
check. Exactly the shape the authorizer already ships (see §4).

## 2. The sprawl (why this is worth doing)

Eight state types for one concept, none sharing a variant set (investigator map):

| Type | Variants | Crate | Consumed by |
|---|---|---|---|
| `LifecyclePhase` | 12 | `product_workflow` `lifecycle.rs:135` | search/install/activate, list, CLI |
| `ExtensionActivationState` | 3 | `extensions` `installations.rs:172` | durable store; 1-way → `LifecyclePhase` |
| `ExtensionCredentialReadiness` | 4 | `product_workflow` `extension_credentials.rs:16` (`pub(super)`) | settings list only |
| `VisibleCapabilityAccess` | 2 | `host_runtime` `surface.rs:94` | model tool surface |
| `BlockedReason::Auth` | 1/5 | `turns` `status.rs:140` | turn status / gate resume |
| `FirstPartyCapabilityError::AuthRequired` | 1/2 | `host_runtime` `first_party.rs:131` | dispatch-time failure |
| `Decision` (Allow/RequireApproval/Deny) | 3 | `authorization` | **both** surface + dispatch (already unified) |
| `CapabilityStatus` (legacy) | 8 | `src/bridge/…` (non-Reborn) | v1/v2 only — out of scope |

Plus three hand-maintained stringifiers of `LifecyclePhase` (`phase_status`,
`phase_label`, the settings `authenticated`/`needs_setup` cross-product) and **three
capability-listing passes**: `CapabilityCatalog::visible_capabilities`
(`host_runtime/surface.rs:147`, → `VisibleCapability`), composition's
`active_model_visible_capabilities` (`extension_lifecycle.rs:254`, →
`ActiveExtensionCapability`), and the settings `lifecycle_extension_infos`
(`reborn_services/extensions.rs:192`, → `RebornExtensionInfo`).

Every new surface re-reconciles the axes by hand → the oscillation class (#4996 →
#5037 → #5416, three patches on one predicate).

## 3. Feasibility verdict

**Achievable, incrementally, no hard blocker.** Three independent investigations:

- **Credential convergence is ~80% already done.** Both credential "ports"
  (`RuntimeExtensionActivationCredentialGate` and
  `ProductAuthExtensionCredentialSetup::credential_status`) already terminate at
  **one** method — `RuntimeCredentialAccountSelectionService::select_unique_configured_runtime_account`
  over one `CredentialAccountRecordSource`. The only duplication is two result-mapping
  match arms differing solely by `AuthSurface::Api` vs `Web`
  (`product_auth_runtime_credentials.rs:152-175` vs `webui_extension_credentials.rs:39-64`).
- **The surface can be made credential-aware through an existing seam.**
  `CapabilityCatalog` has no credential handle today, but `descriptor.runtime_credentials`
  is already in scope in the loop (`surface.rs:181`), `DefaultHostRuntime` already
  holds `credential_preflight_store: Option<Arc<dyn SecretStore>>` on the same struct
  that builds the catalog (`production.rs:1032`), and `secret_present()`
  (`obligations.rs:2061`) is *documented* as the single source of truth for
  presence — already shared by two dispatch-time call sites. Product-auth (Gmail
  OAuth) presence comes in via the existing dependency-inverted
  `RuntimeCredentialAccountResolver` trait (`obligations.rs:60`; host_runtime defines
  the port, composition implements over `ironclaw_auth`).
- **The authorizer already proves the whole pattern.** `Decision`
  (`authorization`) is computed once by `authorize_dispatch_with_trust` and consumed
  by **both** `visible_capabilities` (surface) and actual dispatch. Credential-presence
  should follow the identical shape.

### Hard constraints (not blockers, but they shape the design)

- **Crate boundaries fix where the pipeline lives.** `product_workflow` cannot see
  `host_runtime`'s types and vice-versa; only `ironclaw_reborn_composition` depends
  on all of `product_workflow`, `host_runtime`, `extensions`, `auth`,
  `authorization`. **The canonical state and its assembly can only live in
  composition** (or a new crate above both) — never pushed down into
  `product_workflow` or `host_runtime`. Cross-layer needs use **dependency-inverted
  ports** (host_runtime defines a trait, composition implements it) — the pattern
  `RuntimeCredentialAccountResolver` already uses.
- **Two credential lifetimes must stay separate.** Durable "is an account
  configured" (presence, `SecretHandle`) ≠ ephemeral dispatch-time secret material
  (`RuntimeSecretInjectionStore`, TTL-bound, keyed by in-flight capability). The
  unified readiness port answers only the first. Never fold in the second.
- **ProductAuthAccount ≠ generic secret.** `capability_credential_requirements`
  deliberately excludes product-auth-account requirements from the `secret_present`
  pre-flight (`production.rs:1993`) to avoid false positives — those are resolved by
  `resolve_access_secret` at dispatch. The surface fold must call the **resolver**
  for product-auth creds, not `secret_present`.
- **The model tool schema has no structured state field.** Even today's
  `Available`/`RequiresApproval` is dropped before the LLM schema
  (`surface_snapshot.rs:14` has no `access` field). Surfacing connection-state to the
  model means either prose in `description` (cheap, stringly) or a new structured
  field on `ProviderToolDefinition`/`ToolDefinition` threaded through every provider
  adapter (clean, wider).
- **Identifier sprawl.** `ExtensionId` / `ExtensionInstallationId` (1:1 today, split
  for an unbuilt multi-install model) / `LifecyclePackageRef` (a third string shape,
  lossy round-trip). Unification should *not* build the 1:many model — just stop
  re-deriving.

## 4. The canonical model

**`CapabilityConnectionState`** — the single source of truth. The **enum + its pure
`project(...)` function** live in a thin, near-zero-dependency leaf crate (new
`ironclaw_capability_state`, or `ironclaw_host_api` if that is the established
shared-vocabulary home) — **not** in composition. Rationale (thermo M4): the enum
and projection depend only on the *shapes* of `Decision`, `ExtensionActivationState`,
and `CredentialReadiness`, none of composition's heavy substrate; and composition is
already the largest crate in the tree (155k lines; `runtime.rs` 10.4k, five files
>2.5k — several already past the `architecture.md` >3k decomposition bar). Dropping a
new cross-cutting concept into it compounds the sprawl this effort fights.
**Composition owns only the *assembly/wiring*** (fetching the three inputs and calling
`project`), never the type.

Projected from three inputs, mirroring how `Decision` is projected from
trust+grants+policy:

```
project(authz: Decision, lifecycle: ExtensionActivationState, cred: CredentialReadiness)
    -> CapabilityConnectionState
```

Model-facing collapse (what the LLM / UI reads — the #5416 interim `ExtensionAvailability`
is exactly this, at the search surface):

| State | Meaning | Projection |
|---|---|---|
| `available` | usable now / connect with no user action | authorized ∧ installed ∧ credential present (or none required) |
| `needs_auth` | a required credential is missing → sign-in | authorized ∧ installed ∧ credential missing |
| `needs_approval` | usable but gated on approval | `Decision::RequireApproval` |
| `not_installed` | discover / install first | no installation |
| `unknown` | indeterminate (transient) → claim nothing | credential backend blip |

Lifecycle phase (Active/Installed/Disabled) and the raw readiness enum stay
**internal** — they feed the projection, they are not surfaced. One state out, not
three axes.

## 5. Architecture

The four surfaces are **not symmetric consumers** — they sit in three different
crates, so each reaches the canonical state differently (thermo B1). Type lives in a
leaf crate (§4); assembly in composition; the two surfaces composition can't reach
directly (host_runtime, product_workflow) each get a **dependency-inverted port**.

```
        leaf crate (ironclaw_capability_state):  enum CapabilityConnectionState + fn project(..)

                     ironclaw_reborn_composition  (assembly only)
        ┌──────────────────────────────────────────────────────────────┐
        │  project(Decision, ExtensionActivationState, CredentialReadiness) │
        │        ▲              ▲                    ▲                     │
        │  CredentialReadiness  durable store   authorizer Decision        │
        │  (§P1: one fn over the single selector; already unified)         │
        └───┬───────────────────────────┬──────────────────────┬─────────┘
   in-crate │                    port ▼ (host_runtime)   port ▼ (product_workflow)
     ┌──────┴───────┐        RuntimeCredentialAccountResolver   CapabilityConnectionStateProvider
  search tool   pre-dispatch  + credential-presence handle on    (NEW, §P4a — product_workflow
  (extension_   check         CapabilityCatalog (§P2)            defines trait, composition impls,
   lifecycle)   (composition)  → model tool surface               injected into RebornServices)
                                                                  → settings list (extension_info)
```

- **Type home:** thin leaf crate (§4), not composition.
- **Assembly home:** `ironclaw_reborn_composition` — the only crate that can *see* all
  three inputs (verified: `host_runtime` Cargo.toml has no `product_workflow`/`auth`
  dep; `product_workflow`'s composition dep is `[dev-dependencies]` only). This
  mutual-blindness premise is the load-bearing constraint and it holds.
- **Three dependency-inverted ports** (host_runtime never depends on
  `auth`/`product_workflow`; product_workflow never depends on `host_runtime`/
  composition):
  1. `RuntimeCredentialAccountResolver` (exists) — product-auth presence into host_runtime.
  2. credential-presence handle on `CapabilityCatalog` (NEW, §P2) — surface fold.
  3. `CapabilityConnectionStateProvider` (NEW, §P4a) — **the missing port** the first
     draft omitted: product_workflow defines the trait, composition supplies the impl,
     injected into `RebornServices`, so the settings list consumes the *same* canonical
     projection instead of its hand cross-product. Without this, "consumed by every
     surface" is false and the settings list keeps an independent `authenticated`/
     `needs_setup` forever.
- **The one credential answer:** a single composition-owned `credential_readiness()`
  wrapping the one selector, consumed by search + activate + list + surface fold.

## 6. Phased delivery (each phase independently shippable)

**Phase 0 — interim #5416 fix (already planned, strict subset).** Collapse the
*search* surface to the model-facing `ExtensionAvailability`; fail the settings
`authenticated` bit closed on `Unknown`. Ships the canonical model's *output* shape at
one surface and fixes the reported bug. Doc: the companion 5416 plan.

**Phase 1 — one credential-readiness function (NOT behavior-preserving; needs its own
test).** Extract `credential_readiness(selector, scope, surface, requirement) ->
CredentialReadiness` in composition; route both existing shims through it. **Correction
(thermo M1):** the two paths do *not* "differ only by `AuthSurface`". The
search/activate path flags an account that is `Configured` but missing its
`access_secret` as `Backend`/anomaly (`product_auth_runtime_credentials.rs:164-174`);
the settings path treats **any** `Ok(Some(_))` as `Configured` with no `access_secret`
check (`webui_extension_credentials.rs:35-64` → `extension_credentials.rs:126`). That's
the same fail-open/fail-closed class §3 of the 5416 doc diagnoses. Unifying therefore
**changes settings-list behavior**: an orphaned-secret account stops reading as
"connected". Scope Phase 1 as "extract one fn **and** reconcile the access-secret
divergence behind a `MissingSecretHandle` (or `Unknown`) distinction, with a dedicated
regression test on the settings path" — do not estimate it as a mechanical extract.

**Phase 2 — credential-aware capability surface (closes #5416 Defect C).** Thread a
credential-presence handle into `CapabilityCatalog` via `with_credential_presence(...)`
(mirrors `with_filesystem`); inside `visible_capabilities`, after the authorizer
`Decision`, consult `capability_credential_requirements(descriptor)` +
`secret_present` (generic) / `RuntimeCredentialAccountResolver` (product-auth) and
downgrade to a new `VisibleCapabilityAccess::NeedsAuth`. Verified feasible:
`descriptor.runtime_credentials` is already in the loop, `DefaultHostRuntime` already
holds `credential_preflight_store` on the same struct that builds the catalog
(`production.rs:1032`). `capability_credential_requirements` moves from `pub(crate)` in
`production.rs` to a shared home. **Acceptance criteria (not follow-ups):**
- **Caching (thermo M2).** The surface is rebuilt on *every LLM step*
  (`HostRuntimeLoopCapabilityPort::visible_capabilities` →
  `DefaultHostRuntime::visible_capabilities`, `production.rs:1032`), not once per turn.
  A naive fold adds N credential lookups (DB/keychain/account-resolver) per build.
  Ship a short-TTL presence cache keyed by `(scope, SecretHandle)` shared across
  surface builds within a run, or explicitly justify the added per-step latency.
- **Fingerprint stability (thermo M3).** `access` already feeds `surface_version`
  (`surface.rs:304,393`). `NeedsAuth` is credential-derived → flips on OAuth
  expiry/revocation — a **time-sensitive** signal, the same class that caused the
  #4789 stale-surface incident (`expires_at` kept out of the hash). Decide explicitly
  whether `NeedsAuth` participates in the version hash; default to **excluding** the
  credential-derived component from the fingerprint and re-deriving it per render, as
  #4789's follow-up established.

> **Phase 2 landed (2026-07-02, thermo-approved).** Implementation notes that bind
> Phase 3: (a) product-auth presence uses the new side-effect-free
> `RuntimeCredentialAccountResolver::account_configured` (select-only) — the
> original assumption that `resolve_access_secret` was a pure presence check was
> wrong; it refreshes OAuth tokens and must never run on surface render. (b) The
> presence cache key is `(owner scope, provider, requester_extension, sorted
> provider_scopes)` — scopes are load-bearing (gmail readonly/send/modify differ).
> (c) Confirmed drop point for Phase 3: `ironclaw_loop_support/src/capability_port.rs`
> maps `VisibleCapability` → `CapabilityDescriptorView`
> (`ironclaw_turns/src/run_profile/host.rs:1389`), which carries no `access`
> field — the single seam where the prose-fold (or a structured field) attaches.

**Phase 3 — carry state to the model schema.** Stop dropping capability state at
`surface_snapshot.rs:14` (even today's `Available`/`RequiresApproval` never reaches the
LLM). **Default: fold `CapabilityConnectionState` into the tool `description`** — one
seam, no schema change. The structured-field alternative
(`ProviderToolDefinition`/`ToolDefinition` + a per-provider rendering decision) touches
~8 backend adapters (`codex_chatgpt`, `gemini_oauth`, `bedrock`, `rig_adapter`,
`nearai_chat`, `openai_codex_provider`, `reasoning`, `placeholder_stripping`) — real
cost, opt-in only if the prose channel proves insufficient.

**Phase 4 — collapse the redundant representations (strategic).** Split by reachability:
- **4a — the missing product_workflow port (prerequisite).** Add
  `CapabilityConnectionStateProvider` (thermo B1): product_workflow trait, composition
  impl, injected into `RebornServices`. *Only after this* can the settings
  `authenticated`/`needs_setup` hand cross-product be replaced by the canonical
  projection. Without 4a, the settings surface is unreachable from the canonical state.
- **4b — deletions, opportunistic.** Retire the two extra stringifiers; fold
  `active_model_visible_capabilities` into the one catalog pass; unify the identifier
  round-trips (stop re-deriving `ExtensionInstallationId`/`LifecyclePackageRef` from
  `ExtensionId` — do **not** build the unbuilt 1:many model). Tracking issue; no rush.

**Where "good enough" is.** #5416 is fully fixed by **Phase 0 + Phase 2** (search and
tool surface both credential-honest). Phase 1 is the correctness/dedup investment that
makes 0/2 rest on one credential answer. Phase 3 improves model legibility. Phase 4 is
pure debt paydown — valuable, but not required for correctness. Stop and reassess after
Phase 2.

## 7. Non-goals

- Retiring the legacy v1/v2 stack (`src/extensions/`, `CapabilityStatus`,
  `InstalledExtension`) — disjoint, separately owned.
- Building the multi-install-per-extension model implied by the
  `ExtensionId`/`ExtensionInstallationId` split.
- Folding dispatch-time secret staging (`RuntimeSecretInjectionStore`) into the
  readiness port — different lifetime, must stay separate.

## 8. Interim-vs-north-star contract

The Phase 0 interim fix is a **subset, not throwaway**: its `ExtensionAvailability`
(`available`/`needs_auth`/`not_installed`/`unknown`) maps cleanly onto
`CapabilityConnectionState` (same states minus `needs_approval`, which search has no
axis for). Later phases *extend* it, they don't discard it: Phase 1 swaps its inline
gate mapping for the shared credential-readiness fn, Phase 2 extends the same state to
the tool surface, Phase 3 carries it to the schema.

**One honest caveat (thermo m4):** Phase 1 introduces the `MissingSecretHandle`/`Unknown`
reconciliation (§P1), which changes the *inputs* to Phase 0's projection — so Phase 0's
projection fn and its test #5 get a **one-line follow-up edit** when Phase 1 lands. Not
"thrown away", but not zero-touch either.

## 9. Delivery order & decision points

1. **Phase 0** — ship now (companion 5416 doc); independently valuable.
2. **Decide the crate-home question (§4/M4) before Phase 2** lands new state into
   composition — cheap now, expensive once P2/P4 code exists there.
3. **Phase 1** — rescoped to reconcile the access-secret divergence (M1) with its own
   settings-path regression test. Not "small".
4. **Phase 2** — with the caching strategy (M2) and fingerprint-stability decision (M3)
   as *acceptance criteria*, not deferred follow-ups. `#5416` fully closed here.
5. **Phase 4a** (the missing product_workflow port, B1) — prerequisite before promising
   any "fold the settings cross-product".
6. **Phase 3 + Phase 4b** — opportunistically. Reassess necessity after Phase 2.
