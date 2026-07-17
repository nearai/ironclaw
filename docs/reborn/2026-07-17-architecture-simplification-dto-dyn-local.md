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

### 1.3 `dyn` seams with one production implementation

Verified production implementation counts (test doubles excluded):

| Seam | Prod impls | Notes |
| --- | --- | --- |
| `LoopCapabilityPort` (loop ↔ host) | 1 | the real trust membrane — keep |
| `HostRuntime` | 1 (`DefaultHostRuntime`) | rest are `Recording*`/`Queued*` test doubles |
| `CapabilityDispatcher` | 1 (`RuntimeDispatcher`) | rest are `Recording*`/`Cancelling*` test doubles |
| `RuntimeAdapter` | 4 lanes (WASM/script/MCP/first-party) | a closed set, not open polymorphism |
| `LlmProvider` | many | genuine polymorphism — keep |

Four of these seams pay `dyn` cost for polymorphism that production never uses.
Replaceability that never happens should be deleted; the second implementation
is what should motivate re-introducing a trait.

### 1.4 A parallel store tree per domain (the "local-specific structs")

Each durable domain ships an in-memory implementation *and* a filesystem
implementation *and* (where production-facing) libSQL + Postgres — each a full
reimplementation of the domain logic, not a thin backend swap. Examples:
`InMemoryTurnStateStore` + `FilesystemTurnStateStore` (`ironclaw_turns`),
`InMemoryProcessStore` + `FilesystemProcessStore` (`ironclaw_processes`), and the
same shape for approvals, authorization, and run-state. The `InMemory*` and
local-dev wiring are the literal "structs specific to local," and they double
(or quadruple) the surface every persistence change must touch in lock-step.

Separately, `ironclaw_turns`/`ironclaw_runner` (the leased `TurnRun` work-unit)
and `ironclaw_processes` (the OS-subprocess work-unit) are **two independent
reimplementations** of the same six machinery layers — status enum, store trait,
in-memory + filesystem stores, cancellation, eventing decorator, resource
accounting — unified by no shared abstraction, and they even diverge on recovery
(turns recover expired leases; processes have an unimplemented reconciler).

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
- **Delete** the `HostRuntime`, `CapabilityDispatcher`, and `CapabilityHost`
  traits; make them concrete types (or generic parameters resolved once at
  composition). `CapabilityHost` becomes the `authorize` + `dispatch` pair.

**Hot-path `dyn`: 6+ → ~2, plus one lane enum.**

### 4.3 No structs specific to local — backend-generic stores, and "local" as a config value

- Make each durable store generic over a single storage backend:

  ```rust
  struct TurnStore<B: RowBackend> { backend: B }        // domain logic written ONCE
  enum Backend { Memory(MemBackend), Sqlite(..), Postgres(..) } // 3 backends shared by ALL domains
  ```

  The domain logic (turns, processes, approvals, authorization, run-state) lives
  once; `Memory` / `Sqlite` / `Postgres` are three small `RowBackend`
  implementations reused everywhere. This deletes the entire `InMemory*` /
  `Filesystem*` parallel tree. The existing `filesystem_store/row_store/journal`
  layer in `ironclaw_turns` already gestures at this abstraction — this finishes
  it rather than inventing it.

- Delete **composition mode as a type**. `LocalDev` / `HostedDev` / `EnterpriseDev`
  become one `DeploymentConfig { backend, approval_policy, network_policy }`
  value fed to a single `build_runtime(config)`. "Local may reduce authority,
  never increase it" becomes a policy value enforced in one place — not a fork in
  the struct graph where a local shortcut can silently leak into production.

- Consider a shared "leased recoverable work-unit" abstraction over `TurnRun` and
  `ironclaw_processes`, or explicitly document why the two lifecycles stay
  separate. (This split is a layering choice, not migration debris — it will not
  evaporate when v1 retires — so it deserves an explicit decision.)

---

## 5. Before → after

| | Now | After |
| --- | --- | --- |
| Types per capability call | ~14 | 3 (`Invocation` / `Authority` / `Outcome`) |
| Hot-path `dyn` seams | 6+ | 2 + 1 lane enum |
| Policy decision sites | 4 crates | 1 `authorize()` |
| Store impls per domain | 2–4 | 1 generic + 3 shared backends |
| Deployment modes | struct family | 1 `DeploymentConfig` value |

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

This is the hottest path in the system, so it must land in verifiable slices with
`ironclaw_architecture` boundary tests green at every step.

**First slice (proof of concept):** the capability down-path for the first-party
lane only.

1. Define `Invocation` / `Authority` / `Outcome` in `ironclaw_host_api`.
2. Write `authorize()` as the single policy pass (initially delegating to the
   existing four checks, then inlining them).
3. Make the four mediators accept `(&Invocation, &Authority)` instead of
   re-wrapping — *without merging any crates yet*.
4. Measure: type count on one real call, `dyn` count, and the diff to boundary
   tests.

If the slice drops the type count on one call with green boundary tests, roll it
across the remaining lanes, then tackle the `RuntimeLane` enum, then the
`RowBackend` store collapse (hardest — the backend trait must express
transactions/locks, since in-memory and durable stores differ in more than
storage today), then the `DeploymentConfig` collapse.

### Risks and honest caveats

- **`RowBackend` is the hard part.** In-memory vs durable stores differ in
  locking and overlay semantics (e.g. the runner-lease in-memory overlay in
  `ironclaw_turns`), so the backend trait must model transactions and locks, not
  just get/put. If that abstraction gets leaky, the collapse is not worth it.
- **Closed `RuntimeLane` enum** trades open extensibility for exhaustiveness. This
  is a deliberate choice: new *lanes* are rare and security-sensitive; new *tools*
  (the common case) are data behind the existing lanes and stay open.
- **Crate topology is out of scope here.** Reducing DTOs and `dyn` is about types
  and traits, not crate count; it can be done without merging crates. Merging
  crates has its own compile-parallelism trade-offs and should be a separate
  decision.

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
- `crates/ironclaw_host_api/` — the neutral vocabulary crate (target home for `Invocation`/`Authority`/`Outcome`).
- `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` — boundary tests that must stay green.
- Issues: #6168 (composition god-crate), #6144 (unenforced budget), #6137 / #6138 (gate-resume / capability-path).
