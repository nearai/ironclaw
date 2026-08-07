# `crates/kernel/` — the authority perimeter

**Layer(s):** `kernel` (all nine crates declare it) · **Crates:** 9 ·
**May depend on:** layers at or below `kernel` — `contracts`, `substrates`,
`runtimes` — plus kernel siblings along the stage order (every kernel→kernel
edge is pinned by name, see below) · **Depended on by:** `loops`, `products`,
`app` — every family above reaches privileged effects only through this one.

## What this family is

The kernel is a **security perimeter, not a crate**. Every operation capable of
affecting authority, isolation, durable control-plane state, or sensitive data
crosses a kernel-mediated port. The perimeter is physicalized as nine crates,
each owning exactly one stage of the pipeline every privileged effect passes
through — deliberately nine, because each stage is an independently consumed
contract with its own fail-closed rule, and merging them would trade
compiler-proven stage separation (private mutators invisible outside their
crate) for module discipline. A caller — a loop, an extension, a product
surface — never touches the stages directly; it calls the membrane
(`ironclaw_capabilities`), and the membrane calls the rest.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_trust`](./ironclaw_trust) | requested→effective trust ceiling policy engine, sealed | the question is "what may this package be trusted with" |
| [`ironclaw_authorization`](./ironclaw_authorization) | default-deny grant matching + the capability-lease state machine | the question is "does a grant/lease cover this exact effect" |
| [`ironclaw_approvals`](./ironclaw_approvals) | exact-invocation consent: pending record → fingerprinted lease or durable denial | a human/policy decision must become something the membrane can act on |
| [`ironclaw_resources`](./ironclaw_resources) | reserve → execute → reconcile-or-release accounting over every budget dimension | costed or quota-limited work needs capacity decided before it runs |
| [`ironclaw_runtime_policy`](./ironclaw_runtime_policy) | pure `(deployment, profile, org policy) → EffectiveRuntimePolicy` + per-capability lane planning | deployment posture must pick a lane/backend, with zero I/O |
| [`ironclaw_capabilities`](./ironclaw_capabilities) | the CapabilityHost membrane: the six caller-facing workflows and the authorization fold | any caller wants a privileged effect — this is the only door |
| [`ironclaw_processes`](./ironclaw_processes) | the single durable lifecycle authority: row-native journal + supervisor | work must be claimed, leased, heartbeat, recovered, or queried "what is it doing" |
| [`ironclaw_turns`](./ironclaw_turns) | turn admission (one-active-run, idempotency) + loop-exit validation | conversational work must become durable, or a loop's exit claim must be checked |
| [`ironclaw_host_runtime`](./ironclaw_host_runtime) | mediated services + the closed lane executor: obligations, egress, secret staging, dispatch composition | an authorized witness must actually run, under mediation |

## The effect pipeline and stage ownership

Every privileged effect passes the same ordered stages; two crates bracket the
pipeline as admission/lifecycle authorities, the rest compose the membrane.

| Stage | What happens | Owning crate |
|---|---|---|
| Admission | a request becomes durable admitted work (one active run per thread, idempotent) | `ironclaw_turns` |
| Claimed execution | admitted work is claimed, leased, heartbeat-tracked to a terminal state | `ironclaw_processes` |
| Trust ceiling | requested trust resolves to a host-validated effective ceiling | `ironclaw_trust` |
| Authorization | ceiling + grants resolve to allow / deny / require-approval | `ironclaw_authorization` |
| Approval | require-approval resolves to a scoped, fingerprinted lease or a durable denial | `ironclaw_approvals` |
| Reservation | estimated cost/capacity reserved before work, reconciled after | `ironclaw_resources` |
| Policy planning | deployment/org policy select the lane and enforcement posture | `ironclaw_runtime_policy` |
| The membrane | every prior stage folds into one sealed decision (the `Authorized` witness) | `ironclaw_capabilities` |
| Mediated execution | the witness authorizes exactly one lane call — restricted mounts, staged secrets, scoped egress, redacted evidence | `ironclaw_host_runtime` |

**No stage skipping — first-party is a ceiling, not a bypass.** A higher trust
ceiling still requires explicit grants, scoped mounts, leases, budget, and
obligation handling through the same membrane every other caller uses. Nothing
shipped by the project and nothing running at an elevated trust class may reach
a privileged effect by any other path.

### The sealed mints

Four artifacts prove that a stage ran. Each has exactly one sanctioned mint;
nothing above the perimeter can fabricate one.

| Sealed artifact | Minted only by | Sealed how |
|---|---|---|
| `Authorized` witness (`ironclaw_host_api::authorized`: `CapabilityAuthorizer` grant trait :60, zero-sized `AuthorizationGrant(())` :74, `Authorized` :101) | `ironclaw_capabilities` — the sole `CapabilityAuthorizer` impl (`src/host/mod.rs:107`) | grant-gated construction + `reborn_authorized_seal_ratchet.rs::capability_authorizer_is_implemented_only_by_the_kernel` |
| Effective trust ceiling (`EffectiveTrustClass`) | `ironclaw_trust::TrustPolicy::evaluate` — privileged variants have no public constructor and no `Deserialize` (`src/decision.rs:18-33`) | crate-scoped visibility; host_api's `#[serde(skip_deserializing)]` guards the wire half |
| Fingerprinted approval lease | `ironclaw_approvals::ApprovalResolver` issues into `ironclaw_authorization`'s lease store; decision persisted **before** the lease (`ironclaw_approvals/src/lib.rs:240`) | the issuing port is public, so the mint restriction is charter + the stage `BoundaryRule`s (approvals may not name capabilities/host_runtime; nothing above the kernel may name the store crates around the membrane) — **not** a type seal |
| Verified-inbound evidence | the ingress verifier colocated in `ironclaw_extension_host` (T2) — **outside this directory but inside the conceptual perimeter**; the kernel consumes it as sealed input | `reborn_sealed_evidence_mint_ratchet.rs` (19 tests: sole-implementor census, mint-fn ownership, retired-feature absence) + `ironclaw_extension_contracts` `verified_inbound_seal` |

## What never belongs here

Each exclusion names where the thing goes instead:

- **A kernel stage held by a product, loop, or extension crate.** The nine
  stages live here and only here; a crate above the kernel that wants stage
  behavior calls the membrane. Loop strategy → `crates/loop/`; product
  workflow/UX → `crates/product/`; installable behavior →
  `crates/extensions/`.
- **Reaching upward.** No kernel crate depends on `loops`, `products`, or
  `app`. Higher layers reach back down only through ports the kernel defines
  (e.g. `ProcessExecutor` registration by `ironclaw_turn_runner` /
  `ironclaw_host_runtime`) — the port is defined low, implemented high, never
  the reverse.
- **Minting sealed evidence anywhere else.** See the mint table above. A new
  constructor, `Default`, `Deserialize`, or test-only escape hatch for
  `Authorized`, `EffectiveTrustClass`, or the verified-inbound types is a
  security regression, not a convenience.
- **Prompt assembly, mission orchestration, skill selection, channel
  presentation.** Userland: `crates/loop/`, `crates/product/`,
  `crates/domains/ironclaw_skills`.
- **Vendor-specific behavior.** Vendor names live in `crates/extensions/packages/*`,
  `ironclaw_llm` providers, and the other §8.1-rule-4 homes — never in a kernel
  crate (`reborn_extension_specificity.rs` scans).
- **Lane execution mechanics.** Container/WASM/MCP mechanics live in
  `crates/lanes/`; `ironclaw_host_runtime` holds only the closed executor and
  the adapter seam (`bollard`/`rcgen` are lane-family dependencies, not
  kernel ones — PROPOSAL §8.2 kernel row).
- **Storage-backend implementations.** Backends live behind
  `crates/substrates/ironclaw_filesystem` / `ironclaw_libsql_runtime` /
  `crates/events/ironclaw_event_store`; kernel crates consume the mount
  catalog. (`ironclaw_host_runtime`'s residual direct driver deps are frozen
  shrink-only by `reborn_persistence_driver_boundary.rs`.)
- **Raw payloads in anything a kernel crate emits.** No secrets, host paths,
  backend error detail, or unredacted user content in errors, events,
  snapshots, or logs — redaction obligations live in
  `ironclaw_host_runtime`; see `.claude/rules/safety-and-sandbox.md`.

## Fail-closed, stage by stage

Default-deny everywhere; a missing prerequisite hides or refuses the
capability, never downgrades it (`.claude/rules/safety-and-sandbox.md`).

| Stage | Fail-closed rule | Enforced / pinned by |
|---|---|---|
| Admission (`turns`) | second submit on an active thread → busy/idempotent replay, never a second run; a `LoopExit` is a claim, validated against host-minted evidence before any durable transition | `coordinator.rs`, `loop_exit.rs`; consumers drive it via the repo-root `tests/integration/` suite |
| Lifecycle (`processes`) | terminal status written once — late completions cannot overwrite (`journal_store/state.rs:603-661` terminal guards); result stored before terminal status; store queries never enumerate collections | `reborn_process_storage_scan_gate.rs::process_and_thread_request_storage_paths_do_not_enumerate_collections`; journal contract suite |
| Trust (`trust`) | privileged ceiling unobtainable outside `TrustPolicy::evaluate`; downgrade publishes on `InvalidationBus` synchronously before the lower decision returns; mutation only via `mutate_with` (`policy.rs:269`, per-source mutators `pub(crate)` — `sources.rs:144-570`) | compiler visibility + crate tests |
| Authorization (`authorization`) | no matching grant ⇒ deny; fingerprinted leases are single-winner claim-then-consume (`lib.rs:268-291`) and never become ambient grants (`lib.rs:308-313`) | crate tests; `BoundaryRule` |
| Approval (`approvals`) | decision durably persisted **before** lease issuance (`lib.rs:240`); denial is durable and issues no lease | crate tests; `reborn_origin_gate_matrix_ratchet.rs` freezes which capabilities may ever skip this stage |
| Reservation (`resources`) | reservation failure — including storage failure — is a denial, never proceed-and-true-up | `ironclaw_resources/tests/resource_governor_contract.rs` (`filesystem_resource_governor_fails_closed_then_recovers_after_delta_append_error`, `..._store_fails_closed_on_byte_only_backend`, `reserve_denies_when_usd_limit_would_be_exceeded`) |
| Policy planning (`runtime_policy`) | invalid `(deployment, profile)` → `ResolveError`, not a silent downgrade (`resolver.rs:129`); relaxed (`*Yolo*`) profiles require explicit disclosure ack (`resolver.rs:61`); process effects against `ProcessBackendKind::None` → `PlannerError` (`planner.rs:140`) | crate tests |
| Membrane (`capabilities`) | authorization denial or an unsupported/failed obligation fails **before** dispatch, process start, or lease claim; the witness is minted only by the fold and consumed once | `reborn_authorized_seal_ratchet.rs`; crate tests |
| Mediated execution (`host_runtime`) | unconfigured lane fails closed; credentials attach only over HTTPS or a **literal** loopback host (D-R); staged secrets are one-shot; no verified tenant sandbox ⇒ process/shell capability hidden (`surface.rs`) and refused by the planner — never a host shell | `host_http_egress_refuses_to_attach_a_credential_over_plaintext_http` + `..._attaches_a_credential_over_literal_loopback_http` (`src/services/tests.rs:511,:611`); `.claude/rules/safety-and-sandbox.md` |

## The rules, and what enforces them

Each is runnable; run the architecture suite after any dependency or API
change: `cargo test -p ironclaw_architecture_tests`.

- **The layer matrix.** All nine manifests declare
  `[package.metadata.ironclaw] layer = "kernel"`; the seven-layer ladder
  (`contracts < substrates < runtimes < kernel < loops < products < app`) is
  checked by `reborn_dependency_boundaries.rs`, and the exception register is
  **empty** (`LAYER_MATRIX_EXCEPTIONS = &[]`, baseline 0) — a kernel→loops or
  kernel→products edge cannot land.
- **Per-crate forbidden edges.** `BoundaryRule` entries in
  `reborn_dependency_boundaries.rs` for seven of the nine (`trust`,
  `authorization`, `approvals`, `resources`, `processes`, `turns`,
  `capabilities`) pin the stage order — e.g. `authorization` may not name
  `approvals`; `capabilities` may not name `host_runtime`.
  `ironclaw_runtime_policy` and `ironclaw_host_runtime` have **no**
  `BoundaryRule` of their own; the matrix and the same-layer inventory are
  their enforcement.
- **Same-layer edges are inventoried, not free.**
  `reborn_same_layer_edge_inventory.rs` pins all **21** kernel→kernel edges
  by name with an owner and a deciding workstream, as an equality — a new
  edge fails, a removed edge must leave the list in the same PR.
- **The witness seal.** `reborn_authorized_seal_ratchet.rs::capability_authorizer_is_implemented_only_by_the_kernel`
  — `CapabilityAuthorizer` implemented by `ironclaw_capabilities` and nowhere
  else, workspace-wide, with self-tests that the scan cannot silently degrade.
- **The evidence-mint seal.** `reborn_sealed_evidence_mint_ratchet.rs` (19
  tests) — sole-implementor census for the mint grant traits, mint functions
  named only by their owners, the retired `host-auth-mint` feature pinned
  absent across every manifest, script, and workflow.
- **Kernel never reaches the assembly root.**
  `reborn_composition_boundaries.rs::no_substrate_crate_depends_on_composition_root`
  lists eight of the nine kernel crates (`ironclaw_runtime_policy` is absent
  from that list too — measured; the layer matrix still forbids the edge).
- **Driver custody.** `reborn_persistence_driver_boundary.rs` — DB drivers
  allowlisted per crate, shrink-only; `ironclaw_host_runtime` is named
  residue (a standing narrowing target), not charter.
- **Approval-gate data stays honest.** `reborn_origin_gate_matrix_ratchet.rs`
  freezes the reviewed seed of capabilities the model may invoke ungated and
  requires a well-formed `origin_gate_matrix` on every declared capability.
- **A family directory is never a compilation or trust unit.** The enforced
  truth is each crate's `layer` metadata; family placement is ownership and
  discoverability (PROPOSAL §5). Moving a crate between families is not a
  rename — the directory carries the full package name (§5.1).
- **Before adding a tenth crate:** name your stage and your fail-closed rule,
  or your reason for more than one production implementation — otherwise it
  is a module of one of the nine (`families/kernel.md`, closing rule). A new
  crate lands with its `README.md`, a row in the table above, its layer
  metadata, and `scripts/ci/check-target-tree.py` green.

## Crossing out of this family

- **Down to `crates/contracts/`** (`ironclaw_host_api`, `ironclaw_loop_contracts`,
  `ironclaw_extension_contracts`) — for shared vocabulary and the ports the
  kernel defines for higher layers; turn/scope/id vocabulary lives in
  `host_api::turn`, never here.
- **Down to `crates/substrates/`** — mechanism the kernel mediates:
  `ironclaw_filesystem` (mounts), `ironclaw_secrets` (encrypted custody),
  `ironclaw_network` (egress transport), `ironclaw_safety`. A substrate never
  decides authority; only `host_runtime` (and the stores noted per crate)
  touch them directly.
- **Down to `crates/events/`** (`ironclaw_event_log`) — durable audit append.
- **Down to `crates/lanes/`** (`ironclaw_wasm`, `ironclaw_mcp`,
  `ironclaw_sandbox`) — only from `ironclaw_host_runtime`, to construct the
  closed executor's adapters. Lanes receive sealed work and mediated
  services; they never authorize.
- **Up to `crates/loop/` / `crates/product/`** — never as a dependency. The
  loop tier registers process executors and satisfies exit-evidence ports;
  products call `ProductSurface` → membrane. If you need behavior from up
  there, define a port here and let the upper layer implement it.

## Sources

- `docs/reborn/target-architecture/families/kernel.md` — the family spec (the
  design record; where it and the tree disagree, the code and its gates win
  and both get a dated correction).
- PROPOSAL §6.5.1–§6.5.10 (per-crate contracts), §7 (trust transitions
  T1–T8), §8 (dependency model), §11.2 (mechanical enforcement), §12.13 D-R
  (loopback carve-out) and D-S (lifecycle-authority re-verification).
- `docs/reborn/target-architecture/ws12-security-audit.md` — the 2026-08-05
  adversarial re-verification of the evidence-mint, secrets, verifier, and
  D-R seams (verdicts: HOLDS / HOLDS-WITH-RESIDUAL; residuals recorded
  there).
- `.claude/rules/safety-and-sandbox.md` — the house security frame this
  family implements.
- `docs/reborn/guidance-conventions.md` — what this file is and is not.
