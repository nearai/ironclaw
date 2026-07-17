# Reborn Architecture Simplification: Fewer DTOs, Less `dyn`, No Local-Specific Structs

**Date:** 2026-07-17
**Status:** Proposal / design note (not yet a contract)
**Scope:** The capability/turn execution path and storage substrates in `crates/`

This note proposes a **fundamental** simplification of the Reborn host/runtime
internals. The goal is to remove three recurring costs without weakening any
security invariant:

1. **DTO proliferation** — a single tool call is re-wrapped through ~14
   near-identical request/result structs.
2. **`dyn` proliferation** — ~6 hot-path trait objects, most with exactly one
   production implementation.
3. **Local-specific structs** — a parallel `InMemory*` / `Filesystem*` store
   tree per domain, plus deployment-mode struct families that risk local-only
   shortcuts leaking into production.

The thesis: these are three symptoms of **one** decision — *treating every crate
boundary as if it were a trust boundary.* Reborn has exactly **one** trust
boundary (the untrusted agent loop ↔ the host). Everything below it is trusted
host code that was nonetheless split into ~6 crates, each with its own DTO,
its own `dyn` trait, and its own backend struct family. Collapse the trusted
internals onto a shared vocabulary and concrete wiring; keep the single membrane.

---

## 1. Evidence: where the complexity and the bugs actually are

This proposal is grounded in a code audit plus a cross-reference against the
last ~30 days of merged PRs and open issues.

### 1.1 The capability call re-wraps one payload ~14 times

A single `bash`/`wasm`/first-party tool call is translated through **~8 request
shapes and ~6 result shapes across 6 crate boundaries and 6+ `dyn` seams**. This
is not uniform waste, and it is worth being precise about *why*, because it
determines what is reducible. The field-level diff shows the pipeline holds only
**three genuinely distinct states**; the extra types are duplication forced by the
crate graph plus dead fields that ride along.

Verified request types on the down-path, with what each hop actually adds or drops:

| Hop | Type (crate) | Fields | Change vs. previous |
| --- | --- | --- | --- |
| 1 | `CapabilityInvocation` (`ironclaw_turns`) | `activity_id, surface_version, capability_id, input_ref, approval_resume?, auth_resume?` | the loop's vocabulary — input by **ref**, resume tokens, pre-trust |
| 2 | `RuntimeCapabilityRequest` (`ironclaw_host_runtime`) | `context, capability_id, estimate, input, idempotency_key?, trust_decision` | deref `input_ref`→raw `input`; +`estimate`, +`context`; +2 **dead** fields |
| 3 | `CapabilityInvocationRequest` (`ironclaw_capabilities`) | `context, capability_id, estimate, input, trust_decision` | **identical to hop 2 minus `idempotency_key`** — zero new info |
| 4 | `CapabilityDispatchRequest` (`ironclaw_host_api`) | `capability_id, scope, authenticated_actor_user_id?, estimate, mounts?, resource_reservation?, input` | decompose `context`→`scope`+`actor`; **drop `trust_decision`**; **+`mounts`, +`resource_reservation`** (authorization outputs) |
| 5 | `RuntimeAdapterRequest<'a, F, G>` (`ironclaw_dispatcher`) | hop 4 + `package, descriptor, filesystem&, governor&, runtime_policy` | + resolved substrate handles for the lane |
| — | lane requests (`ScriptExecutionRequest` / `WitToolRequest` / `FirstPartyCapabilityRequest`) | per-lane shapes | final re-wrap into the lane's own type |

Four mechanisms produce this, each visible in the code above:

1. **The dependency DAG forbids type sharing, so identical shapes are
   re-declared.** Hops 2 and 3 are the *same struct* (`context, capability_id,
   estimate, input, trust_decision`), differing by one field. They are distinct
   types only because `host_runtime` (upper) and `capabilities` (lower) are
   separate crates and the boundary rule forbids either importing the other's
   request type. The one shareable place — `host_api`, the bottom — is used for
   hop 4 but not the mid-flight shapes, so the mid-flight shape is declared twice.
   This is the concrete form of "every crate boundary treated as a trust
   boundary."

2. **Three real states are modeled as five look-alike structs.** The pipeline has
   exactly three meaningful states: *loop-expressed* (hop 1: ref-based, resume
   tokens, pre-trust), *authorized* (hop 4: raw input + `scope` + `mounts` +
   `resource_reservation`, the outputs of authorization), and
   *resolved-for-a-lane* (hop 5: + substrate handles). Those transitions are
   genuine. Because each state is modeled as "a request struct that looks like the
   others ± a field," the three real states blur into five, and hops 2–3 fall out
   as duplication *between* transitions rather than being transitions themselves.

3. **Fields accrete but never retire — dead transitional cruft rides along.**
   `trust_decision` is copied through hops 2–3 and dropped at hop 4; its own doc
   comment states `DefaultHostRuntime` **ignores it entirely** ("Legacy... kept
   for transitional request-shape compatibility... Callers must not rely on this
   field"). `idempotency_key` at hop 2 is "advisory... does not yet implement...
   kept so shape doesn't break when dedup is wired through downstream." Two of the
   fields every layer copies do nothing — one dead-past, one dead-future — and each
   new field multiplies across every type that mirrors it.

4. **A context bundle is composed, then decomposed.** Hops 2–3 carry
   `context: ExecutionContext` as one field; hop 4 explodes it into loose `scope` +
   `authenticated_actor_user_id`. Bundling, passing, then unpacking is two more
   struct shapes for the same information.

`CapabilityDispatchRequest` already living in `ironclaw_host_api` is the key
signal: the canonical shape *can* live at the bottom. And
`RuntimeAdapterRequest<'a, F, G>` — generic over closures with a lifetime — is a
complexity tell: the seam is parameterized for flexibility production wiring does
not exercise.

**Net:** of the five request types, hops 2 and 3 carry no new information (they
exist for Mechanism 1 and are padded by Mechanism 3); the other three are genuine
states that should be explicit and named. "~14 re-wraps" is really **~3 real
transitions + ~2 pure duplications + dead fields copied at every hop** — so ~40%
is pure duplication that a shared bottom-crate vocabulary removes outright, and
the rest becomes legible once the three states are named.

### 1.2 Authority is smeared across four layers, not centralized

The policy that actually matters (trust classification, credential pre-flight,
approval, obligations, resource reservation, run-state) is **not** in one
gateway. It is interleaved across four hops:

- `ironclaw_loop_host` — resume-mode validation, dispatch reservation, idempotency key;
- `ironclaw_host_runtime` — runtime-policy, trust eval, credential pre-flight, persistent-approval policy;
- `ironclaw_capabilities` — authorize, prepare/complete/abort obligations, run-state, approval blocking;
- `ironclaw_dispatcher` — reservation reconcile/release, registry/runtime-kind routing.

A change to ordering (e.g. credential-before-approval) can only be reasoned
about by holding all four crates in one's head. A mistake in the return-mapping
is silent: a recoverable `Ok(CapabilityOutcome::Failed)` and a run-terminating
`Err(HostRuntimeError)` are structurally identical — a footgun the loop-capability
contract docs record shipping three times.

### 1.3 `dyn` seams that exist for test doubles, not runtime variation

Verified production vs. test-double implementation counts, and how each seam is
stored:

| Seam | Prod impls | Test doubles | Stored as | Verdict |
| --- | --- | --- | --- | --- |
| `LoopCapabilityPort` (loop ↔ host) | 1 terminal (`HostRuntimeLoopCapabilityPort`) + decorators (hooks, surface filters, logging) | several | `Arc<dyn>` decorator chain | **keep** — the trust membrane; decoration is genuine composition |
| `HostRuntime` | 1 (`DefaultHostRuntime`) | 6 (`Recording*`, `Queued*`, `dummy_runtime`) | `Arc<dyn HostRuntime>` | collapse |
| `CapabilityDispatcher` | 1 (`RuntimeDispatcher<F, G>`) | 2 (`Recording*`, `Cancelling*`) | `Arc<dyn CapabilityDispatcher>` | collapse |
| `RuntimeAdapter<F, G>` | 5 (`Script`, `Mcp`, `FirstParty`, `Wasm`, + a `ServiceResolved` wrapper) | — | trait object behind the dispatcher | closed set → enum |
| `LlmProvider` | ~12 providers + decorators (`CircuitBreaker`, `Truncating`, `Swappable`, …); 32 impls total | — | `Arc<dyn>` | **keep** — genuine polymorphism |

Note: `CapabilityHost` is **not** a trait — it is a concrete generic struct
`CapabilityHost<'a, D>`, instantiated as `CapabilityHost<'a, dyn CapabilityDispatcher>`.
So the `dyn` on that path is the dispatcher it holds, not the host.

Three mechanisms produce the avoidable `dyn`:

1. **The trait exists to inject test doubles, not to vary production.**
   `HostRuntime` has one production impl and six test doubles; `CapabilityDispatcher`
   one and two. The trait + `Arc<dyn>` is paid on every production call so tests can
   swap in a `Recording*`/`Queued*`/`Cancelling*` fake. That is a real need met in an
   expensive way — the whole hot path takes vtable dispatch and every layer maintains
   a trait mirroring the concrete API, all to serve a test seam that a generic
   parameter or a single boundary fake would give more cheaply.

2. **Speculative replaceability.** "Replaceable everything" was applied to internal
   mediators, not only to the seams that actually vary. The single-prod-impl trait
   objects on the hot path have never been replaced in production. The second
   implementation is what should motivate re-introducing a trait.

3. **Generic *and* `dyn` — double indirection.** `RuntimeDispatcher<F, G>` and
   `RuntimeAdapter<F, G>` are generic over `RootFilesystem`/`ResourceGovernor` *and*
   used as trait objects (`CapabilityHost<'a, dyn CapabilityDispatcher>` holds a
   generic-turned-`dyn`). The path pays monomorphization complexity and vtable
   dispatch at once — a tell that the design never settled on static or dynamic.

Additionally, `RuntimeAdapter`'s impls are a **closed, enumerable set** — the four
runtime lanes plus a resolver wrapper — modeled as an open trait. New lanes are rare
and security-sensitive; new *tools* are data behind the existing lanes. An open trait
buys extensibility exactly where it is not needed.

### 1.4 A full store reimplementation per backend, per domain (the "local-specific structs")

Backend variation is modeled as **type** variation, not parameterization: each
backend is a separate struct that re-implements the whole domain trait by hand. The
turn store alone is two full implementations of the same semantics:

| Domain | In-memory impl | Durable impl | Reimplemented logic |
| --- | --- | --- | --- |
| turns | `InMemoryTurnStateStore` (`memory/mod.rs`, ~4,260 LOC) | `FilesystemTurnStateStore` (~1,710 LOC) | lease, active-lock, checkpoint, idempotency, events |
| processes | `InMemoryProcessStore` + `InMemoryProcessResultStore` (~230 LOC) | `FilesystemProcessStore` + `FilesystemProcessResultStore` (~920 LOC) | process lifecycle + result store |
| approvals | `InMemory{AutoApprove, PersistentApprovalPolicy, CapabilityPermissionOverride}Store` | matching `Filesystem*` (×3) | three separate approval stores |
| authorization | `InMemoryCapabilityLeaseStore` | `FilesystemCapabilityLeaseStore` | lease store |
| run_state | `InMemory{RunState, ApprovalRequest}Store` | matching `Filesystem*` (×2) | two run-state stores |

Every `InMemory*Store` is a local/test-only parallel implementation of logic that
also exists in the durable impl; a change to turn semantics (a new transition, a
lock rule) must be written **twice, in lock-step**, and production-facing domains add
libSQL + Postgres on top (2–4× per domain). The `InMemory*` structs are the literal
"structs specific to local."

Two mechanisms:

1. **Storage mechanism and domain logic are not separated.** The turn store's
   `filesystem_store/row_store/` layer (journal / delta / row materialization) is a
   *partial* gesture at that split, but it is turns-specific and does not unify
   in-memory vs filesystem vs the other domains. Without a shared backend seam, "what
   the turn state *is*" and "how bytes are persisted" are welded together, so each
   backend re-derives the former just to change the latter.

2. **The split is multiplied by the two work-unit lifecycles.**
   `ironclaw_turns`/`ironclaw_runner` (the leased `TurnRun`) and `ironclaw_processes`
   (the OS-subprocess) are **independent reimplementations** of the same six machinery
   layers — status enum, store trait, in-memory + filesystem stores, cancellation,
   eventing decorator, resource accounting — unified by no shared abstraction, and they
   diverge on recovery (turns recover expired leases; processes have an unimplemented
   reconciler). So the per-backend duplication is itself duplicated across two parallel
   lifecycles.

### 1.5 The last month agrees

- **≥510 merged PRs; 188 (37%) are `fix(...)`.** Backend fixes cluster on exactly
  these seams: channel/identity (22), turn/lease (16), capability/gate (12).
- A **daily automated "failure taxonomy" issue** and a `bug_bash_*` label stream
  exist — institutionalized after-the-fact bug harvesting.
- Review-iteration churn concentrates on the gate/hold/turn/auth seams (single
  "fix" PRs with dozens of review cycles, tens of files, thousands of lines).
- Open issues already target this surface: **#6168** ("Shrink the
  `ironclaw_reborn_composition` god-crate 24% → ~10%") and **#6144** item 1
  (a resource budget that is *defined as a field but never enforced at the call
  site* — the strongest form of "invariant lives at runtime, not in a type").
  Also directly on the capability path: **#6137** ("mixed-batch gate resume never
  redispatches the non-first gated call") and **#6138** (harness can't express a
  compound denied-gate + HTTP-egress-error scenario).

---

## 2. Root cause: one trust boundary, six crate boundaries pretending to be one

The security model has exactly one hard seam: the untrusted loop must not be able
to name secrets, the dispatcher, or the network. That is the loop ↔ host line.
Everything below it is trusted host code.

The implementation split that trusted region into ~6 crates and gave each:

- its **own request DTO**, because the boundary rule (`ironclaw_architecture`
  forbids importing "upward") means a layer that wants to reference a type must
  either depend on the crate that owns it or re-declare it — and re-declaring
  wins each time a layer needs one extra field;
- its **own `dyn` trait**, because "replaceable everything" was applied to every
  seam rather than only the ones with more than one implementation;
- its **own backend struct family**, because "composition mode changes which
  backends are legal" was expressed as parallel types instead of one backend
  parameter.

None of those six internal boundaries is a trust boundary.

### 2.1 The operating-system lens: mechanism vs policy

State it the way an OS does. A kernel provides **mechanism** — a small, stable set
of primitives (files, processes, address spaces, syscalls) — and is deliberately
*slow-moving*: adding an application does not change the kernel. **Policy** — which
app runs, what it may touch, how much it gets — lives outside, as configuration and
userland.

Reborn's kernel boundary must obey the same rule: **adding a feature, or a
deployment target, must not change the kernel.** Everything above collapses to one
violation of it — *policy encoded as kernel types*:

- **Deployment mode is a kernel enum.** `RuntimeProfile::{LocalDev, HostedDev,
  EnterpriseDev}` and `DeploymentMode` live in `ironclaw_host_api` — the vocabulary
  crate — and code across the host branches on them. Mode is the definition of
  policy; putting it in the kernel forces every mode to grow its own type family
  (the ~66-identifier `LocalDev*` shadow runtime, §4.4).
- **Storage medium is a domain type.** Each domain hand-writes an in-memory store
  *and* a durable store (§1.4); the medium — a deployment choice — is baked into the
  type instead of injected. The kernel should name *"a store"*; the config should
  pick the medium (§4.3).

So the simplification is one idea applied consistently: **the kernel is mechanism —
a small, frozen authority vocabulary plus a few real seams; everything that varies
by feature or deployment is policy, resolved to data at the composition edge.** The
`Invocation` / `Authority` / `Outcome` triple below is what feature-agnostic
mechanism looks like; §4.3–§4.5 remove the policy that leaked into types.

---

## 3. Proposed model: one payload, authority as a fold, one seam

Separate the **data plane** (the payload, which never changes shape) from the
**control plane** (authority decisions, which accrete in a side value).

```rust
// ── ironclaw_host_api (the bottom crate everyone already depends on) ──
struct Invocation { capability: CapabilityId, input: Json, scope: Scope, estimate: Estimate } // the ONE payload
struct Authority  { trust: TrustClass, approval: Option<ApprovalLease>,
                    reservation: Reservation, mounts: ScopedMounts }                          // accreted decisions
enum   Blocked    { Approval(GateRef), Auth(GateRef), Resource(GateRef) }
struct Outcome    { /* sanitized refs + summary */ }                                          // the ONE result

// ── the host kernel (trusted, below the loop seam) ──
fn authorize(inv: &Invocation, scope: &Scope) -> Result<Authority, Blocked>;  // ALL policy, ONE place
fn dispatch (inv: &Invocation, auth: &Authority, lane: &RuntimeLane) -> Outcome;
```

- The payload is `Invocation`, full stop. `RuntimeCapabilityRequest`,
  `CapabilityInvocationRequest`, `CapabilityDispatchRequest`, and
  `RuntimeAdapterRequest` disappear — they were `Invocation` plus a field that
  now lives in `Authority`, threaded by reference.
- Because `host_api` is the bottom crate, putting the vocabulary there *satisfies*
  the boundary rule (and Golden Boundary #1: `host_api` stays vocabulary-only)
  instead of fighting it. This finishes a move that `CapabilityDispatchRequest`
  already half-made.
- Authorization becomes a single `authorize()` body — the four scattered policy
  checks collapse into one reviewable function with visible ordering. This also
  removes the `Ok(Failed)` vs `Err(terminate)` ambiguity, because the single seam
  returns one `Result<Outcome, Blocked>` shape.

**Type count on a capability call: ~14 → 3.**

### 3.1 The three real states, named

This directly resolves the four mechanisms in §1.1. The five request types
collapse onto the three states the field diff identified; the two duplicates and
the dead fields disappear with them:

| Real state | Carried as | Replaces |
| --- | --- | --- |
| loop-expressed (pre-trust) | `Invocation` (input by ref, resume tokens) | `CapabilityInvocation` |
| authorized | `Invocation` + `&Authority` (trust, approval lease, reservation, mounts) | `RuntimeCapabilityRequest`, `CapabilityInvocationRequest`, `CapabilityDispatchRequest` |
| resolved-for-a-lane | `Invocation` + `&Authority` + resolved handles (package, descriptor, filesystem, governor) | `RuntimeAdapterRequest` |

- **Mechanism 1 (DAG re-declaration) is eliminated:** the one authorized shape is
  `Invocation` + `&Authority`, both defined in `host_api`, so `host_runtime` and
  `capabilities` reference it instead of each re-declaring it.
- **Mechanism 3 (dead fields) is eliminated:** `trust_decision` vanishes because
  trust is *computed inside* `authorize()` and lands in `Authority.trust`, never
  carried as a request field; `idempotency_key` is either implemented once in the
  authorization pass or deleted.
- **Mechanism 2 (blurred states) becomes explicit:** the three transitions are
  now named — `Invocation`, then `+ Authority`, then `+ resolved handles` — rather
  than five near-identical structs. Mechanism 4 (compose/decompose `context`) goes
  away because `Authority` carries `scope`/`actor` in one shape end to end.

---

## 4. The three moves, mapped to the three costs

### 4.1 Less DTO — `authorize`/`dispatch` over one payload

Define `Invocation` / `Authority` / `Outcome` in `ironclaw_host_api`. Every layer
references those instead of re-declaring; extra per-layer context is threaded by
reference (`&Authority`), not by re-wrapping the payload. The mirror-struct tax
goes to zero because nothing mirrors.

### 4.2 Less `dyn` — a trait earns a trait object only if it has ≥2 production impls or is the trust boundary

- **Keep** `LoopCapabilityPort` (the one trust membrane) and `LlmProvider`
  (genuine polymorphism).
- **Replace** `RuntimeAdapter`'s `dyn` with a closed `enum RuntimeLane`
  (`Wasm | Script | Mcp | FirstParty`). Adding a lane becomes a compile error
  until every `match` handles it. WASM extensions stay open — they are *data*
  behind the `Wasm` lane, not new lanes — so a closed lane set costs no real
  extensibility.
- **Delete** the `HostRuntime` and `CapabilityDispatcher` traits; make them
  concrete (or a single generic parameter resolved once at composition), and get
  the test seams they currently serve (§1.3, mechanism 1) from generics or one
  boundary fake instead of a production `Arc<dyn>`. `CapabilityHost` is **already**
  a concrete struct — collapsing `CapabilityDispatcher` to a concrete type removes
  the `dyn` it holds today (`CapabilityHost<'a, dyn CapabilityDispatcher>`), and its
  role folds into the `authorize` + `dispatch` pair.

**Hot-path `dyn`: 6+ → ~2, plus one lane enum.**

### 4.3 Delete every in-memory store; the storage seam already exists

The realization that reshapes this move: **the single storage seam is already in
the tree — it is `RootFilesystem`** (`ironclaw_filesystem`). It already has four
production-grade backends — `InMemoryBackend`, on-disk (`LocalFilesystem`),
`LibSqlRootFilesystem`, `PostgresRootFilesystem` — and the durable stores are
**already generic over it**: `FilesystemTurnStateStore<F>`,
`FilesystemProcessStore<F>`, `FilesystemCapabilityLeaseStore<F>`,
`FilesystemRunStateStore<F>`, `FilesystemAutoApproveSettingStore<F>`, and so on. The
`RowBackend` I earlier proposed inventing already exists and is already wired.

So the move is subtractive, not additive:

1. **Delete every hand-written `InMemory*Store`.** Tests instantiate the *same*
   store the deployment runs — `FilesystemTurnStateStore<InMemoryBackend>` — so
   "in-memory" stops being a store and becomes a **filesystem backend**
   (`InMemoryBackend`, which already implements `RootFilesystem`). One store
   implementation per domain, exercised in tests over the in-memory backend and in
   production over libSQL/Postgres. The ~4,260-LOC `InMemoryTurnStateStore` becomes
   deletable once `FilesystemTurnStateStore<InMemoryBackend>` covers its cases.

2. **Backend choice is deployment config, not a type.** Which `RootFilesystem`
   impl backs a run is one value in a `DeploymentConfig` fed to a single
   `build_runtime(config)`. "Local may reduce authority, never increase it" is a
   policy value, not a `Local*` code fork (§4.4).

Why the in-memory stores exist today: they predate the generic
`Filesystem*Store<F>` and were kept as the fast reference/test path. Now that a
first-class `InMemoryBackend: RootFilesystem` exists, they are redundant — a whole
second implementation per domain, kept alive only for tests that a memory-backed
filesystem serves for free. (Honest caveat: the turn store's ~4,260-LOC in-memory
impl is larger than the ~1,710-LOC filesystem one, so consolidation is *reconcile
then delete*, not a blind delete — but the target is one store, backend-injected.)

Open follow-on: consider a shared "leased recoverable work-unit" abstraction over
`TurnRun` and `ironclaw_processes` (§1.4, mechanism 2), or explicitly document why
the two lifecycles stay separate.

### 4.4 Eliminate `Local*`: deployment mode is policy, not a kernel type

**Local-dev is a policy, so it must be a policy *config* — a value — not an
implementation with its own structs and code.** That is the whole rule. Today the
tree has ~66 `LocalDev*` identifiers across **42 files in
`ironclaw_reborn_composition`** — a whole shadow local-dev *runtime*
(`LocalDevApprovalGatePolicy`, `LocalDevCapabilityLeaseStore`,
`LocalDevAutoApproveSettingStore`, `LocalDevMountProfile`, `LocalDevNetworkProfile`,
`LocalDevOutboundStores`, `LocalDevLoopCapabilityPortFactory`,
`LocalDevConstraintSource`, …). All of it collapses to a single config literal that
selects the *same* substrates every deployment uses:

```rust
// The entirety of "local dev" — data, not types. No LocalDev* structs, no code path.
const LOCAL_DEV: DeploymentConfig = DeploymentConfig {
    filesystem: Backend::InMemory,                 // or Backend::Disk
    approval:   ApprovalPolicy::AutoApproveEligible, // wider than hosted — a value
    network:    NetworkPolicy::AllowAll,           // vs Allowlist(..) in hosted
    process:    ProcessPolicy::HostProcess,        // vs Sandboxed
    owner_seed: Some(OwnerSeed::EnvToken),          // was the local_trigger_access module
};
```

`build_runtime(LOCAL_DEV)` wires the ordinary `FilesystemTurnStateStore<InMemoryBackend>`,
the ordinary approval/capability/lease substrates, and the ordinary ports — with
these values. There is no `LocalDev*` type because there is nothing local-dev
*implements*; it only *chooses*. The same is true of hosted and enterprise: each is a
`DeploymentConfig` constant, and the difference between them is data a reviewer can
read in one place, not a struct family spread across 42 files.

The `LocalDev*` identifiers fall in three buckets:

**Bucket 1 — deployment-mode-as-type leaks (delete).** They exist *only* because
`RuntimeProfile::{LocalDev, HostedDev, EnterpriseDev}` / `DeploymentMode` are kernel
enums that code branches on — the `DeploymentMode` doc comment literally says a
variant decides whether "`Local*` profiles" are allowed — so local-dev was built as
its own parallel wiring of approval, capability, lease, mount, network, and outbound
policy. **Fix:** resolve mode to policy *data* at the composition edge. The kernel
consumes the already-existing `EffectiveRuntimePolicy` (`ironclaw_runtime_policy`) —
an allowlist, an approval width, a sandbox requirement, a mount profile — and never
names a mode. The `local_trigger_access` module
(`LocalTriggerAccessSource::LocalDev{Env,Sso,Run}Bootstrap`) becomes "seed the owner
grant from config at boot," a policy value, not a module. Local-dev then wires the
*same* substrates with a config that selects `InMemoryBackend`/disk, a wider approval
width, and a permissive allowlist — no `LocalDev*` types.

**Bucket 2 — genuine resource/trust names mis-prefixed `Local` (rename).** Two types
describe a real resource or trust boundary and only *look* like mode leaks:

- `LocalFilesystem` → `DiskFilesystem` / `OnDiskBackend`: it names the storage
  medium (disk vs memory vs libSQL vs Postgres), a backend, not a deployment mode.
- `LocalHostProcessPort` → `HostProcessPort`: it names the trust boundary (a process
  on the host vs a sandboxed process). The trust-boundary baseline already requires
  that "sandbox/native/host names accurately describe the trust boundary"
  (`docs/reborn/2026-05-11-trust-boundary-stack-note.md`); `Local` obscures it,
  `Host` states it.

**Bucket 3 — false positives (leave).** `Locale`/`LocaleError` (localization),
`HookLocalId`, `LocalTraceSubmissionRecord` (this-node submission to Trace Commons;
rename to `NodeTraceSubmission*` only if convenient).

Enforce it with an `ironclaw_architecture` test: **no public type name contains
`Local`/`LocalDev`/`Hosted`/`Enterprise`.** A deployment mode is a config value that
selects backends and policy; it is never a type the kernel or a substrate names.

### 4.5 Name and freeze the kernel boundary

The boundary is what every feature and reviewer must hold in their head, so it is
the thing to make small and stable. Enumerated today it is:

- **Vocabulary — `ironclaw_host_api`: 21 files, ~124 public types** (67 structs, 55
  enums) and only 2 traits (`CapabilityDispatcher`, `RuntimeHttpEgress`). Concern
  files: `action, approval, audit, capability, capability_profile, decision,
  dispatch, dotted_id, error, host_port, http, ids, ingress, mount, path, resource,
  runtime_policy, runtime, scope, trust`.
- **The loop ↔ kernel seam — `AgentLoopDriverHost`**: ~13 fine-grained ports
  (`LoopRunInfoPort, LoopContextPort, LoopInputPort, LoopPromptPort, LoopModelPort,
  LoopCapabilityPort, LoopTranscriptPort, LoopCheckpointPort, LoopProgressPort,
  LoopCancellationPort`, plus model sub-ports).
- **The host mediators**: `HostRuntime`, `CapabilityHost`, `CapabilityDispatcher`
  (collapsing per §4.2).

Two cleanups make it a *kernel*:

1. **`runtime_policy.rs` does not belong in the vocabulary.** `DeploymentMode` /
   `RuntimeProfile` are deployment policy, not authority vocabulary. The kernel
   should speak `EffectiveRuntimePolicy` (resolved data) and let mode resolve at the
   edge (§4.4). Moving it out is the first concrete shrink.
2. **~124 types is too large for a "slow-moving" boundary.** Audit the 124 into
   (a) mode/policy types that belong at the edge, (b) product/feature-shaped types
   that leaked down, (c) genuinely neutral authority vocabulary — IDs, scopes, paths,
   decisions, mounts, resources, trust. Only (c) stays, and gets **frozen by a
   boundary test**, so a new feature that wants to add a `host_api` type must justify
   that it is authority vocabulary, not policy. That freeze is the operational
   meaning of "slow-moving kernel": the boundary changes when the *security model*
   changes, never when a feature ships.

---

## 5. Before → after

| | Now | After |
| --- | --- | --- |
| Types per capability call | ~14 | 3 (`Invocation` / `Authority` / `Outcome`) |
| Hot-path `dyn` seams | 6+ | 2 + 1 lane enum |
| Policy decision sites | 4 crates | 1 `authorize()` |
| Store impls per domain | 2–4 hand-written | 1 (`FsStore<F>`); in-memory is a backend, not a store |
| `InMemory*Store` structs | one per domain | 0 (tests use `FsStore<InMemoryBackend>`) |
| `LocalDev*` identifiers | ~66 across 42 files | 0 (one `DeploymentConfig` value) |
| Deployment modes | struct family (`Local*`/`Hosted*`) | `DeploymentConfig` constants (data) |
| `host_api` boundary | ~124 types incl. mode/policy | neutral authority vocab, frozen by test |

---

## 6. Invariants that must NOT change (and why this keeps them)

The simplification touches only trusted internals, so every security invariant
survives — several are strengthened:

1. **The loop cannot name privileged types.** Preserved: the one
   `LoopCapabilityPort` seam stays, and `Invocation` / `Outcome` remain ref-only.
2. **Authorize-before-dispatch, fail-closed.** *Strengthened*: it becomes one
   function, not four interleaved layers where an ordering bug can hide.
3. **`LoopExit` is refs-only and evidence-verified.** Untouched.
4. **Durable single-writer lease + terminal-on-expiry recovery.** Untouched;
   `TurnStore<B>` keeps the same records, it just stops reimplementing them per
   backend.
5. **Golden Boundaries (README).** `host_api` stays vocabulary-only (the new
   canonical types belong there by definition); substrates still do not depend
   upward; PostgreSQL/libSQL parity is *easier* to preserve because parity lives
   in one `RowBackend` impl pair, not per-domain.

Relationship to **#6168**: that issue sheds *product* code (host-side Slack) *out*
of the composition god-crate. This proposal sheds *mirror DTOs / traits* *in*
across the host-kernel chain. Same goal, orthogonal axes — no conflict.

---

## 7. Migration: incremental, not a rewrite

Land in verifiable slices with `ironclaw_architecture` boundary tests green at
every step. The moves have very different risk, so sequence by risk, not by section
order. Two are low-risk and independently valuable — start there:

**Slice A (lowest risk — delete in-memory stores).** The seam already exists
(§4.3), so this is subtractive: pick one domain (say approvals), delete its
`InMemory*Store`, repoint its tests to `Filesystem*Store<InMemoryBackend>`, confirm
green. Repeat per domain. No hot-path change, immediate deletion of a whole
implementation per domain, and it de-risks the store story before anything else.

**Slice B (low risk — `Local*` → config).** Add an `ironclaw_architecture` test
banning `Local`/`Hosted`/`Enterprise` in public type names; introduce the
`DeploymentConfig` constants; migrate the `LocalDev*` family in
`ironclaw_reborn_composition` to wire shared substrates from a config value. Mostly
mechanical, concentrated in one crate, and it directly shrinks the god-crate (#6168).

**Slice C (proof of concept for the DTO/`dyn` collapse — capability down-path,
first-party lane only).**

1. Define `Invocation` / `Authority` / `Outcome` in `ironclaw_host_api`.
2. Write `authorize()` as the single policy pass (initially delegating to the
   existing four checks, then inlining them).
3. Make the four mediators accept `(&Invocation, &Authority)` instead of
   re-wrapping — *without merging any crates yet*.
4. Measure: type count on one real call, `dyn` count, diff to boundary tests.

If C drops the type count with green boundary tests, roll it across the remaining
lanes, then the `RuntimeLane` enum, then the mediator-trait collapse (§4.2).

### Risks and honest caveats

- **The turn store consolidation is the trickiest store case.** The in-memory turn
  store is larger than the filesystem one and the runner-lease uses an in-memory
  overlay; consolidating onto `FilesystemTurnStateStore<InMemoryBackend>` is
  reconcile-then-delete, not a blind delete. Do the small domains (Slice A) first to
  build confidence, turns last.
- **Closed `RuntimeLane` enum** trades open extensibility for exhaustiveness — a
  deliberate choice: new *lanes* are rare and security-sensitive; new *tools* (the
  common case) are data behind the existing lanes and stay open.
- **Crate topology is out of scope here.** Reducing DTOs, `dyn`, and `Local*` is
  about types, not crate count; it can be done without merging crates. Crate merges
  have their own compile-parallelism trade-offs and are a separate decision.

---

## 8. Open questions

1. Should `TurnRun` and `ironclaw_processes` converge on a shared leased-work-unit
   abstraction, or stay deliberately separate with a documented rationale?
2. Does `Authority` accrete as one value, or should trust/approval/reservation
   remain separately-typed witnesses passed as a tuple to preserve sealed
   construction guarantees (see the trust-boundary stack note,
   `docs/reborn/2026-05-11-trust-boundary-stack-note.md`)?
3. Is `DeploymentConfig` expressive enough to encode every current
   LocalDev/HostedDev/EnterpriseDev difference as data, or do some differences
   genuinely need code paths?

---

## References

- `crates/Architecture.md` — Reborn kernel-boundary / substrate architecture thesis.
- `docs/reborn/2026-05-11-trust-boundary-stack-note.md` — trust-boundary baseline invariants.
- `docs/reborn/2026-04-25-storage-catalog-and-placement.md` — storage placement.
- `crates/ironclaw_host_api/` — the neutral vocabulary crate (target home for `Invocation`/`Authority`/`Outcome`); ~124 public types across 21 files (§4.5).
- `crates/ironclaw_host_api/src/runtime_policy.rs` — `DeploymentMode` / `RuntimeProfile`: the deployment-mode enums that leaked into the kernel vocabulary (§4.4–§4.5).
- `crates/ironclaw_runtime_policy/` — `EffectiveRuntimePolicy`: the resolved policy *data* the kernel should consume instead of a mode enum.
- `crates/ironclaw_filesystem/src/in_memory.rs` — `InMemoryBackend: RootFilesystem`: the existing seam that makes every `InMemory*Store` deletable (§4.3).
- `crates/ironclaw_reborn_composition/src/local_dev_capability_policy.rs` — `LocalDevConstraintSource` and the ~66-identifier `LocalDev*` shadow runtime to collapse to config (§4.4).
- `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` — boundary tests that must stay green; home for the "no `Local*` type names" and "freeze `host_api`" checks.
- Issues: #6168 (composition god-crate), #6144 (unenforced budget), #6137 / #6138 (gate-resume / capability-path).
