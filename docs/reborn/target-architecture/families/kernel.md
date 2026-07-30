# `crates/kernel/` — the authority perimeter

**Layer(s):** kernel · **Crates:** 9 — `ironclaw_trust`, `ironclaw_authorization`,
`ironclaw_approvals`, `ironclaw_resources`, `ironclaw_runtime_policy`, `ironclaw_capabilities`,
`ironclaw_processes`, `ironclaw_turns`, `ironclaw_host_runtime` · **Security posture:**
default-deny and fail-closed at every stage; a sealed authorization witness, a sealed effective
trust ceiling, and fingerprinted approval leases can be minted only inside this family — never
by a loop, an extension, or a product surface.

*This document specifies the target architecture as designed. Dispositions, migration
constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md),
[CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/kernel/
├── ironclaw_trust             trust ceilings (sealed)
├── ironclaw_authorization     default-deny grants & capability leases
├── ironclaw_approvals         exact-invocation consent
├── ironclaw_resources         reservation & quota accounting
├── ironclaw_runtime_policy    pure policy resolution & lane planning
├── ironclaw_capabilities      the CapabilityHost membrane
├── ironclaw_processes         lifecycle authority: journal & supervisor
├── ironclaw_turns             turn admission & exit validation
└── ironclaw_host_runtime      mediated services & the lane executor
```

## Role

The kernel is a security perimeter, not a crate. It is defined by what it mediates, not by how
much behavior it performs: every operation capable of affecting authority, isolation, durable
control-plane state, or sensitive data must cross a kernel-mediated port, regardless of which
crate implements that port. There is no single `ironclaw_kernel` crate, and there should not be
one — collapsing the perimeter into one crate would hide that it is made of independently
verifiable stages, each with its own fail-closed rule and its own sealed public surface.

`crates/kernel/` is that perimeter, physicalized as nine crates, each owning exactly one stage of
the pipeline that every privileged effect passes through: trust ceiling, authorization decision,
approval resolution, resource reservation, runtime-policy planning, the invocation membrane,
durable process lifecycle, turn admission, and mediated execution. A caller — a loop, an
extension, a product surface — never touches these stages directly; it calls the membrane, and
the membrane calls the rest. Everything the kernel does not need to mediate stays out of it:
agent-loop strategy, prompt assembly, mission orchestration, skill selection, and channel
presentation live above the kernel, running under whatever grants, mounts, leases, and budget the
kernel issues them.

## Boundaries — what makes this family distinct

- **vs `substrates/`** — substrate (filesystem, secrets, network, safety) is mechanism:
  backend-generic, policy-agnostic machinery. Kernel crates decide **whether** and **how** a
  substrate is touched, then call it; a substrate never decides authority.
- **vs `domains/`** — domains own record grammar for a durable thing; the kernel owns decisions
  about doing something. Domains answer "what is this record and is it valid"; the kernel
  answers "may this happen, and did it happen safely."
- **vs `lanes/`** — lanes execute already-authorized work, receiving a sealed witness and
  mediated services; they never authorize anything themselves. The kernel is the only place that
  witness can be minted.
- **vs `loop/`** — the loop tier is replaceable userland strategy with no inherent authority; it
  reaches the kernel only through typed ports, and the kernel never depends on loop code.
- **vs `app/`** — composition wires which concrete kernel services exist for a deployment,
  holding no authority logic of its own; the kernel decides, `app/` assembles the decider.
- **Why nine crates and not one.** Each stage is an independently consumed contract with its own
  fail-closed rule and its own sealing or multi-implementation shape. Collapsing them into one
  crate would trade a compiler-provable dependency boundary — a policy engine whose private
  mutators are invisible outside its own crate, a reservation governor whose platform-specific
  dependency never reaches the invocation membrane — for a discipline enforced only by review.

## The effect pipeline and stage ownership

Every privileged effect passes through the same ordered stages. Two crates bracket the pipeline
as durable admission and lifecycle authorities; the rest compose the membrane itself.

| Stage | What happens | Owning crate |
|---|---|---|
| Admission | a request becomes durable, admitted work under one-active-run-per-thread and idempotency guarantees | `ironclaw_turns` |
| Claimed execution | admitted work is claimed, leased, and heartbeat-tracked through to a terminal state | `ironclaw_processes` |
| Trust ceiling | requested trust resolves to a host-validated effective ceiling | `ironclaw_trust` |
| Authorization | the ceiling and the caller's grants resolve to allow, deny, or require-approval | `ironclaw_authorization` |
| Approval | a require-approval verdict resolves to a scoped, fingerprinted lease or a denial | `ironclaw_approvals` |
| Reservation | estimated cost and capacity are reserved before work starts and reconciled after | `ironclaw_resources` |
| Policy planning | deployment and organization policy select the runtime lane and enforcement posture | `ironclaw_runtime_policy` |
| The membrane | every prior stage folds into one decision, sealed into a witness only the membrane can mint | `ironclaw_capabilities` |
| Mediated execution | the witness authorizes exactly one lane call, with a restricted mount view, staged secrets, and scoped egress | `ironclaw_host_runtime` |

`ironclaw_turns` and `ironclaw_processes` are the two admission/lifecycle authorities: turns
decides whether work is durably admitted at all; processes decides how admitted work is claimed,
tracked, and brought to a terminal state, foreground turn or background invocation alike.

## What belongs here / What never belongs here

**Belongs:** decisions, leases, reservations, mediation, lifecycle authority, dispatch
composition — a decision value, a capability lease, a resource reservation, an obligation
handler, a process or turn transition, the sealed witness itself.

**Never belongs here:** product UX, loop strategy, vendor-specific behavior, lane execution
mechanics, or storage-backend implementations. No crate in this family renders a prompt, chooses
a delivery target, branches on a vendor protocol, spawns a container directly, or persists raw
secrets, host paths, or backend error detail in anything it emits.

## Dependency direction

Kernel crates depend on the neutral authority vocabulary every layer shares (identifiers, scopes,
capability and decision types), on the substrate family for mediated storage, credential, and
egress mechanics, on the events family for durable audit, on the lane family where a closed
executor must construct lane adapters, and on each other along the stage order above. Kernel
crates never depend on `loop/`, `product/`, or `app/`, with one deliberate inversion: a port
defined inside the kernel and implemented by a higher layer — process-kind execution is defined
here and registered by whichever crate owns that kind of work. The dispatch port is neutral
contracts vocabulary (`host_api`); the membrane is its production implementation.

Every other family depends on this one, directly or through a typed port; nothing in this family
depends back.

## Crates

### `ironclaw_trust`

- **Purpose:** resolves a manifest's requested trust into the host-validated effective ceiling
  every authorization decision consumes.
- **Pipeline stage:** trust ceiling.
- **Owns:** the policy engine that evaluates package identity, source, and requested authority
  under layered host policy; synchronous invalidation, so a trust downgrade invalidates affected
  grants before any subsequent side effect can occur under the superseded ceiling.
- **Never contains:** capability registration, grant issuance, dispatch, secret custody, or
  runtime execution — this crate answers only "what is this package allowed to be trusted with,"
  never "what may it do right now."
- **Public surface:** an effective-trust type sealed so its privileged variants can be
  constructed only by this crate's own policy evaluation — never deserialized from a wire type,
  never constructed by a caller, however privileged.
- **Depends on:** the neutral authority vocabulary only.
- **Never depends on:** any other kernel crate, any substrate, any lane.
- **Security & authority role:** the authority-ceiling gate. A user-installed package cannot
  fabricate a privileged ceiling by any means available to it, and a ceiling by itself grants
  nothing — a loop or extension running at an elevated ceiling still needs every later stage's
  explicit authorization, exactly like any other caller.
- **Why a separate crate:** the seal is a property of crate-scoped visibility. If policy
  evaluation were a module inside a larger crate, its private mutators would be reachable from
  every other module in that crate; a dedicated crate is what makes "only this code may change
  trust state" actually true rather than a convention agents are expected to honor.

### `ironclaw_authorization`

- **Purpose:** matches a caller's grants and active leases to the requested effect, default-deny.
- **Pipeline stage:** authorization decision.
- **Owns:** grant matching under the effective trust ceiling; the capability-lease lifecycle,
  including the single-winner claim that lets a resumed, previously-approved call safely
  re-enter without granting a second dispatch in parallel.
- **Never contains:** approval resolution, runtime dispatch, obligation execution, or any policy
  content beyond grant-vs-context matching — this crate answers only "does a grant exist that
  covers this," never "should one be created."
- **Public surface:** a capability lease sealed to carry an invocation fingerprint, so a lease
  issued for one exact input can never authorize a different one; the lease's status transitions
  are themselves part of the public surface, since callers coordinate resume attempts through
  them.
- **Depends on:** `ironclaw_trust`, for the effective ceiling every grant must satisfy.
- **Never depends on:** `ironclaw_approvals`, `ironclaw_capabilities`, `ironclaw_processes`,
  `ironclaw_resources`, or anything above the kernel — approval resolution depends on this
  crate, never the reverse.
- **Security & authority role:** the default-deny gate; the sole owner of the lease state every
  fingerprinted approval rides on.
- **Why a separate crate:** authorization is a distinct, independently testable decision from
  consent resolution — matching a static grant and resolving a one-off human decision are
  different questions with different failure modes, and only one of them should be able to mint
  a lease.

### `ironclaw_approvals`

- **Purpose:** resolves a require-approval verdict into a scoped, fingerprinted lease or a
  durable denial — the crate that turns a human or policy decision into something the membrane
  can act on.
- **Pipeline stage:** approval resolution.
- **Owns:** durable approval-request and gate records; the fail-closed resolution order that
  durably records a decision before it issues the lease that decision authorizes; persistent,
  scope-bounded "always allow" policy for capabilities whose manifest explicitly permits reuse.
- **Never contains:** UI rendering, notification delivery-target selection, or dispatch before a
  matching-fingerprint lease is validated and claimed. Choosing who to notify and how is a
  product concern that calls into this crate, never the reverse.
- **Public surface:** the approve/deny decision types that hand a fingerprinted lease to the
  membrane; a reusable-approval record shape distinct from a one-shot lease, so a capability a
  manifest marks reusable does not have to re-prompt on every call.
- **Depends on:** `ironclaw_authorization`, for the lease it issues into.
- **Never depends on:** `ironclaw_capabilities`, `ironclaw_host_runtime`, `ironclaw_processes`,
  `ironclaw_turns` — the membrane depends on this crate, never the reverse.
- **Security & authority role:** the human/policy consent authority — the only place a pending
  decision becomes either a scoped lease or a terminal denial. A denial is durable and final for
  that request; a caller must raise a new request rather than retry a denied one.
- **Why a separate crate:** consent resolution is a distinct authority from grant matching, with
  its own durability and ordering guarantees; folding it into authorization would blur "does
  this grant apply" with "did a human agree to this."

### `ironclaw_resources`

- **Purpose:** governs cost, quota, and scarce runtime capacity through a reserve, execute,
  reconcile-or-release protocol.
- **Pipeline stage:** reservation, at dispatch and once more at completion.
- **Owns:** the reservation protocol and its accounting across every budget dimension a
  deployment cares about — cost, tokens, wall-clock, bytes, egress, process count, concurrency;
  a pause-threshold approval gate for reservations that cross an operator-configured ceiling,
  kept as a distinct machine from capability approval.
- **Never contains:** runtime or process execution logic, or best-effort accounting anywhere the
  contract requires a fail-closed denial instead — a reservation failure is always treated as a
  denial, never as a reason to proceed and true up later.
- **Public surface:** a reservation governor implemented by more than one backing store, so the
  identical protocol runs in memory, on disk, or through a durable backend without its callers
  knowing which; a receipt type that closes the loop between an estimate and what a completed
  invocation actually spent.
- **Depends on:** the neutral authority vocabulary only.
- **Never depends on:** any crate that calls it — `resources` is called by the membrane, by
  process lifecycle, and by mediated execution; it never reaches into them.
- **Security & authority role:** no costed or quota-limited work executes without an active
  reservation, and a storage failure denies exactly like a quota denial.
- **Why a separate crate:** it is the only stage with more than one production backing
  implementation behind one protocol, and its platform-specific concerns must never leak into
  any other kernel crate's build.

### `ironclaw_runtime_policy`

- **Purpose:** resolves deployment mode, runtime profile, and organization policy into the
  effective runtime policy every dispatch enforces.
- **Pipeline stage:** policy planning, folded into the membrane's own decision rather than a
  separate call.
- **Owns:** monotonic policy resolution — deployment and organization policy may only reduce
  requested authority, never increase it; per-capability lane planning, selecting which
  execution lane a given invocation is allowed to use; an explicit opt-in requirement for any
  profile that relaxes default safety posture, so relaxed enforcement is always a deliberate,
  recorded choice rather than a silent default.
- **Never contains:** process startup, dispatch, or any product-facing strategy beyond profile
  resolution — it answers "which lane and posture," never "run it."
- **Public surface:** an effective-runtime-policy type with exactly one sanctioned producer; a
  value constructed any other way is untrusted by contract, so downstream stages never need to
  re-derive or second-guess a policy they receive.
- **Depends on:** the neutral authority vocabulary only.
- **Never depends on:** any other crate — this stage is pure computation with no side effects of
  its own.
- **Security & authority role:** the policy-math gate feeding the membrane; deterministic and
  reproducible, so an audit record can name the exact policy that gated an invocation.
- **Why a separate crate:** the only dependency-free stage — pure policy math over neutral vocabulary — and keeping it a leaf means policy resolution stays consumable and testable without the membrane's full service cone.

### `ironclaw_capabilities`

- **Purpose:** the caller-facing invocation service — the membrane every privileged effect must
  cross.
- **Pipeline stage:** the membrane; folds trust, authorization, approval, and reservation into
  one sealed decision, then routes dispatch.
- **Owns:** six workflows — invoke, resume, resume-after-auth, decline-auth, resume-spawn, and
  spawn — each running the same fold before any side effect; the obligation seam that hands
  mediated execution a restricted mount view and a prepared reservation; the runtime dispatcher
  that routes a sealed witness to its bound lane and rejects any mismatch between the witness's
  sealed lane and the resolved binding.
- **Never contains:** process lifecycle or result storage, parallel dispatch paths, or any side
  effect that has not passed through authorization, approval, and obligation preparation in that
  order.
- **Public surface:** the sealed authorization witness — the one artifact that proves a specific
  effect was authorized, mintable only by this crate's own fold, consumed exactly once by
  dispatch.
- **Depends on:** the stage crates before it — trust, authorization, approvals, resources,
  runtime_policy — never the mediated-services crate or the turn coordinator.
- **Never depends on:** `ironclaw_host_runtime` — the direction is strictly the other way — nor
  any loop, product, or app crate, nor any lane crate directly.
- **Security & authority role:** the membrane itself. No loop, extension, or product surface
  reaches a privileged effect any other way.
- **Why a separate crate:** the sealing invariant — that only this crate may produce an
  authorization witness — is enforced by a dedicated boundary test; one crate gives that test
  exactly one thing to check, and keeps the six-workflow fold reviewable as a single unit
  instead of a decision scattered across a boundary an attacker would probe first.

### `ironclaw_processes`

- **Purpose:** the durable lifecycle authority for every piece of host-tracked work, foreground
  or background.
- **Pipeline stage:** claimed execution.
- **Owns:** a row-native journal recording process identity, lineage, and status;
  `ProcessSupervisor`, which claims, leases, heartbeats, and recovers registered work,
  containing panics and driving orderly shutdown; process kinds registered by the crate that
  owns that kind of work — `ironclaw_turn_runner` registers the agent-turn kind, `ironclaw_host_runtime`
  registers the capability-invocation kind; child-process relationships recorded as edges in the
  same journal rather than a side table; checkpoint payloads stored as journal rows; process
  input treated as immutable once accepted.
- **Never contains:** capability authorization or approval policy, and no opinion about whether
  a caller may spawn — only about what happens once it has.
- **Public surface:** a process-executor port that a registering crate implements to claim its
  kind of work; a process-dependency port for recording and querying child relationships.
- **Depends on:** `ironclaw_resources`, to reserve capacity for work it tracks; the neutral
  authority vocabulary.
- **Never depends on:** `ironclaw_capabilities`, `ironclaw_host_runtime`, `ironclaw_turns`,
  `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_trust` — nothing above or beside it
  reaches back down except through the ports it defines.
- **Security & authority role:** the claimed-execution authority; a terminal status is written
  once and never overwritten by a late completion.
- **Why a separate crate:** one journal answering "what is this work doing right now" for every
  kind of host-tracked work — foreground turns and background capability invocations alike — is
  the entire reason this stage exists as a single authority rather than several.

### `ironclaw_turns`

- **Purpose:** the turn admission kernel — the durable entry point for a unit of conversational
  work.
- **Pipeline stage:** admission, and the exit-claim boundary where a loop's reported outcome is
  validated before it becomes durable truth.
- **Owns:** a coordinator enforcing one active run per thread and request idempotency; exit
  validation, which treats a loop's reported completion, failure, or block as a claim, never as
  truth, until it is checked using host-minted evidence. Turn and run state are not a second
  durable store: they are a typed projection over the process journal that `ironclaw_processes`
  owns, so a turn's lifecycle and its underlying process's lifecycle can never diverge into two
  different answers to "is this still running."
- **Never contains:** raw dispatch or runtime handles, raw prompt or tool-input content,
  secrets, host paths, or channel-identity parsing.
- **Public surface:** the coordinator's accept/resume/cancel surface; the exit-evidence port a
  loop's host adapter must satisfy before an exit claim is accepted.
- **Depends on:** `ironclaw_processes`, for the journal its state projects over; the neutral
  authority vocabulary.
- **Never depends on:** any dispatch, runtime, or lane crate — admission and exit validation
  never touch execution directly.
- **Security & authority role:** one active run per thread, and a structural guarantee that a
  loop cannot talk itself into a durable state transition it was not granted.
- **Why a separate crate:** admission and exit validation answer "is this turn allowed to keep
  running," a fail-closed authority question distinct from "is this one capability call
  allowed" — the two questions have different callers and different blast radii, and conflating
  them would let a turn-level concern block a capability-level one or vice versa.

### `ironclaw_host_runtime`

- **Purpose:** the kernel's mediated-execution service — the boundary between an authorized
  witness and an actual runtime lane.
- **Pipeline stage:** mediated execution, plus completing the obligations the membrane prepared.
- **Owns:** the obligations engine — audit before and after, network-policy staging, one-shot
  secret staging and consumption, mount restriction, resource-ceiling enforcement, output
  redaction and limits; mediated egress and secret staging, the only path through which a lane
  receives network access or credential material, always scoped and always consumed exactly
  once; the closed lane executor, which invokes only the lane a sealed witness names and nothing
  else; dispatch composition, assembling the membrane from the kernel's other services for a
  given deployment; resolution of the memory service through its provider-neutral contract — the
  concrete provider arrives from assembly and is never named here.
- **Never contains:** vendor-specific tool-handler implementations, sandboxed process or
  container mechanics, product workflow, or any driver dependency beyond what mediation itself
  requires.
- **Public surface:** the obligation-handler contract the membrane calls into; the mediated
  egress port every runtime lane's network access flows through.
- **Depends on:** the rest of the kernel, to compose and drive the membrane; the substrate
  family, for the storage, credential, and network mechanics it mediates; the lane family, to
  construct the closed executor's adapters; the events family, for durable audit.
- **Never depends on:** any extension, product, or app crate; any lane's execution mechanics
  beyond the adapter surface it mediates through.
- **Security & authority role:** turns a sealed witness into exactly one lane call under a
  restricted mount, staged credentials, and scoped egress, then turns that lane's raw output
  into redacted evidence appended to the durable audit log.
- **Why a separate crate:** the obligation-completion, lane-execution, and evidence-sanitization
  sequence is one atomic operation every runtime lane depends on identically; separating it into
  more crates would scatter one security-critical fold across boundaries without adding
  isolation.

## Family AGENTS.md requirements

`crates/kernel/AGENTS.md` states, for every agent working in this family:

- the pipeline diagram and stage-ownership table above, so "which crate owns this effect" never
  requires re-deriving from source;
- **no stage skipping — first-party is a ceiling, not a bypass.** A higher trust ceiling still
  requires explicit grants, scoped mounts, leases, budget, and obligation handling through the
  same membrane every other caller uses; nothing shipped by the project and nothing running at
  an elevated trust class may reach a privileged effect by any other path;
- each crate's one-line stage assignment, so a new kernel contribution can be placed correctly
  on first read: `trust` → ceiling, `authorization` → decision, `approvals` → consent,
  `resources` → reservation, `runtime_policy` → planning, `capabilities` → the membrane,
  `processes` → lifecycle, `turns` → admission, `host_runtime` → mediated execution;
- the family's dependency direction, restated as a check: kernel crates depend on contracts,
  substrate, events, the lane family where the closed executor needs adapters, and each other
  only along the stage order — never on `loop/`, `product/`, or `app/`, which reach back only
  through a defined port or a registered executor;
- the crate-boundary test every new addition to this family must pass: name your stage, name
  your fail-closed rule or your reason for more than one implementation — otherwise it is a
  module of one of the nine, not a tenth crate.
