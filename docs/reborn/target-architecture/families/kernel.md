# Family: `crates/kernel/` — the authority perimeter

**Layer(s):** `kernel` (nine crates already declare `layer = "kernel"` today; `ironclaw_processes`
re-layers in from `runtimes`) · **Crates (target):** 9 (+1 transitional) — `ironclaw_trust`,
`ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_resources`, `ironclaw_runtime_policy`,
`ironclaw_capabilities`, `ironclaw_processes`, `ironclaw_turns`, `ironclaw_host_runtime`
(+ `ironclaw_run_state`, transitional — deleted once `#6696`-or-equivalent lands) ·
**Security posture:** default-deny and fail-closed at every stage; the sealed `Authorized`
witness, the effective `TrustClass`, and fingerprinted `CapabilityLease`s can be minted only
inside this family — never by a loop, extension, or product crate.

## Identity — what this family IS

`docs/reborn/contracts/kernel-boundary.md:11` is the source of truth for what "kernel" names in
this codebase:

> "The Reborn kernel is the security perimeter. It is defined by what it mediates and secures,
> not by how much product behavior it performs."

And, explicitly, at `kernel-boundary.md:20`:

> "There is no requirement to create an `ironclaw_kernel` crate. The contract is about the
> boundary: privileged operations must cross kernel-mediated ports, regardless of which
> concrete crate wires them."

`crates/kernel/` is that boundary made physical, deliberately as a **family of nine focused
crates plus one transitional crate**, not one mega-crate (PROPOSAL §6.5 preamble; invariant #1,
§3). `kernel-boundary.md:17` already names the terminology this family embodies: `kernel` is the
architectural security boundary, `ironclaw_host_runtime` is "current concrete composition crate
for kernel-facing services/adapters" — one member of the family, not its synonym. The kernel
mediates the operation list in `kernel-boundary.md:28-46`: capability invocation/resume/spawn,
authorization and grant matching, trust-class policy, obligation preparation, approval/lease
coordination, run-state coordination, one-active-run-per-thread, filesystem mount/path
authority, network policy and hardened egress, secret lease/one-shot consumption, resource
reservation/reconciliation/quota, redaction/leak obligations, prompt-injection write-safety,
durable audit/event append, and process lifecycle authority. Every item on that list has exactly
one crate in this family as its concrete home, enumerated in the pipeline map below. Everything
in `kernel-boundary.md §3` — agent-loop strategy, prompt assembly, routine/mission
orchestration, skill selection, channel UX, profile presentation, reference-loop implementations
— is explicitly **not** kernel and lives in `loop/` or `product/` instead.

## What makes it distinct

- **vs `substrate/`** — substrate (`filesystem`, `secrets`, `network`, `safety`) is *mechanism*:
  backend-generic, policy-agnostic, reusable machinery the kernel mediates access to. Kernel
  crates are *mediation and authority*: they decide **whether** and **how** a substrate gets
  touched, then call it — `host_runtime`'s obligations decide when a secret lease gets consumed
  and stage the handoff; the kernel never re-implements a substrate's job (§6.5: "does not
  belong": storage backends), and a substrate never decides authority.
- **vs `domains/`** — domains (`threads`, `conversations`, `triggers`, `llm`) own record grammar
  for a durable *thing*; kernel crates own *decisions about doing something* (authorize,
  approve, reserve, dispatch). Domains answer "what is this record and is it valid"; kernel
  answers "may this happen, and did it happen safely." `turns` sits closest to this line but
  earns kernel placement because admission and exit-claim *validation* are fail-closed authority
  decisions, not record-keeping.
- **vs `lanes/`** — lanes (`wasm`, `mcp`, `sandbox`) **execute** already-authorized work; they
  receive a sealed `Authorized` witness and never authorize anything themselves (§6.6: "Lanes...
  never authorize, never own product behavior, never hold ambient network/secrets"). The kernel
  is the *only* place that mints the witness a lane may consume.
- **vs `loop/`** — the loop tier is replaceable userland strategy (§6.7: "Nothing here is
  trusted with authority"); the kernel is the membrane it must cross. `loop_contracts` is the
  typed fence between them — loops depend on ports, never kernel crates directly, and the kernel
  never depends on loop crates (§8.2: `kernel`→`loops` is `✗`, with one inversion for the
  processes/supervisor executor port).
- **vs `app/`** — composition wires *which* concrete kernel services exist for a deployment
  (profile/backend selection, construction order); it holds zero authority logic. The kernel
  decides; `app/` assembles the decider.
- **Why nine crates and not one.** Each pipeline stage is an independently consumed contract
  with its own fail-closed rule and its own multi-impl or security-seal shape (§6.5 preamble).
  Merging them would trade a compiler-provable dependency cone — today `authorization` cannot
  see `host_runtime`'s Docker/TLS deps, `trust`'s `pub(crate)` seal is invisible outside its own
  crate, `resources`' Windows-only file-lock dep never reaches `capabilities` — for module
  discipline enforced only by review: the "mass pooling inside a crate" failure Strategy C was
  rejected for (§4: "It erases mechanically enforced security seams").

## The effect pipeline and stage ownership

PROPOSAL §7's trust-transition walkthrough (T1–T8) names the kernel as the heart of T3 through
T7; the two admission/lifecycle authorities (`turns`, `processes`) bracket the membrane on
either side:

| Stage | Transition | Owning crate(s) | What crosses |
|---|---|---|---|
| Durable admission | **T3** | `ironclaw_turns` | `TurnCoordinator` persists the turn/run under the one-active-run-per-thread lock; idempotent accept/resume/cancel |
| Claimed execution | **T4** | `ironclaw_processes` (background work); `ironclaw_turns` + `ironclaw_runner` (foreground turns, loop-tier) | lease claim, heartbeat, terminal-state protection; the loop receives only `loop_contracts`-scoped ports for the claimed run |
| Trust ceiling | **T5a** | `ironclaw_trust` | requested → effective `TrustClass`, sealed |
| Authorization decision | **T5b** | `ironclaw_authorization` | default-deny grant/lease matching |
| Approval resolution | **T5c** | `ironclaw_approvals` | exact-invocation human/policy consent, out of band |
| Obligations + reservation | **T5d** | `ironclaw_host_runtime` (obligation composition); `ironclaw_resources` (reservation) | audit-before, network-policy staging, one-shot secret staging, mount narrowing, resource reservation |
| Policy planning | **T5e (in-fold)** | `ironclaw_runtime_policy` | lane selection, budget/approval-bypass classification — runs *inside* `capabilities::authorize()`, not as a separate hop |
| The membrane | **T5** | `ironclaw_capabilities` | folds T5a–T5e; mints the sealed `Authorized` witness |
| Dispatch | **T6a** | `ironclaw_capabilities` (`RuntimeDispatcher`) | sealed-lane verification, routes to the bound adapter |
| Mediated execution | **T6b** | `ironclaw_host_runtime` (closed lane executor) | narrowed mounts, staged one-shot secrets, policy-scoped egress |
| Safe evidence | **T7** | `ironclaw_host_runtime` (sanitize + obligation completion) | redaction, output limits, resource reconciliation, durable audit append |
| *(transitional)* | **T5b′** | `ironclaw_run_state` | today records `BlockedApproval`/`BlockedAuth` current-state during T5; dissolves into `approvals` + a `processes` projection under `#6696` |

`ironclaw_capabilities` is the only crate that appears at two rows because it is the fold *and*
the dispatch entry point — everything from T5a through T6a happens inside one call into
`CapabilityHost`, which is exactly why that crate's internal six-workflow split (below) matters
more than its external dependency shape.

## What belongs here / What must never be here

**Belongs** (§6.5 family role, verbatim): decisions, leases, reservations, mediation, lifecycle
authority, dispatch composition. Concretely: a `Decision` (`Allow`/`Deny`/`RequireApproval`), a
`CapabilityLease`, a `ResourceReservation`, an `Obligation` handler, a `ProcessRecord`/
`TurnRunState` transition, the `Authorized` witness, the `RuntimeDispatcher` routing fold.

**Must never be here** (§6.5, verbatim): product UX, loop strategy, vendor anything, lane
mechanics, storage backends. Concretely, and pinned by crate-local guidance across the family:

- no rendering/prompting/UI (`approvals.md:14` — "does not prompt users, render UI")
- no vendor names or protocol branches (the specificity scanner's allowlist excludes every
  kernel crate)
- no direct `std::process`/Docker calls outside the one sanctioned lane-executor seam
- no raw secrets/host paths/backend error strings in any error, event, snapshot, or log — every
  crate's `AGENTS.md` repeats this line verbatim
- no capability/runtime execution logic masquerading as "best-effort" where the contract
  requires fail-closed (`ironclaw_resources/AGENTS.md`)

## Dependency direction

Kernel crates may depend on:

- **contracts** — `host_api`, and, once the carve-outs land, `loop_contracts` for turn/loop
  vocabulary such as `LoopSafeSummary`
- **substrate** — `filesystem`, `secrets` (via obligations), `network` (via obligations),
  `safety`
- **events** — `events`, and `event_store` transitively through `host_runtime`
- **domains** — `extensions`, once it re-layers to `substrates` (this is exactly what legalizes
  today's `capabilities→extensions` and `host_runtime→extensions` W7 exceptions)
- **lanes** — `host_runtime`'s closed executor constructs `wasm`/`mcp`/`sandbox` adapters
- **kernel siblings** — per the map in §6.5, e.g. `capabilities` depends on `authorization`,
  `resources`, `runtime_policy`, `run_state`, `trust`, `turns`, `processes`

Kernel crates may **never** depend on `loops`, `products`, or `app` (§8.2's forbidden-edge
matrix, `kernel` row) — with two documented, deliberate inversions where a kernel crate defines
a port that a higher layer implements:

- `CapabilityDispatcher` (`host_api`, contracts tier → `capabilities::RuntimeDispatcher`)
- `ProcessExecutor` / supervisor-executor registration (`processes` → `runner`/`host_runtime`,
  §8.1 rule 3)

Everything in `loop/`, `extensions/`, `product/`, and `app/` depends on this family, directly or
through a contracts-tier port; nothing in this family depends back.

## Crate specifications

### `ironclaw_trust`

- **Path & disposition:** `crates/ironclaw_trust` → `crates/kernel/ironclaw_trust`; retain,
  directory move only (PROPOSAL §9 row 31, §6.5.1).
- **Purpose:** the host-controlled policy engine that turns a manifest's *requested* trust into
  the host-validated *effective* trust ceiling every downstream authorization decision consumes.
- **Pipeline stage:** T5a — trust-ceiling evaluation, the first sub-fold inside
  `capabilities::authorize()`.
- **Target contents:** unchanged module map:
  - `decision.rs` — `EffectiveTrustClass`, `TrustDecision`, `AuthorityCeiling`,
    `HostTrustAssignment`, `TrustProvenance`
  - `policy.rs` — `HostTrustPolicy`, `TrustPolicy`, `TrustPolicyInput`
  - `sources.rs` — `PolicySource`, `AdminConfig`/`AdminEntry`, `BundledRegistry`/`BundledEntry`
  - `invalidation.rs` — `InvalidationBus`, `TrustChange`, `TrustChangeListener`
  - `clock.rs` — `Clock`; `error.rs` — `TrustError`; `fixtures.rs` — test-only
- **Migration delta:** none structural — a pure `git mv`; `[package.metadata.ironclaw]
  layer = "kernel"` is already correct. The one open item is a product decision, not a code
  move (see Open questions).
- **Owns:**
  - the sealed `EffectiveTrustClass::FirstParty`/`::System` constructors (crate-private —
    `crates/ironclaw_trust/src/decision.rs`)
  - `HostTrustPolicy::mutate_with`'s hard-wired pre-evaluate/stage/commit/post-evaluate/publish
    sequence, which makes synchronous grant invalidation on trust downgrade a compile-time
    guarantee rather than a review convention
- **Must never contain:** capability registration, grant issuance, dispatch, secret custody, or
  runtime knowledge (`crates/ironclaw_trust/CONTRACT.md:33-36`).
- **Allowed internal deps:** `ironclaw_host_api` only.
- **Forbidden:** every other internal crate — `trust` is the family's purest leaf, one
  dependency deep.
- **Public contracts & ports:** `EffectiveTrustClass` is **sealed**: it implements `Serialize`
  for audit envelopes but deliberately not `DeserializeOwned`, pinned at compile time by a
  `static_assertions::assert_not_impl_any!` lock (`lib.rs` test module; `CONTRACT.md:96-98`).
  `TrustClass` itself (the wire vocabulary in `ironclaw_host_api::runtime`) uses
  `#[serde(skip_deserializing)]` on its `FirstParty`/`System` variants — the two-layer seal this
  crate's policy evaluation sits behind.
- **Security & authority role:** the authority-ceiling gate — the only place a privileged
  `EffectiveTrustClass` can be constructed; a user-installed manifest cannot fabricate one by
  deserializing into a wire type (`kernel-boundary.md §5`, minimum policy rule 2).
- **Why a crate (not a module):** the seal depends on Rust's crate-scoped `pub(crate)`
  visibility. If `trust` were a module inside `capabilities`, its `pub(crate)` mutators
  (`upsert`/`remove` on `SourceMutators`) would be visible to every other module in that
  ~7.3k-line crate, not just its own file — the seal would still compile but would no longer
  mean what the contract claims. One crate boundary is what makes "only this code can mutate
  policy state" actually true. 9 consumers plus a distinct `PolicySource` layering also satisfy
  criterion 1.
- **Enforcement:** `cargo test -p ironclaw_trust`; `cargo test -p ironclaw_architecture` for the
  layer/boundary check; the crate's own `static_assertions` pin is a compile-time gate, not a
  runtime test.
- **Open questions (PROPOSAL §12.10):** trust's inert `SignedRegistry`/`DevTrustOverride`/
  `BundledRegistry` sources — commit to the signed-package roadmap they anticipate, or delete
  them. Not resolved by this proposal.

### `ironclaw_authorization`

- **Path & disposition:** `crates/ironclaw_authorization` → `crates/kernel/ironclaw_authorization`;
  retain, directory move only (§9 row 32, §6.5.2).
- **Purpose:** default-deny grant matching plus the `CapabilityLease` state machine — Stage 3's
  decision engine, invoked inside `CapabilityHost::authorize`.
- **Pipeline stage:** T5b — the authorization decision.
- **Target contents:** unchanged single-file crate:
  - `lib.rs` — `CapabilityDispatchAuthorizer`/`TrustAwareCapabilityDispatchAuthorizer` traits;
    `GrantAuthorizer`; `LeaseBackedAuthorizer`; `CapabilityLease`/
    `CapabilityLeaseStatus{Active,Claimed,Dispatching,Consumed,Revoked}`/`CapabilityLeaseError`;
    `CapabilityLeaseStorePort` plus the one production `CapabilityLeaseStore<F>` (bounded
    compare-and-swap over versioned roots with a 3-attempt retry budget)
  - `test_support.rs` — in-memory-backed constructor over `InMemoryBackend`, feature-gated
- **Migration delta:** none structural. Scope note: this crate owns the port plus its two
  generic implementations (`GrantAuthorizer`, `LeaseBackedAuthorizer`); the third production
  `TrustAwareCapabilityDispatchAuthorizer` impl (composition's `ProfileApprovalPolicyAuthorizer`,
  carrying policy *content*) stays in `app/ironclaw_composition` — this crate owns the
  mechanism, not every policy that plugs into it.
- **Owns:**
  - grant-vs-context matching (`grant_exceeds_authority_ceiling`)
  - the lease lifecycle, including `begin_dispatch_claimed`/`abort_dispatch_claimed`, the
    single-winner CAS that lets `auth_resume_json` safely re-bounce a claimed lease
- **Must never contain:** approval lease *claiming* decisions (that sequencing lives in
  `capabilities`), runtime dispatch, obligation execution, or stringly permission logic
  (`crates/ironclaw_authorization/AGENTS.md:19-21`).
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_trust`.
- **Forbidden:** `run_state`, `approvals`, `capabilities`, `processes`, `resources`, and
  everything above kernel — `approvals` depends on `authorization`, never the reverse.
- **Public contracts & ports:** `CapabilityLeaseStorePort` is the **fingerprinted approval
  lease** authority — leases carry an `InvocationFingerprint` computed by `capabilities` and
  validated on resume so an approval for one input can never be replayed with a different one.
  `TrustAwareCapabilityDispatchAuthorizer` is kept distinct from plain
  `CapabilityDispatchAuthorizer` specifically because `EffectiveTrustClass` deliberately does
  not implement `Deserialize` (`src/lib.rs:69-76`) — trust-aware authorization cannot be a
  wire-shaped call.
- **Security & authority role:** the Stage-3 default-deny gate; sole owner of the lease state
  machine every fingerprinted approval resume rides on.
- **Why a crate:** three production `TrustAwareCapabilityDispatchAuthorizer` implementations
  (criterion 4, multi-impl) split across two crates by design — two generic ones here, one
  policy-content one in composition — plus a lease authority correlated to `approvals` only by
  an opaque fingerprint value, never by a direct dependency. The one-directional edge is the
  whole point of the split.
- **Enforcement:** `cargo test -p ironclaw_authorization`; `cargo test -p ironclaw_architecture`;
  the `test-support` feature keeps the in-memory lease-store constructor out of production
  binaries by default.
- **Open questions:** none from PROPOSAL §12.10.

### `ironclaw_approvals`

- **Path & disposition:** `crates/ironclaw_approvals` → `crates/kernel/ironclaw_approvals`;
  retain-**widen**, move (§9 row 33, §6.5.3). The widening is **[#6696]**-contingent; the move
  itself is not.
- **Purpose:** resolves durable pending approval requests into scoped, fingerprinted
  `CapabilityLease`s (or denials) — Stage 3.5, human/policy consent kept out of band from grant
  matching.
- **Pipeline stage:** T5c — approval resolution, between authorization's `RequireApproval`
  verdict and dispatch's resume.
- **Target contents:** unchanged module map:
  - `lib.rs` — `ApprovalResolver`, `LeaseApproval`/`DenyApproval`, `ApprovalResolutionError`
  - `policy.rs` — `PersistentApprovalPolicy`/`PersistentApprovalPolicyStore`, the durable
    "always allow" reuse policy, scoped tenant/user/agent/project and gated on manifest
    `default_permission = allow`
  - `auto_approve.rs` — `AutoApproveSettingStore`
  - `capability_permission.rs` — `CapabilityPermissionOverrideStore`
  - `cas_record.rs`, `test_support.rs`
- **Migration delta:**
  - **CURRENT** — resolves against `ironclaw_run_state`-owned `ApprovalRecord` via a normal
    cross-crate dependency.
  - **[#6696] TARGET** — widens to own the approval-request record *and* gate-record storage
    itself, absorbing `run_state`'s `ApprovalRequestStore` and `GateRecordStore` (§6.5.3:
    "absorbs the approval-request + gate records from `run_state` when the journal collapse
    lands"; `#6696` slice 2 names exactly this move). The `run_state` dependency inverts to
    nothing once that lands.
  - Independent of `#6696`: deletes `ToolPermissionOverrideStorePort` — a marker trait with a
    blanket impl and zero named implementors (PROPOSAL §2.6).
- **Owns:**
  - `ApprovalResolver`'s fail-closed ordering — the approve-record write is durable *before*
    the lease write; a crash between the two is recoverable via
    `retry_lease_issue_for_dispatch`/`_spawn`, never by re-approving
  - `PersistentApprovalPolicy`, `AutoApproveSettingStore`, `CapabilityPermissionOverrideStore`
  - **[#6696]** future: the `ApprovalRequestStore`/`GateRecordStore` records themselves
- **Must never contain:** reusable scoped approvals promoted to ambient grants (a fingerprinted
  lease is excluded from `active_grants_for_context` by design — `approvals.md:256`), dispatch
  before the matching-fingerprint lease is validated and claimed, or notification
  delivery-target selection (`approvals.md:14-16`: "does not... choose a delivery target").
- **Allowed internal deps:** `ironclaw_authorization`, `ironclaw_events` (`AuditSink`),
  `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_run_state` (current; dissolves under
  `#6696`).
- **Forbidden:** `capabilities`, `host_runtime`, `processes`, `turns`, and everything above
  kernel.
- **Public contracts & ports:** `LeaseApproval`/`DenyApproval` decision DTOs;
  `PersistentApprovalPolicyStorePort`; the approve-before-lease fail-closed sequencing is itself
  a load-bearing, retry-recoverable contract, not merely an implementation detail.
- **Security & authority role:** the Stage-3.5 human/policy consent authority — the only place a
  `Pending` record becomes either a fingerprinted resume-scoped lease or a terminal denial.
- **Why a crate:** a distinct consent authority from grant matching — `authorization` matches
  pre-existing grants, `approvals` resolves one-off human/policy decisions — with 6 consumers
  today. The **[#6696]** widening sharpens rather than dilutes this: it becomes the sole owner
  of "what is this blocked invocation waiting on," collapsing a responsibility currently split
  with `run_state`.
- **Enforcement:** `cargo test -p ironclaw_approvals`; `cargo test -p ironclaw_architecture`; a
  fail-closed-ordering regression test (approve-record precedes lease-write) is part of the
  crate's own suite.
- **Open questions:** none from PROPOSAL §12.10 (the `BudgetApprovalGate`-vs-capability-approval
  separation is chartered, not listed as unresolved, and belongs to `resources` in any case).

### `ironclaw_resources`

- **Path & disposition:** `crates/ironclaw_resources` → `crates/kernel/ironclaw_resources`;
  retain, directory move only (§9 row 34, §6.5.4).
- **Purpose:** the host-level governor for cost, quota, and scarce runtime capacity — the
  reserve → execute → reconcile-or-release protocol every costed lane must use before it spends
  money or capacity.
- **Pipeline stage:** T5d — reservation at dispatch time, plus post-dispatch reconciliation as
  part of T7.
- **Target contents:**
  - `lib.rs` — `ResourceGovernor` trait plus `InMemoryResourceGovernor` and
    `PersistentResourceGovernor<S>`; `ResourceDimension{Usd,InputTokens,OutputTokens,
    WallClockMs,OutputBytes,NetworkEgressBytes,ProcessCount,ConcurrencySlots}`;
    `ResourceAccount`/`ResourceLimits`/`ResourceTally`/`ResourceDenial`/
    `ResourceGovernorSnapshot`; `JsonFileResourceGovernorStore`
  - `filesystem_governor.rs` + `filesystem_governor/{authority.rs,journal.rs}` —
    `FilesystemResourceGovernor<F>`, the third `ResourceGovernor` impl, `ScopedFilesystem`-backed
  - `cas_snapshot.rs` — the CAS snapshot pattern shared with `resource_store.rs`
  - `event.rs` — `BudgetEventSink` family
  - `gate.rs` — `BudgetApprovalGate`, a **separate** pause-threshold gate machine with three
    terminal states: `Approved`-with-new-limit, `Cancelled`, `Expired`
  - `period.rs` — `BudgetPeriod`/`BudgetThresholds`/`period_bounds`
  - `resource_store.rs` — `ResourceGovernorStore`/`BudgetGateStore`; `test_support.rs`
- **Migration delta:** gains the four budget constants currently squatting in
  `ironclaw_common::lib.rs` (§6.5.4: "absorbs the budget constants from common"). No other
  structural change.
- **Owns:**
  - the reserve/execute/reconcile-or-release protocol and its fail-closed-on-storage-error
    invariant
  - three production `ResourceGovernor` implementations — `FilesystemResourceGovernor`,
    `PersistentResourceGovernor`, `InMemoryResourceGovernor` (verified via
    `impl ResourceGovernor for` across `filesystem_governor.rs`/`lib.rs`)
  - `BudgetApprovalGate`, explicitly chartered as *not* the same machine as capability approval
    (`gate.rs:1-16`) — no shared vocabulary with `run_state`/`approvals` by design; unification
    is an ADR, not assumed here
- **Must never contain:** process/runtime execution logic, or best-effort accounting anywhere
  the contract requires fail-closed behavior (`crates/ironclaw_resources/AGENTS.md:22`).
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Forbidden:** everything else internal — `resources` is called by `capabilities`/
  `processes`/`host_runtime`, it never reaches into them.
- **Public contracts & ports:** `ResourceGovernor` (3 production impls — the kernel family's
  only genuinely multi-implementation core trait); `BudgetGateStorePort`;
  `ResourceReservation`/`ResourceReceipt` (owned upstream in `host_api`, re-exported here for
  ergonomics).
- **Security & authority role:** the Stage-5d cost/capacity gate. Core invariant
  (`resources.md §2`): "No costed or quota-limited work should execute in hosted/multi-tenant
  mode without an active reservation," and a storage failure denies exactly like a quota denial
  — never a silent allow.
- **Why a crate:** the only multi-production-impl core trait in the entire kernel family
  (criterion 4), 9 consumers, and a `[target.'cfg(windows)'.dependencies]` platform-specific
  file-lock dependency (`windows-sys`) that criterion-5 platform isolation keeps out of every
  other kernel crate's build.
- **Enforcement:** `cargo test -p ironclaw_resources`; `cargo test -p ironclaw_architecture`;
  PostgreSQL/libSQL parity exercised through the filesystem governor's `ScopedFilesystem`
  backend selection.
- **Open questions:** none from PROPOSAL §12.10.

### `ironclaw_runtime_policy`

- **Path & disposition:** `crates/ironclaw_runtime_policy` → `crates/kernel/ironclaw_runtime_policy`;
  retain as-is, directory move only (§9 row 35, §6.5.5).
- **Purpose:** pure `(DeploymentMode, RuntimeProfile, OrgPolicy) → EffectiveRuntimePolicy`
  resolution plus per-capability lane planning — zero I/O, the cleanest crate in the audit.
- **Pipeline stage:** T5e, folded — runs *inside* `capabilities::authorize()` rather than as a
  separate hop, per its own module doc: "Relocated here from `ironclaw_host_runtime`... so the
  capability kernel can run runtime-policy enforcement inside `authorize()` without an upward
  dependency on the host runtime" (`src/lib.rs:55-60`).
- **Target contents:**
  - `lib.rs` — re-exports
  - `resolver.rs` — `resolve`, `ResolveRequest`, `BudgetEnforcement`, `MinimalApprovalBypass`,
    `OrgPolicyConstraints`
  - `planner.rs` — `plan_capability`, `ExecutionPlan`, `PlannerError`, already relocated in from
    `host_runtime` (arch-simplification §5.3.1)
- **Migration delta:** none — pure directory move; the crate is already fully narrowed. Note:
  `crates/ironclaw_host_runtime/AGENTS.md` still incorrectly claims `plan_capability` ownership;
  that guidance-drift fix travels with `host_runtime`'s entry below, not a code change here.
- **Owns:**
  - monotonic-safety enforcement — deployment/org policy may only *reduce* requested authority,
    never increase it
  - fail-closed rejection of invalid `(deployment, profile)` pairs
  - the Yolo opt-in disclosure gate; the hosted-multi-tenant boundary (`Local*` profiles never
    resolve, and `HostWorkspace`/`LocalHost` backends never get selected, under
    `HostedMultiTenant`)
- **Must never contain:** runtime process startup, action dispatch, or product strategy outside
  profile selection (`crates/ironclaw_runtime_policy/AGENTS.md:19`).
- **Allowed internal deps:** `ironclaw_host_api` only.
- **Forbidden:** everything else — 2 total dependencies (`host_api` + `serde`), 0 traits.
- **Public contracts & ports:** `EffectiveRuntimePolicy` — the sole sanctioned producer is
  `resolve()`; values constructed elsewhere are untrusted by contract. `ExecutionPlan` —
  `plan_capability`'s output selects the `RuntimeLane` sealed into the `Authorized` witness at
  T6.
- **Security & authority role:** the policy-math gate feeding trust/authorization; deterministic
  and round-trippably serializable so an audit log can record the exact policy that gated an
  invocation (`docs/reborn/contracts/runtime-profiles.md §4.4`).
- **Why a crate:** zero-I/O pure policy math consumed identically by both `capabilities`
  (in-fold, at decision time) and `host_runtime` (at lane-selection time) — a shared leaf
  dependency, kept a crate so neither of those two much-larger crates has to depend on the
  other's tree just to reuse this function.
- **Enforcement:** `cargo test -p ironclaw_runtime_policy`; `cargo test -p ironclaw_architecture`;
  determinism is exercised as a property (equal inputs, equal outputs).
- **Open questions:** none from PROPOSAL §12.10.

### `ironclaw_capabilities`

- **Path & disposition:** `crates/ironclaw_capabilities` → `crates/kernel/ironclaw_capabilities`;
  retain, directory move (§9 row 36, §6.5.6); **internal** `host.rs` split along its six
  workflows as a module charter, non-breaking (WS3 item 5).
- **Purpose:** the caller-facing capability invocation service — `CapabilityHost`, the membrane
  every privileged effect crosses.
- **Pipeline stage:** T5 core — folds T5a–T5e (trust ceiling → authorization → approval resume →
  obligations composition → the sealed `Authorized` witness) and owns dispatch routing at T6a;
  also the re-authorization seam for detached process continuations at the T4/T6 boundary.
- **Target contents:** unchanged module map:
  - `host.rs` (4,534 lines) — `CapabilityHost` and its six public workflows, verified in
    source: `invoke_json`, `resume_json`, `auth_resume_json`, `decline_auth_json`,
    `resume_spawn_json`, `spawn_json`
  - `dispatch.rs` — `RuntimeDispatcher`, the sole `CapabilityDispatcher` implementation;
    `BoundCapabilityAdapter`/`ToolResolver`/`ChainToolResolver`
  - `obligations.rs` — `post_dispatch_obligations` fold, obligation request/outcome types
  - `ports.rs` — `HostPolicyFacts`/`PolicyAction`/`CredentialPresence`, the policy-facts port
    `authorize()` reads for credential preflight
  - `process_authorization.rs` — `ProcessAuthorizationRemintPort`, kernel-owned re-minting for
    detached process continuations; re-seals only the durable spawn decision already persisted,
    after reloading and validating the record
  - `registry.rs` — `CapabilityDispatchRegistry`
  - `replay_payload.rs` — `ReplayPayload`/`ReplayPayloadStore`, host-private raw tool-input
    persistence for gate/auth resume
  - `requests.rs`, `trust.rs` (`evaluate_invocation_trust`), `error.rs`, `helpers.rs`
- **Migration delta:** internal-only — `host.rs`'s six workflows become chartered modules (no
  public-API or crate-boundary change). No dependency-edge change to the crate itself; the
  `capabilities→extensions` W7 exception dissolves for free once `ironclaw_extensions`
  re-layers to `substrates` (same code, newly legal under the layer matrix).
- **Owns:**
  - the `authorize()` in-fold — trust classification, runtime-policy planning, credential
    preflight, grant/lease authorization, and `InvocationFingerprint` computation, all before
    any obligation prepares
  - the **sealed `Authorized` witness** minting; `CapabilityAuthorizer` is implemented
    **nowhere else** in the workspace, enforced by the `reborn_authorized_seal_ratchet` test
    (`host.rs:72-79`, own comment: "only this crate can mint an `AuthorizationGrant`")
  - the host-private `ReplayPayload` store — deliberately *not* in `run_state` (whose charter
    forbids persisting raw replay input, `run_state/CLAUDE.md:7`) or `turns` (same prohibition)
    — because `capabilities` owns the invoke/resume/spawn workflow this payload serves and
    carries no such prohibition (`replay_payload.rs:23-27`)
- **Must never contain:** parallel dispatch paths, process lifecycle/result APIs (that is
  `processes`' job — `capabilities` only calls `ProcessManager`), or dispatch before
  authorization/obligations/approval gates (`crates/ironclaw_capabilities/AGENTS.md:24`).
- **Allowed internal deps:**
  - contracts — `ironclaw_host_api`
  - kernel siblings — `ironclaw_authorization`, `ironclaw_processes`, `ironclaw_resources`,
    `ironclaw_runtime_policy` (in-fold), `ironclaw_run_state` (current; **[#6696]** replaced by
    a process-invocation port), `ironclaw_trust`, `ironclaw_turns` (resume-payload field types
    only: `CapabilityInputRef`, `AuthResumeApprovalIdentity`)
  - substrate — `ironclaw_filesystem`, `ironclaw_safety`; events — `ironclaw_events`; domains —
    `ironclaw_extensions` (legal once it re-layers to `substrates`)
- **Forbidden:** `ironclaw_host_runtime` (capabilities must never depend upward on its own
  composer — direction is strictly `host_runtime → capabilities`), any `loops`/`products`/`app`
  crate, any lane crate directly (dispatch happens only through the `CapabilityDispatcher`
  port).
- **Public contracts & ports:** `Authorized` — the **sealed witness** (re-exported from
  `host_api`, minted only here); `CapabilityDispatcher` (the port, implemented in this crate by
  `RuntimeDispatcher`); `CapabilityObligationHandler` (the seam `host_runtime`'s
  `BuiltinObligationHandler` implements); `HostPolicyFacts` (the seam `host_runtime`'s
  production wiring implements to supply credential-presence facts); `ProcessAuthorizationRemintPort`.
- **Security & authority role:** **the** membrane. `kernel-boundary.md §8`'s first acceptance
  test — "loops cannot bypass `CapabilityHost` for privileged effects" — is written against
  this crate specifically.
- **Why a crate (not a module):** the seal here is ratchet-tested, not compiler-sealed (unlike
  `trust`'s serde seal) — but one crate boundary gives the ratchet exactly one thing to scan,
  and keeps the six-workflow fold (obligations/approvals/dispatch composing atomically inside
  one `CapabilityHost`) as one reviewable unit rather than scattering the witness's construction
  across a crate boundary an attacker-model review would probe first.
- **Enforcement:** `cargo test -p ironclaw_capabilities`; `cargo test -p ironclaw_architecture`
  (the authorized-seal ratchet plus the `capabilities→extensions` exception-removal check); the
  six workflows are tested caller-level, driving `CapabilityHost` methods directly rather than
  only their helpers.
- **Open questions:** none from PROPOSAL §12.10.

### `ironclaw_processes`

- **Path & disposition:** `crates/ironclaw_processes` → `crates/kernel/ironclaw_processes`;
  retain-**widen**, move + re-layer `runtimes` → `kernel` (§9 row 37, §6.5.7). The base
  move/re-layer is unconditional; the widening beyond today's shape is **[#6696] DIRECTION**.
- **Purpose:** host-tracked background capability process lifecycle. Intentionally below
  `CapabilityHost` — it does not decide *whether* a caller may spawn, only tracks what happens
  once `spawn_json` creates the record (`docs/reborn/contracts/processes.md §1`).
- **Pipeline stage:** T4 — claimed execution: lease claim, heartbeat, cooperative cancellation,
  terminal-state protection for backgrounded work.
- **Target contents:** unchanged module map:
  - `types.rs` — `ProcessRecord`, `ProcessStatus{Running,Completed,Failed,Killed}`,
    `ProcessStart`, `ProcessExit`, `ProcessStorePort`/`ProcessResultStorePort`/
    `ProcessExecutor`/`ProcessManager` traits
  - `process_store.rs` — `ProcessStore<F>`/`ProcessResultStore<F>`, the one production impl
    pair, exercised over `InMemoryBackend` in tests
  - `wrappers.rs` — `EventingProcessStore`, `ResourceManagedProcessStore` (rejects
    caller-supplied reservation IDs)
  - `cancellation.rs` — `ProcessCancellationRegistry`/`Token`
  - `host.rs` — `ProcessHost`/`ProcessSubscription`, the read/poll/await/cancel surface
  - `services.rs` — `ProcessServices`, `BackgroundProcessManager`,
    `BackgroundErrorHandler`/`Failure`/`Stage`; `test_support.rs`
- **Migration delta:**
  - **CURRENT** — a layer reassignment only (`runtimes`→`kernel`), which legalizes today's
    `processes→resources` W7 exception (`resources` is `substrates`; `kernel` may depend on
    `substrates` under the monotone ladder). No structural code change for the base move.
  - **[#6696] DIRECTION** — widens into the general durable lifecycle authority: a row-native
    journal, `ProcessSupervisor` (claim/lease/heartbeat/recovery/panic-containment/shutdown,
    generalizing `runner`'s 1,129-line turn scheduler), process kinds as registered executors
    (`ProcessKind::AgentTurn` from `runner`, `ProcessKind::CapabilityInvocation` from
    `host_runtime` — **neither exists in code today**, confirmed by a repo-wide search), child
    dependencies as process edges (replacing `runner`'s ~4.6k-line await-edge machinery:
    `await_edge`/`store` 1,457 + `boot_recovery` 720 + `resolver` 1,879 + `roster` 561 lines),
    unified checkpoint payload rows (replacing `turns`' `CheckpointStateStorePort` and its
    `/checkpoint-state` mount), and immutable process input (replacing a 527-line subagent goal
    store).
- **Owns:** `ProcessRecord` identity and lifecycle — current, tenant/user/agent partitioned;
  cross-scope reads return `None`/`UnknownProcess`, never leaking existence. **[#6696]** the
  one-lifecycle-authority journal and executor registration.
- **Must never contain:** capability authorization, approval policy, or runtime lane internals
  outside adapter-facing contracts (`crates/ironclaw_processes/AGENTS.md:23`) — it never decides
  *if* something may spawn, only tracks that it did.
- **Allowed internal deps:** `ironclaw_events`, `ironclaw_filesystem`, `ironclaw_host_api`,
  `ironclaw_resources` (legal once the layer flips).
- **Forbidden:** `capabilities`, `host_runtime`, `turns`, `authorization`, `approvals`, `trust`
  — nothing above or beside it in the kernel reaches back down except through its own ports.
- **Public contracts & ports:** `ProcessStorePort`/`ProcessResultStorePort`; `ProcessExecutor` —
  the **dependency-inversion port** `runner` and `host_runtime` implement (`processes →
  runner/host_runtime`, §8.1 rule 3), the mechanism that lets a kernel crate trigger loop-tier
  execution without depending on the loop tier. **[#6696]** adds `ProcessDependencyPort`
  (child-edge port, replacing `runner`'s await-edge store) and a process-invocation state port
  `capabilities` consumes in place of `run_state`.
- **Security & authority role:** the T4 claimed-execution authority; terminal states
  (`Completed`/`Failed`/`Killed`) are write-once — a late executor completion after `kill` is
  silently ignored, never resurrected (`processes.md §4`).
- **Why a crate:** already the kernel's dedicated process-tracking authority (1 public trait
  beyond its store ports, direct production wiring from `capabilities`/`host_runtime`/
  `composition`); the **[#6696]** widening sharpens rather than weakens the case — one crate
  owning "what is this piece of work doing right now" for both turns and background
  capabilities is the entire point of the collapse.
- **Enforcement:** `cargo test -p ironclaw_processes`; `cargo test -p ironclaw_architecture`
  (the `processes→resources` exception-removal check); tenant/user/agent partition tests
  asserting cross-scope `UnknownProcess`.
- **Open questions:** none from PROPOSAL §12.10 (the `#6696` contingency is the family-wide tag,
  not a §12.10 unresolved-decision item).

### `ironclaw_turns`

- **Path & disposition:** `crates/ironclaw_turns` → `crates/kernel/ironclaw_turns`;
  retain-**narrow** (split out contracts), move (§9 row 38, §6.5.8).
- **Purpose:** the turn admission kernel — `TurnCoordinator` (accept/resume/cancel),
  one-active-run-per-thread, idempotency, and exit validation: "`LoopExit` is a claim, not
  truth" (§6.5.8).
- **Pipeline stage:** T3 — durable admission (binding/idempotency resolved upstream in
  `product`, then `TurnCoordinator` persists under the lock); also the T5→T7 boundary's
  exit-claim validation — `LoopExitApplier` validates host-minted evidence refs before any
  durable transition.
- **Target contents (current, 33.7k lines total):**
  - `admission.rs` — `TurnAdmissionPolicy`, limits/buckets/capacity denial
  - `coordinator.rs` — `DefaultTurnCoordinator`/`TurnCoordinator`, bounded
    `MAX_PREPARED_RUN_IDS`, the one-active-run enforcement
  - `store.rs` + `turn_state_row_store/**` (~10.2k lines) — the durable CAS row store:
    `row_store/{commit,delta,events_index,io,journal,load,traits,write_behind}.rs`,
    `turn_state_engine/{admission,concurrency_limiter,idempotency,limits,run_record,snapshot,
    spawn_tree,transitions}.rs`, `profile_resolver.rs`, `projection.rs`, `runner_lease.rs`
  - `checkpoint_state.rs` — `CheckpointStateStorePort`
  - `loop_exit.rs` — `LoopExit`/`LoopExitApplier`/evidence ports
  - `events.rs` — `TurnLifecycleEvent`/`TurnEventSink`/projection service
  - `lifecycle.rs`, `origin.rs`, `request.rs`/`response.rs`, `block_persistence.rs`
  - `status.rs` — `TurnStatus`, which currently duplicates `run_state`'s
    `RunStatus::{BlockedApproval,BlockedAuth}` verbatim (verified at `src/status.rs:17-18,198-212`)
  - `ids.rs` — 8-line re-export shim of `host_api`; `scope.rs` — 1-line re-export shim
  - `run_profile/**` (~14.3k lines) — 11 `Loop*Port` traits plus driver/resolver/policy/
    prompt/model/context submodules, its own `CLAUDE.md`
  - `external_tool_catalog.rs` — per-run OpenAI-Responses external-tool catalog
  - `product_adapter/{mod.rs,fakes.rs}` — compatibility re-export of
    `ironclaw_host_api::product_adapter`
  - `product_context.rs`; `runner.rs` — trusted runner-only transition APIs, deliberately
    excluded from the adapter prelude; `test_support.rs`
- **Migration delta:**
  - **Sheds** — `ids.rs`/`scope.rs` re-export shims deleted (turn-ID vocabulary completes in
    `host_api::turn`, its already-canonical home); `run_profile/**` (~14.3k lines, all 11
    `Loop*Port` traits) moves whole to the new `contracts/ironclaw_loop_contracts`; the
    `CheckpointStateStorePort`/`LoopExit` **DTO half** moves with it (the `LoopExitApplier`'s
    *validation logic*, which consumes those DTOs, stays here); `external_tool_catalog.rs`
    moves to `product`, its self-described owner; `product_adapter/` compatibility re-export is
    deleted outright.
  - **[#6696] DIRECTION** — the store/scheduler halves become projections/adapters over
    `processes` (an internal consolidation, not itself a crate move).
  - **Net effect:** consumers drop from today's 18-crate fan-in (`turns` is currently the
    biggest vocabulary hub in the workspace) to ~4 (`product`, `composition`, `runner`,
    `loop_host`) once the six vocabulary-only consumers (`auth`, `event_streams`, `outbound`,
    `telegram_extension`, `triggers`, `event_projections`) repoint to `host_api::turn` directly.
- **Owns:** one-active-run-per-thread plus idempotency (the coordinator's core invariant); the
  durable `TurnStateRowStore`; `LoopExitApplier`'s claim-vs-truth validation boundary.
- **Must never contain:** raw `CapabilityHost`/dispatcher/runtime handles, raw prompts/content/
  tool inputs/secrets/host paths, or channel-identity parsing (`AGENTS.md:29`).
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_observability`.
- **Forbidden:** everything else internal — deliberately thin given an 18-crate current fan-in;
  one more edge here is a fan-out risk the architecture test exists to catch.
- **Public contracts & ports:** `TurnCoordinator` (the adapter-safe API; the `runner` module's
  trusted-worker transition APIs are deliberately excluded from the crate's public prelude);
  `CheckpointStateStorePort` (moving to `loop_contracts`); `LoopExitEvidencePort`.
- **Security & authority role:** the T3 durable-admission authority and the T5–T7 boundary's
  "claims aren't truth" enforcement point. `kernel-boundary.md §6` names "one active run per
  thread" and "approval/auth/resource waits recorded as structured blocked states" as
  kernel-mediated turn invariants — this crate is their concrete home.
- **Why a crate (not a module):** admission-and-exit-validation is a fail-closed authority
  distinct from capability dispatch — T3/T7 versus T5/T6, "is this turn allowed to keep
  running" versus "is this one capability call allowed" — with a vastly larger fan-in (18
  today) than `capabilities`. Folding the two would mix two different questions answered at two
  different pipeline stages into one crate's blast radius.
- **Enforcement:** `cargo test -p ironclaw_turns`; `cargo test -p ironclaw_architecture` (the 7
  W4.3 `*→turns` exceptions' removal check); PostgreSQL/libSQL durable-replay parity tests (the
  crate's dev-deps wire live `libsql`/`tokio-postgres` specifically for this).
- **Open questions:** none from PROPOSAL §12.10 (the `TurnStatus`/`run_state` overlap is covered
  by the `[#6696]` tag on `run_state`'s and `approvals`' entries, not a separate §12.10 item).

### `ironclaw_host_runtime`

- **Path & disposition:** `crates/ironclaw_host_runtime` → `crates/kernel/ironclaw_host_runtime`;
  retain-**narrow** (multi-way shed), move (§9 row 39, §6.5.9).
- **Purpose:** the kernel service graph — the concrete crate `kernel-boundary.md:17` names as
  "current concrete composition crate for kernel-facing services/adapters." Composes
  `CapabilityHost` with the neutral kernel services and the closed lane executor.
- **Pipeline stage:** T5d (obligation composition) through T7 (safe evidence), plus T6b, the
  closed-lane mediated-execution hop.
- **Target contents (current, 46.5k lines — the largest kernel crate):**
  - `production.rs` (2,893 lines) — `DefaultHostRuntime`, sole `HostRuntime` + `HostPolicyFacts`
    production impl; the sole `CapabilityHost` construction site (`production.rs:979`)
  - `services.rs` + `services/{builder,extension_tool_binder,process_executor,
    production_services,production_wiring,runtime_adapters,tool_resolver,wasm_blocking,
    wasm_diagnostics,wasm_execution}.rs` — assembly plus the closed `RuntimeLaneExecutor`'s
    crate-private adapters
  - `obligations.rs` (2,966 lines) — `BuiltinObligationHandler`/`BuiltinObligationServices` +
    `RuntimeSecretInjectionStore` + `NetworkObligationPolicyStore` +
    `ProcessObligationLifecycleStore`, currently fused in one file
  - `egress/{credential,host_port,mod,pipeline,sanitize}.rs` — `HostHttpEgressService`, sole
    `RuntimeHttpEgress` impl
  - `capability_catalog.rs` (`HotCapabilityCatalog`), `surface.rs`
    (`CapabilitySurfacePolicy`/`VisibleCapability`), `extension_contracts.rs`
    (`default_host_api_contract_registry`/`default_host_port_catalog`), `invocation_services.rs`,
    `process_port.rs`/`process_output.rs`/`process_aliases.rs`
  - `memory_binding.rs`/`memory_context.rs`/`memory_provider.rs`/`memory_native_extension.rs` —
    `MemoryServiceResolver`, provider-agnostic resolution; **stays**
  - `first_party.rs` — `FirstPartyCapabilityRegistry`, the registrar pattern; **stays**
  - `first_party_tools/**` (~7.3k lines) — `echo`/`http`/`http_output`/`json`/`memory`/
    `model_visible_output`/`outbound_delivery`/`schemas`/`shell`/`shell_core`/`skill_management`/
    `skill_url_install{+bundle,github,zip_bundle}`/`spawn_subagent`/`time`/`trace_commons`/
    `trigger_management` — the actual tool *handlers*; **sheds**
  - `sandbox_process/**` (~3.6k lines) — `attribution`/`broker`/`ca`/`container_identity`/
    `credential_firewall`/`key_codec`/`mounts`/`registry`/`scope_key`/`user_key`, the
    `bollard`+`rcgen`+`libc` cone; **sheds**
  - `wasm_credentials.rs`, `document_output.rs`, `post_edit_check.rs`, `http_body.rs`,
    `latency.rs`, `user_profile_source.rs`
- **Migration delta:**
  - **Sheds** — `first_party_tools/**` (~7.3k) → `extensions/packages/first_party/
    ironclaw_first_party_extensions`, via the existing `FirstPartyHandlerRegistrar` pattern
    (host_runtime keeps only the registrar *port*, not the handlers); `sandbox_process/**`
    (~3.6k, plus `ironclaw_process_sandbox`'s plan-contract half) → the new
    `lanes/ironclaw_sandbox` (merged with `ironclaw_scripts`' Docker backend) — this removes
    `bollard`/`rcgen`/`libc` from `host_runtime`'s `Cargo.toml` entirely (WS3's own verification
    command: `rg "bollard|rcgen" crates/kernel/ironclaw_host_runtime/Cargo.toml` → nothing);
    `services/builder.rs` + `services/production_wiring.rs` (2.9k lines of pure assembly)
    shrink toward composition-facing factories; `default_host_api_contract_registry`/
    `default_host_port_catalog`'s product-specific manifest contracts move to the owning
    composition/product layer, leaving only the neutral discovery mechanism here.
  - **Internal split, no dependency change** — `obligations.rs` (2,966 lines) splits into its
    three chartered owners: obligation handling (`BuiltinObligationHandler`/`Services`) ∣
    staged secret/network handoffs (`RuntimeSecretInjectionStore`/`NetworkObligationPolicyStore`,
    both TTL'd and consumed via `take()`) ∣ process-obligation store
    (`ProcessObligationLifecycleStore`).
  - **Layer** — the `host_runtime→extensions`/`→first_party_extensions`/`→skills` W7 exceptions
    dissolve through those *other* crates' own re-layering plus the sheds above, not through any
    change to `host_runtime` itself.
  - **Guidance fix (travels with the move):** `AGENTS.md:16` currently claims `plan_capability`
    ownership; that already moved to `runtime_policy::planner` (arch-simplification §5.3.1) —
    corrected in the same PR wave (WS11).
- **Owns:** `DefaultHostRuntime` (the sole `CapabilityHost` construction site); the closed
  `RuntimeLaneExecutor` and its lane adapters; the mediated egress pipeline (policy staging +
  secret staging + sanitize, all one-shot and TTL'd); memory-service *resolution*
  (provider-agnostic — distinct from the memory tool *handlers*, which shed away).
- **Must never contain:** vendor names, product features, or DB drivers beyond what mediation
  itself needs (§6.5.9) — post-shed it should carry no direct `bollard`/`rcgen`/`libc` at all,
  and no product-feature-sized module like the 1,867-line `trace_commons` onboarding flow that
  `first_party_tools` currently smuggles in.
- **Allowed internal deps (target, post-shed):** `ironclaw_approvals`, `ironclaw_authorization`,
  `ironclaw_capabilities`, `ironclaw_events`, `ironclaw_filesystem`, `ironclaw_mcp`,
  `ironclaw_memory`, `ironclaw_memory_native`, `ironclaw_network`, `ironclaw_observability`,
  `ironclaw_outbound`, `ironclaw_processes`, `ironclaw_reborn_event_store`, `ironclaw_resources`,
  `ironclaw_run_state`, `ironclaw_runtime_policy`, `ironclaw_safety`, `ironclaw_secrets`,
  `ironclaw_trust`, `ironclaw_turns`, `ironclaw_wasm`, plus the new `lanes/ironclaw_sandbox`.
- **Forbidden (post-shed):** `ironclaw_first_party_extensions`, `ironclaw_extensions`,
  `ironclaw_skills`, `ironclaw_triggers`, `ironclaw_extractors`, `ironclaw_reborn_traces` as
  normal deps; `ironclaw_process_sandbox`/`ironclaw_scripts` (both deleted, merged into
  `sandbox`).
- **Public contracts & ports:** `HostRuntime` (the stable contract upper turn/loop services
  depend on); `RuntimeHttpEgress` (`HostHttpEgressService`, sole impl); the
  `CapabilityObligationHandler` implementation (`BuiltinObligationHandler`); the
  `HostPolicyFacts` implementation (feeds `capabilities`' credential-preflight fold).
- **Security & authority role:** T5d–T7 mediated execution — turns a sealed `Authorized`
  witness into a lane call with narrowed mounts, one-shot staged secrets, and policy-scoped
  egress, then turns raw lane output into redacted, model-safe, durably-audited evidence.
- **Why a crate (not several):** post-narrowing it is exactly what `kernel-boundary.md` names
  as the concrete kernel-facing composition crate — the real question is why it stays *one*
  crate rather than splitting further: the obligation-completion/lane-execution/sanitize
  sequence is one atomic T6–T7 fold every runtime lane depends on identically, and splitting it
  further would re-introduce the god-crate's internal coupling as inter-crate coupling instead
  of removing it.
- **Enforcement:** `cargo test -p ironclaw_host_runtime`; `cargo test -p ironclaw_architecture`
  (every `host_runtime→*` W7 exception plus the `bollard`/`rcgen` absence check); PostgreSQL/
  libSQL parity for persistence-touching obligation stores.
- **Open questions:** none from PROPOSAL §12.10 (its sheds are §9/WS3 execution detail, not a
  listed unresolved decision).

### `ironclaw_run_state`

- **Path & disposition:** `crates/ironclaw_run_state` → `crates/kernel/ironclaw_run_state`;
  retain **transitional**, then delete-after-migration (§9 row 40, §6.5.10). **[#6696]**-
  contingent throughout.
- **Purpose:** the current-state tracker for host-managed invocations — "what is this invocation
  waiting on now?" — distinct from `events`' append-only history (`run-state.md §1`).
- **Pipeline stage:** today, T5b′ — records `BlockedApproval`/`BlockedAuth` current-state during
  `CapabilityHost`'s authorize/resume fold; also the model-visible gate-rendering seam via
  `GateRecordStore`.
- **Target contents (current, 1,098 lines + test_support):**
  - `lib.rs` — `RunStatus{Running,BlockedApproval,BlockedAuth,Completed,Failed}`/`RunRecord`/
    `RunStart`; `ApprovalStatus{Pending,Approved,Denied,Expired,Discarded}`/`ApprovalRecord`
  - three production stores, all backend-generic over `ScopedFilesystem<F>` — `RunStateStore<F>`,
    `ApprovalRequestStore<F>`, and `GateRecordStore<F>` (verified at `src/lib.rs:242-821`) —
    exercised over `InMemoryBackend` in tests via `test_support`
  - **Guidance-drift note:** both this crate's own `AGENTS.md` and `run-state.md` still
    describe separate hand-rolled `InMemoryRunStateStore`/`InMemoryApprovalRequestStore` types;
    those no longer exist — arch-simplification consolidated them into the generic `Store<F>` +
    `InMemoryBackend` pattern shared with `authorization`/`approvals`/`processes`. `AGENTS.md`
    additionally omits `GateRecordStore` entirely even though `CLAUDE.md` names it. This is
    exactly the drift PROPOSAL WS11 flags for correction, not a target-architecture code change.
- **Migration delta:**
  - **Frozen** until `#6696`-or-equivalent lands — §6.5.10's charter is explicit: "no new
    consumers; approval/gate stores are the live part."
  - **[#6696] then:** `ApprovalRequestStore` + `GateRecordStore` records move to `approvals`
    (its §6.5.3 widening); `RunRecord`/`RunStatus` dissolve as invocation state becomes a
    `processes` projection (its §6.5.7 widening); the crate is **deleted outright** — no
    successor crate keeps its name or its `RunRecord` type.
  - Until the gate opens, every one of its 8 current dependents (`approvals`, `capabilities`,
    `extension_host`, `host_runtime`, `loop_host`, `product`, `composition`, `runner` —
    verified via `Cargo.toml` search) keeps its edge unchanged.
- **Owns (current, frozen):** `RunRecord`/`ApprovalRecord`/`GateRecord` current-state storage.
  The audit-verified three-way overlap this crate is one leg of: `RunRecord.status`/`RunStatus`
  duplicates process status under the same `InvocationId` with one real consumer
  (`capabilities`); `RunStatus::{BlockedApproval,BlockedAuth}` is duplicated verbatim in
  `ironclaw_turns::TurnStatus`; three parallel "what is this blocked on" handles exist across
  `run_state`/`turns`/`processes` today.
- **Must never contain:** runtime execution, product projections, or raw prompts/assistant
  text/secrets/backend details in state (`AGENTS.md:23`) — and, per the freeze charter, no
  *new* capability beyond what the three live stores already do.
- **Allowed internal deps:** `ironclaw_filesystem`, `ironclaw_host_api`.
- **Forbidden:** everything else — unchanged from today; the freeze charter means this list
  does not grow.
- **Public contracts & ports:** `RunStateStorePort`/`ApprovalRequestStorePort`/
  `GateRecordStorePort`/`RunStateApprovalStorePort` — the combined port
  `CapabilityHost::with_run_state_approval_store` prefers, so the pending-approval record and
  the `BlockedApproval` transition commit together. All four remain live and load-bearing right
  up to deletion; there is no "narrow first" step here, only "frozen, then gone."
- **Security & authority role:** today, one of three places recording what an invocation is
  blocked on — exactly the fragmentation `#6696` exists to collapse into a single authority.
- **Why a crate (not a module) — for now:** this is the one entry in the family where the
  honest answer is "temporarily yes, structurally no." Keeping it a separate crate during the
  freeze — rather than inlining it into `approvals` early — preserves a clean single-PR deletion
  boundary: when `#6696` lands, removing the package and repointing 8 `Cargo.toml`s is the
  entire mechanical footprint, with no risk of tangling frozen code into a crate that is still
  actively changing underneath it.
- **Enforcement:** `cargo test -p ironclaw_run_state`; `cargo test -p ironclaw_architecture`;
  the freeze charter itself ("no new consumers") functions as a review-time ratchet even though
  no automated test currently pins it.
- **Open questions:** none from PROPOSAL §12.10 directly (the `#6696` contingency is the
  family-wide tag; the guidance-drift fix is a WS11 item, addressed above under Target
  contents, not an open architectural decision).

## Family AGENTS.md obligations

Per §11.4, `crates/kernel/AGENTS.md` is written when the family directory first exists (Wave 5)
and must carry, verbatim or by direct reference:

- **The pipeline diagram reference** — PROPOSAL §7's mermaid trust-transition flow and the
  T1–T8 walkthrough table, plus this document's stage-ownership table, so an agent can answer
  "which crate owns this effect" without re-deriving it from source.
- **"No stage skipping — first-party is a ceiling not a bypass."** Invariant #12 (PROPOSAL §3):
  a shipped loop or first-party extension never bypasses `CapabilityHost`; a higher trust
  ceiling still requires explicit grants, scoped mounts, leases, resource budget, and obligation
  handling through the same six workflows every other caller uses. `kernel-boundary.md §4` is
  the canonical statement: "A loop is not trusted because the project shipped it... The kernel
  secures the loop environment; it does not rely on the loop to preserve kernel invariants."
- **Each crate's stage assignment**, as a compact recap agents can grep for:
  - `trust` = T5a; `authorization` = T5b; `approvals` = T5c
  - `resources` = T5d (+ T7 reconcile); `runtime_policy` = T5e (in-fold)
  - `capabilities` = T5 membrane (+ T6a dispatch)
  - `host_runtime` = T5d/T6b/T7 (mediated execution + evidence)
  - `turns` = T3 (+ T5–T7 exit-claim boundary); `processes` = T4
  - `run_state` = T5b′ (transitional)
- **The crate-boundary-must-be-earned gate** (invariant #13 applied to this family): a new
  kernel crate must name its own pipeline stage and its own fail-closed rule or multi-impl port
  — otherwise it is a module of an existing stage-owner. This family added zero crates and
  merged zero crates in this proposal; every crate here already earned its boundary before the
  restructure, which is itself evidence the gate works.
- **The dependency-direction rule** from this document's "Dependency direction" section,
  restated as a check: kernel may depend on contracts/substrate/events/domains/lanes and kernel
  siblings; kernel may never depend on loops/products/app, with the two named port inversions as
  the only exceptions.
- **The two `[#6696]`-gated crates' freeze/direction language**, restated so agents don't
  "helpfully" pre-implement: `run_state` is frozen (no new consumers) until the gate opens;
  `processes`' widening and `approvals`' widening are `DIRECTION`, not `CURRENT` — code today
  must not assume `ProcessKind`, a journal, or a `ProcessSupervisor` exist, because they do not
  (verified: zero occurrences of `ProcessKind` in `crates/ironclaw_processes/src/` or
  `crates/ironclaw_runner/src/` at the time of this document).

## Current → target summary

| # | Crate | Current layer | Target layer | Current path | Target path | Disposition | Key delta |
|---|---|---|---|---|---|---|---|
| 31 | `ironclaw_trust` | kernel | kernel | `crates/ironclaw_trust` | `crates/kernel/ironclaw_trust` | retain, move | none (dir move only); inert-sources decision open (§12.10) |
| 32 | `ironclaw_authorization` | kernel | kernel | `crates/ironclaw_authorization` | `crates/kernel/ironclaw_authorization` | retain, move | none |
| 33 | `ironclaw_approvals` | kernel | kernel | `crates/ironclaw_approvals` | `crates/kernel/ironclaw_approvals` | retain-**widen**, move | **[#6696]** absorbs `run_state` approval + gate records; deletes `ToolPermissionOverrideStorePort` now |
| 34 | `ironclaw_resources` | kernel | kernel | `crates/ironclaw_resources` | `crates/kernel/ironclaw_resources` | retain, move | gains 4 budget constants from `common` |
| 35 | `ironclaw_runtime_policy` | kernel | kernel | `crates/ironclaw_runtime_policy` | `crates/kernel/ironclaw_runtime_policy` | retain, move | none |
| 36 | `ironclaw_capabilities` | kernel | kernel | `crates/ironclaw_capabilities` | `crates/kernel/ironclaw_capabilities` | retain, move | internal `host.rs` split along 6 workflows (module charter only) |
| 37 | `ironclaw_processes` | **runtimes** | **kernel** | `crates/ironclaw_processes` | `crates/kernel/ironclaw_processes` | retain-**widen**, move + re-layer | legalizes `processes→resources`; **[#6696]** journal + `ProcessSupervisor` DIRECTION |
| 38 | `ironclaw_turns` | kernel | kernel | `crates/ironclaw_turns` | `crates/kernel/ironclaw_turns` | retain-**narrow** (split out contracts), move | sheds `run_profile`→`loop_contracts`, `ids`/`scope` shims deleted, `external_tool_catalog`→`product` |
| 39 | `ironclaw_host_runtime` | kernel | kernel | `crates/ironclaw_host_runtime` | `crates/kernel/ironclaw_host_runtime` | retain-**narrow** (multi-way shed), move | sheds `first_party_tools`→package, `sandbox_process`→`lanes/ironclaw_sandbox`, assembly shrinks; `obligations.rs` splits internally into 3 owners |
| 40 | `ironclaw_run_state` | kernel | kernel (**transitional**) | `crates/ironclaw_run_state` | `crates/kernel/ironclaw_run_state` → **deleted** | retain-transitional → **delete-after-migration** | **[#6696]** deleted once `approvals`/`processes` absorb its three stores; frozen (no new consumers) until then |

Row numbers match PROPOSAL §9's master mapping table. Nine of ten crates require no
dependency-graph change to enter this family beyond a directory move and, for `processes`, one
layer-metadata edit; the tenth (`run_state`) is the family's only crate whose target state is
nonexistence, gated on work outside this family's control.
