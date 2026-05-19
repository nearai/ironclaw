# Phase 3 — Integration

**Status:** Proposed
**Date:** 2026-05-19
**Depends on:** all of Phase 2 (P2.A, P2.B, P2.C, P2.D), transitively all of Phase 1
**Workstream:** P3 — single workstream, one reviewable PR
**Crate:** `crates/ironclaw_reborn` (plus `crates/ironclaw_reborn/tests/`)

This is the wiring-and-verification phase. Phases 1 and 2 produce the
*components* — contracts, the `subagent` `LoopFamily`, the `subagent`
`PlannedDriver`, the spawn-handling capability port, the prompt composition,
the goal store, and the `SubagentCompletionObserver`. Phase 3 *composes* them
inside `ironclaw_reborn/src/runtime.rs`, declares the `spawn_subagent`
capability surface entry, proves the system end-to-end with integration tests
in `crates/ironclaw_reborn/tests/`, and clears the quality gate.

Phase 3 writes **no new mechanism**. If a test below needs behaviour that does
not exist, that behaviour is a Phase 2 gap — fix it there, not here.

> **Note on the overarching doc.** §11 of `README.md` says Phase 3 is
> "runtime.rs wiring". That is accurate but incomplete: the real `runtime.rs`
> today is a *generic, library-style composition* (`build_default_planned_runtime`
> / `build_product_live_planned_runtime`) parameterised over `T`, `S`, `G`. It
> does **not** construct concrete stores, observers, or capability ports — those
> arrive through `DefaultPlannedRuntimeParts`. Phase 3 therefore (a) extends the
> generic composition with the family/driver/observer wiring that is genuinely
> generic, and (b) adds a *concrete* subagent assembly seam. See §1 and §3 for
> the corrected split. Where this doc and `README.md` disagree, this doc wins
> and the divergence is called out inline.

---

## 1. Files to create / modify

### 1.1 Files modified

| File | Change |
|---|---|
| `crates/ironclaw_reborn/src/runtime.rs` | Extend `DefaultPlannedRuntimeParts` with subagent fields; wire the `subagent` family + driver into `build_default_planned_runtime`; register `SubagentCompletionObserver` as a `TurnEventSink`; thread the goal store and spawn-capable capability port. New `ProductLiveRuntimeReadinessComponent` variants for the subagent components. |
| `crates/ironclaw_reborn/src/app_loop_family.rs` | `build_loop_family_registry()` now registers **two** families: `families::default()` and `families::subagent()`. |
| `crates/ironclaw_reborn/src/planned_driver_factory.rs` | Add `register_subagent_planned_driver`, `subagent_planned_driver_descriptor`, `subagent_planned_profile_definition`, and fold the subagent profile into `default_planned_run_profile_resolver`. |
| `crates/ironclaw_reborn/src/production_readiness.rs` | Add `RebornLoopProductionComponent::SubagentGoalStore` and `::SubagentCompletionObserver`; extend `RebornLoopComponentGraphReadiness` with the two fields; add `subagent_driver_requirements()`. |
| `crates/ironclaw_reborn/src/lib.rs` | `pub mod` the new `subagent_runtime` module (see §1.2). |
| `crates/ironclaw_reborn/CLAUDE.md` | One line under "Main entry points": `subagent_runtime.rs` composes the concrete subagent assembly. |

### 1.2 Files created

| File | Purpose |
|---|---|
| `crates/ironclaw_reborn/src/subagent_runtime.rs` | The **concrete subagent assembly seam**. Owns `SubagentRuntimeParts` and `build_subagent_runtime`, which builds the durable bounded goal store, the spawn-capable capability port, the `SubagentCompletionObserver`, and returns a fully wired `RebornRuntimeLoopComposition` with the observer registered as a `TurnEventSink`. Keeps `runtime.rs` free of concrete-store construction (CLAUDE.md: "Keep `runtime.rs` limited to planned-runtime composition"). |
| `crates/ironclaw_reborn/tests/subagent_spawn_e2e.rs` | All eight end-to-end integration tests (§4). |
| `crates/ironclaw_reborn/tests/subagent_runtime_wiring.rs` | Composition-level tests: family/driver registration, profile resolution, observer-as-event-sink registration, production readiness for the subagent family. |

> The `subagent_runtime.rs` split exists because `crates/ironclaw_reborn/CLAUDE.md`
> explicitly forbids growing `runtime.rs` into a "composition catch-all" and
> says to "add a new file when adding a new ... runtime-composition concern".
> The durable goal store and observer construction are exactly that.

---

## 2. What each Phase 2 workstream hands Phase 3

Phase 3 cannot start until every item below is present. This is the integration
contract — if any name differs from what Phase 2 shipped, Phase 3 adapts and the
divergence is logged in the PR description.

| Phase 2 WS | Artifact Phase 3 consumes | Where it is used in Phase 3 |
|---|---|---|
| **P1.B / P2.C** | `ironclaw_agent_loop::families::subagent() -> LoopFamily` with `LoopFamilyId::new("subagent")` | `app_loop_family.rs::build_loop_family_registry` |
| **P1.A / P1.B** | `GateKind::AwaitDependentRun` (sealed) + `LoopGateKind::AwaitDependentRun`, `LoopBlockedKind::AwaitDependentRun`, `BlockedReason::DependentRun`, `TurnStatus::BlockedDependentRun` | flows through unchanged; asserted in §4.2 / §4.3 |
| **P1.A** | `CapabilityOutcome::SpawnedChildRun { child_run_id, result_ref, safe_summary }`; `CapabilityOutcome::AwaitDependentRun { gate_ref, safe_summary }`; `SubmitTurnRequest` / `TurnRunRecord` lineage fields; `TurnStateStore::{children_of, get_run_record}` queries; `DefaultTurnCoordinator::with_event_sink` | capability port, observer, §4 assertions |
| **P2.C** | `subagent_planned_driver()` building a `PlannedDriver` over the `subagent` family with its own descriptor + checkpoint schema | `planned_driver_factory.rs` |
| **P2.A** | The spawn-capable capability port type (call it `SpawnCapableLoopCapabilityPort` / its factory) and its `spawn_subagent` capability-id constant | `subagent_runtime.rs` |
| **P2.B** | prompt composition (direction system message + `## Task` user message) — internal to the capability port / context port; Phase 3 only asserts the *effect* (child sees the goal) |
| **P1.C** | `SubagentFlavorTable` (built-in static table: `general`, `researcher`), direction `.md` files, and the **durable bounded goal store** type (call it `BoundedSubagentGoalStore`) keyed by child `TurnRunId` | `subagent_runtime.rs` |
| **P2.D** | `SubagentCompletionObserver` implementing `TurnEventSink`, constructed from `(coordinator, turn_state_store, thread_service, goal_store, safety_layer)` | `subagent_runtime.rs` |

If a Phase 2 type name is still open at integration time, Phase 3 uses the name
in this table and Phase 2 conforms.

---

## 3. `runtime.rs` composition — exact wiring points

Today `runtime.rs` has two public builders. The `subagent` family and driver
are *generic* (they need no concrete store), so they wire into
`build_default_planned_runtime` directly. The goal store, capability port, and
observer are *concrete*, so they wire in `subagent_runtime.rs`, which calls
`build_default_planned_runtime` and then layers the observer registration on
top.

### 3.1 `app_loop_family.rs` — register the `subagent` family

```rust
// crates/ironclaw_reborn/src/app_loop_family.rs

use ironclaw_agent_loop::{families, family::{LoopFamilyRegistry, LoopFamilyRegistryError}};

/// Build the production loop-family registry.
///
/// Reborn composition root for loop families. v1 binds two Builtin families:
/// `default` (text-tool baseline) and `subagent` (child-loop family with a
/// tighter BudgetStrategy).
pub fn build_loop_family_registry() -> Result<Arc<LoopFamilyRegistry>, LoopFamilyRegistryError> {
    LoopFamilyRegistry::with_families(vec![
        Arc::new(families::default()),
        Arc::new(families::subagent()), // P1.B / P2.C
    ])
}
```

The existing test `production_registry_binds_default_family_only` is renamed to
`production_registry_binds_default_and_subagent_families` and updated:
`registry.ids().count() == 2`, both `LoopFamilyId::DEFAULT` and
`LoopFamilyId::new("subagent")` resolve.

### 3.2 `planned_driver_factory.rs` — register the `subagent` driver + profile

Mirror the `reborn:planned-default` wiring exactly. The subagent driver gets a
**distinct `LoopDriverId`** (`reborn:subagent-default`) so its
`LoopDriverRegistryKey` cannot collide with the default planned driver
(`DriverRegistry::register_driver` rejects duplicate keys).

```rust
// crates/ironclaw_reborn/src/planned_driver_factory.rs  (additions)

pub const SUBAGENT_DRIVER_ID: &str = "reborn:subagent-default";
pub const SUBAGENT_DRIVER_VERSION: u64 = 1;
pub const SUBAGENT_PROFILE_ID: &str = "reborn-subagent-default";
pub const SUBAGENT_CAPABILITY_SURFACE_ID: &str = "subagent_tools";

pub fn subagent_planned_driver_descriptor() -> Result<AgentLoopDriverDescriptor, String> {
    AgentLoopDriverDescriptor::new(SUBAGENT_DRIVER_ID, RunProfileVersion::new(SUBAGENT_DRIVER_VERSION))?
        .with_checkpoint_schema(
            PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID, // same canonical CHECKPOINT_SCHEMA_ID
            planned_driver_checkpoint_schema_version(),
        )
}

/// Build the `subagent` PlannedDriver over the `subagent` LoopFamily.
pub fn subagent_planned_driver(
    family_registry: Arc<LoopFamilyRegistry>,
) -> Result<DefaultPlannedDriverBuild, AgentLoopDriverError> {
    let family = family_registry
        .get(&LoopFamilyId::new("subagent").map_err(|reason| {
            AgentLoopDriverError::InvalidRequest { reason }
        })?)
        .ok_or_else(|| AgentLoopDriverError::InvalidRequest {
            reason: "subagent loop family is not registered".to_string(),
        })?;
    let descriptor = subagent_planned_driver_descriptor()
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;
    let executor = Arc::new(CanonicalAgentLoopExecutor);
    let driver = PlannedDriver::from_family_with_descriptor(family, executor, descriptor.clone())?;
    Ok(DefaultPlannedDriverBuild { driver: Arc::new(driver), descriptor })
}

pub fn register_subagent_planned_driver(
    registry: &mut DriverRegistry,
    family_registry: Arc<LoopFamilyRegistry>,
) -> Result<LoopDriverRegistryKey, DefaultPlannedDriverRegistrationError> {
    let build = subagent_planned_driver(family_registry)?;
    registry
        .register_driver(build.driver, planned_driver_requirements(), DriverKind::Production)
        .map_err(Into::into)
}

pub fn subagent_planned_profile_definition() -> Result<RunProfileDefinition, RunProfileRegistryError> {
    let descriptor = subagent_planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let profile_id = RunProfileId::new(SUBAGENT_PROFILE_ID)
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let checkpoint_schema_id = planned_driver_checkpoint_schema_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    // Child runs see ONLY the `subagent_tools` ceiling — never `interactive_tools`.
    let capability_surface_profile_id = CapabilitySurfaceProfileId::new(SUBAGENT_CAPABILITY_SURFACE_ID)
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    Ok(RunProfileDefinition::interactive_like(
        profile_id, descriptor, checkpoint_schema_id,
        planned_driver_checkpoint_schema_version(), capability_surface_profile_id,
    ))
}
```

`default_planned_run_profile_resolver()` registers the subagent profile so the
spawn path can resolve it by id without re-plumbing the resolver:

```rust
pub fn default_planned_run_profile_resolver()
-> Result<InMemoryRunProfileResolver, RunProfileRegistryError> {
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    register_default_planned_profile(&mut registry)?;
    registry.register(subagent_planned_profile_definition()?)?;   // ◄ new
    let implicit_default = planned_default_profile_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    Ok(InMemoryRunProfileResolver::new_with_implicit_default(registry, implicit_default))
}
```

The implicit default is **unchanged** — `reborn-planned-default`. A subagent run
is reached only by *explicit* `RunProfileRequest::new("reborn-subagent-default")`,
which is exactly what the spawn capability port issues when it builds the child
`SubmitTurnRequest`. No interactive turn ever lands on the subagent profile.

### 3.3 `build_default_planned_runtime` — register the subagent driver

The single change inside `build_default_planned_runtime` (after
`register_default_planned_driver`):

```rust
// crates/ironclaw_reborn/src/runtime.rs  (inside build_default_planned_runtime)

    let mut registry = DriverRegistry::new();
    register_default_text_only_driver(&mut registry, parts.config.text_only_driver)?;
    let family_registry = build_loop_family_registry().map_err(/* ... */)?;
    register_default_planned_driver(&mut registry, Arc::clone(&family_registry))?;
    register_subagent_planned_driver(&mut registry, family_registry)?;   // ◄ new
    let driver_registry = Arc::new(registry);
```

`build_loop_family_registry()` now returns the 2-family registry, and
`register_subagent_planned_driver` borrows it via `Arc::clone`. Note the
existing call passes `family_registry` by value to `register_default_planned_driver`;
change it to `Arc::clone(&family_registry)` so the second registration can reuse
it. Both drivers share the canonical `CanonicalAgentLoopExecutor`; their
`LoopDriverRegistryKey`s differ on `id`, so registration cannot collide
(asserted by `key_collision_with_textonly_is_impossible`-style test, §4 wiring
suite).

`requirements_snapshot()` on the registry now carries three drivers; the host
factory's `with_driver_requirements` call is unchanged and picks all of them up.

### 3.4 `runtime.rs` — extend `DefaultPlannedRuntimeParts`

The subagent goal store, spawn-capable capability port, and observer are
*concrete* and arrive through `parts`, exactly like `cancellation_factory` and
`identity_context_source`. Add three fields:

```rust
// crates/ironclaw_reborn/src/runtime.rs  (DefaultPlannedRuntimeParts)

pub struct DefaultPlannedRuntimeParts<T, S, G>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    // ... all existing fields unchanged ...

    /// Durable bounded goal store keyed by child `TurnRunId` (P1.C).
    /// `build_product_live_planned_runtime` fails closed when `None`.
    pub subagent_goal_store: Option<Arc<dyn SubagentGoalStore>>,

    /// Completion observer for child runs (P2.D). When present it is
    /// registered as a `TurnEventSink` on the coordinator.
    pub subagent_completion_observer: Option<Arc<SubagentCompletionObserver>>,
}
```

`SubagentGoalStore` is the durable-store trait from P1.C;
`BoundedSubagentGoalStore` is its bounded in-process implementation.
`SubagentCompletionObserver` is the P2.D type.

> **Divergence from README §5.3.** README lists the observer and goal store
> only under "ironclaw_reborn ... + durable goal store / observer / runtime.rs
> wiring". It does not say they enter through `DefaultPlannedRuntimeParts`. They
> must, because `runtime.rs` is generic over `T/S/G` and cannot itself decide
> the concrete store backend. Phase 3 makes them `Option<…>` parts, consistent
> with every other product-live extension already in the struct.

### 3.5 `runtime.rs` — register the observer as a `TurnEventSink`

`DefaultTurnCoordinator` currently takes a `wake_notifier` but **no event sink**.
The coordinator publishes lifecycle transitions; the observer must receive them.
Two integration options — Phase 3 uses option (a) because it touches no
`ironclaw_turns` API:

- **(a) Composite event sink, chosen.** The coordinator already accepts a
  `TurnRunWakeNotifier`. P2.D's `SubagentCompletionObserver` is *also* wired to
  fire on terminal child events. The cleanest seam without a new coordinator
  setter: the observer subscribes to the **same `TurnLifecycleEvent` stream**
  the coordinator emits, via a `TurnEventSink` the coordinator is given.

  If `DefaultTurnCoordinator` has no `with_event_sink` setter at integration
  time, that setter is a **Phase 2 (P1.A) deliverable**, not Phase 3 — Phase 3
  only *calls* it. The wiring:

```rust
// crates/ironclaw_reborn/src/runtime.rs  (inside build_default_planned_runtime)

    let mut coordinator = DefaultTurnCoordinator::new(Arc::clone(&parts.turn_state))
        .with_run_profile_resolver(Arc::clone(&run_profile_resolver))
        .with_wake_notifier(wake_notifier);
    if let Some(observer) = parts.subagent_completion_observer.clone() {
        let event_sink: Arc<dyn TurnEventSink> = observer;
        coordinator = coordinator.with_event_sink(event_sink);   // ◄ P1.A setter
    }
    let coordinator = Arc::new(coordinator);
```

`SubagentCompletionObserver` implements `TurnEventSink`:
`async fn publish(&self, event: TurnLifecycleEvent)`. On a `Completed` /
`Failed` / `Cancelled` event whose `run_id` has a non-`None` `parent_run_id`
(looked up via `turn_state_store`), the observer runs the §7.2 / §7.1 logic
from the README.

### 3.6 `runtime.rs` — product-live readiness for the new parts

Extend `ProductLiveRuntimeReadinessComponent` and the fail-closed checks in
`build_product_live_planned_runtime`:

```rust
// crates/ironclaw_reborn/src/runtime.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLiveRuntimeReadinessComponent {
    ModelRouteResolver,
    InputQueue,
    CancellationFactory,
    IdentityContextSource,
    ModelPolicyGuard,
    ModelBudgetAccountant,
    SafetyContext,
    SubagentGoalStore,            // ◄ new
    SubagentCompletionObserver,   // ◄ new
}

impl ProductLiveRuntimeReadinessComponent {
    pub fn as_str(self) -> &'static str {
        match self {
            // ... existing ...
            Self::SubagentGoalStore => "subagent_goal_store",
            Self::SubagentCompletionObserver => "subagent_completion_observer",
        }
    }
}
```

And in `build_product_live_planned_runtime`, after the `safety_context` check:

```rust
    if parts.subagent_goal_store.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::SubagentGoalStore,
        ));
    }
    if parts.subagent_completion_observer.is_none() {
        return Err(ProductLiveRuntimeBuildError::Missing(
            ProductLiveRuntimeReadinessComponent::SubagentCompletionObserver,
        ));
    }
```

Rationale: a product-live runtime that registers the `subagent` driver/profile
but has no goal store would let a child run reach `## Task` with a store miss —
which §9 of the README mandates must "fail the child run loudly". Failing
*closed at composition time* is strictly safer than failing per-run. The
generic `build_default_planned_runtime` keeps both `Option`, so helper-level
tests that exercise only the default family still compile (matching the
existing pattern for `cancellation_factory`).

### 3.7 `subagent_runtime.rs` — the concrete assembly seam

```rust
// crates/ironclaw_reborn/src/subagent_runtime.rs  (new file)

use std::sync::Arc;

use ironclaw_loop_support::SpawnCapableLoopCapabilityPortFactory; // P2.A
use ironclaw_turns::{TurnCoordinator, ...};

use crate::{
    runtime::{
        DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition, build_default_planned_runtime,
        DefaultPlannedRuntimeBuildError,
    },
    subagent_flavors::SubagentFlavorTable,            // P1.C
    subagent_goal_store::BoundedSubagentGoalStore,    // P1.C
    subagent_observer::SubagentCompletionObserver,    // P2.D
};

/// Concrete dependencies for the subagent assembly. The caller still supplies
/// the generic runtime parts; this struct adds the subagent-specific knobs.
pub struct SubagentRuntimeParts {
    /// Bound on the durable goal store (fork-bomb / memory containment).
    pub goal_store_capacity: usize,
    /// Built-in static flavor table (`general`, `researcher`).
    pub flavor_table: Arc<SubagentFlavorTable>,
    /// Caps enforced before `submit_turn` (README §8.2).
    pub spawn_caps: SubagentSpawnCaps,
}

#[derive(Debug, Clone, Copy)]
pub struct SubagentSpawnCaps {
    pub max_subagent_depth: u32,
    pub max_spawn_per_turn: u32,
    pub max_tree_descendants: u32,
}

/// Build a fully wired Reborn runtime composition with subagent spawn enabled.
///
/// This is the ONE place that constructs the concrete goal store, the
/// spawn-capable capability port, and the completion observer. It then hands
/// them to `build_default_planned_runtime` via the `Option` parts and returns
/// the composition with the observer registered as a `TurnEventSink`.
pub fn build_subagent_runtime<T, S, G>(
    mut parts: DefaultPlannedRuntimeParts<T, S, G>,
    subagent: SubagentRuntimeParts,
) -> Result<RebornRuntimeLoopComposition<T, S, G>, DefaultPlannedRuntimeBuildError>
where
    T: TurnStateStore + TurnRunTransitionPort + Send + Sync + 'static,
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    // 1. Durable, bounded goal store keyed by child TurnRunId.
    let goal_store: Arc<dyn SubagentGoalStore> =
        Arc::new(BoundedSubagentGoalStore::with_capacity(subagent.goal_store_capacity));

    // 2. Completion observer — needs the coordinator to resume / submit follow-up
    //    turns. The coordinator does not exist yet, so the observer is built with
    //    a deferred coordinator handle (Arc<OnceLock<...>>), set in step 4.
    let observer = Arc::new(SubagentCompletionObserver::new(
        Arc::clone(&parts.turn_state),
        Arc::clone(&parts.thread_service),
        Arc::clone(&goal_store),
        parts.safety_context.clone(),     // child output is untrusted -> safety scan
    ));

    // 3. Spawn-capable capability port factory wraps the surface-profiled port.
    //    It recognises the `spawn_subagent` capability id, performs the spawn,
    //    and returns CapabilityOutcome::SpawnedChildRun / AwaitDependentRun gate.
    let spawn_port_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(SpawnCapableLoopCapabilityPortFactory::new(
            parts.capability_factory.clone(),     // inner = base profiled port
            Arc::clone(&goal_store),
            Arc::clone(&subagent.flavor_table),
            subagent.spawn_caps,
        ));
    parts.capability_factory = spawn_port_factory;
    parts.subagent_goal_store = Some(goal_store);
    parts.subagent_completion_observer = Some(Arc::clone(&observer));

    // 4. Build the generic runtime — registers the subagent family/driver and
    //    binds the observer as a TurnEventSink (runtime.rs §3.3 / §3.5).
    let composition = build_default_planned_runtime(parts)?;

    // 5. Hand the observer the coordinator handle it deferred in step 2 so it
    //    can resume blocked parents / submit coalescing follow-up turns.
    observer.bind_coordinator(Arc::clone(&composition.coordinator) as Arc<dyn TurnCoordinator>);

    Ok(composition)
}
```

The deferred-coordinator pattern (`OnceLock<Arc<dyn TurnCoordinator>>` inside
the observer) is required because the observer must be passed *into*
`build_default_planned_runtime` (to register as the event sink) but also needs
the *coordinator that builder returns*. `bind_coordinator` panics if called
twice; it is called exactly once, here.

A product-live variant `build_product_live_subagent_runtime` mirrors
`build_product_live_planned_runtime`: same fail-closed prelude (it runs the
existing seven checks plus §3.6's two new ones), then calls `build_subagent_runtime`.

---

## 4. `spawn_subagent` capability surface declaration

`spawn_subagent` is an **ordinary capability** on the surface (README §5.2). The
parent loop sees it through `LoopCapabilityPort::visible_capabilities`; the
executor invokes it through `invoke_capability_batch`. The host's spawn-capable
port (`SpawnCapableLoopCapabilityPort`, P2.A) intercepts the `spawn_subagent`
capability id and bypasses the host-runtime dispatch.

### 4.1 Capability descriptor

`spawn_subagent` is registered against the `subagent_tools` *and* the
`interactive_tools` capability surface profile — the parent (an interactive
run) must *see* it; the child (a `subagent_tools` run) only sees it when its
flavor has `allow_nesting = true`.

```rust
// declared in P2.A; Phase 3 wires it into the surface resolver.

pub const SPAWN_SUBAGENT_CAPABILITY_ID: &str = "ironclaw.spawn_subagent";

/// Surface descriptor view the parent loop receives.
CapabilityDescriptorView {
    capability_id: CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
    provider: Some(ExtensionId::new("reborn:subagent-default")?),
    runtime: RuntimeKind::FirstParty,
    safe_name: "spawn_subagent".to_string(),
    safe_description:
        "Spawn a child agent loop with a fresh context and attenuated tools."
            .to_string(),
    // SpawnProcess-class effect => Exclusive: spawn calls serialise within a
    // batch so per-turn ordinals (and thus idempotency keys) are deterministic.
    concurrency_hint: ConcurrencyHint::Exclusive,
}
```

`ConcurrencyHint::Exclusive` is load-bearing: the idempotency key is
`(parent_run_id, parent_turn_id, spawn-call ordinal)` (README §6), and the
ordinal is only deterministic if the batch processes spawn calls serially.
`invoke_capability_batch` already iterates serially, so a model emitting N
`spawn_subagent` calls in one turn gets ordinals `0..N` deterministically.

### 4.2 Capability input

The model-supplied input (resolved through `CapabilityInputRef` →
`LoopCapabilityInputResolver`) deserialises to:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SpawnSubagentInput {
    /// Flavor selector — `general` | `researcher`. Resolved against the static
    /// SubagentFlavorTable; an unknown value is a `Denied` outcome.
    pub agent_type: String,
    /// The child's task. Becomes the child's first USER message under
    /// `## Task (from parent)` — never the system message (README §8.4).
    pub goal: String,
    /// Context seed. `Fresh` (goal only) or `Handoff` (goal + curated blob).
    /// `Fork` is reserved/unimplemented -> Denied.
    #[serde(default)]
    pub seed: SubagentSeed,
    /// false = blocking (parent waits); true = background (parent continues).
    #[serde(default)]
    pub run_in_background: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "mode", content = "context")]
pub enum SubagentSeed {
    #[default]
    Fresh,
    Handoff(String),
    // Fork reserved — not deserialisable in v1; absence => Denied if requested.
}
```

### 4.3 Capability output

`SpawnCapableLoopCapabilityPort::invoke_capability` returns, per the spawn
mode:

- **background** (`run_in_background = true`):
  `CapabilityOutcome::SpawnedChildRun { child_run_id, result_ref, safe_summary }`
  (P1.A variant). The executor pushes `result_ref` as the tool result and keeps
  `child_run_id` for lineage/observability. The parent turn completes normally.
- **blocking** (`run_in_background = false`):
  `CapabilityOutcome::AwaitDependentRun { gate_ref, safe_summary }`. The
  executor maps it to `GateKind::AwaitDependentRun`, checkpoints `BeforeBlock`,
  and returns `LoopExit::Blocked` with `LoopBlockedKind::AwaitDependentRun`.
- **rejected** (depth / fan-out / nesting / unknown flavor / `Fork`):
  `CapabilityOutcome::Denied(CapabilityDenied { reason_kind, safe_summary })`
  with a typed `reason_kind` — `subagent_depth_exceeded`,
  `subagent_fanout_exceeded`, `subagent_tree_exceeded`, `nesting_not_allowed`,
  `unknown_subagent_flavor`, `fork_seed_unimplemented`. Rejection happens
  **before** any `submit_turn` (README §8.2).

The "all children already terminal when the parent would block" case
(README §9) is resolved **inside the gate-entry reconciliation** — that is
executor + P1.B behaviour, asserted by §5.4 here.

---

## 5. Integration test plan — `tests/subagent_spawn_e2e.rs`

### 5.0 Shared harness

The existing tests drive a `PlannedDriver` directly against
`MockAgentLoopDriverHost` (see `planned_driver_e2e.rs`). That harness exercises
*one* run. The subagent E2E tests need **a real runner pool** so child runs
actually execute. Phase 3 builds a `SubagentTestHarness` on top of the **real
composition** (`build_subagent_runtime`) with these substitutions:

| Component | Real or test double | Why |
|---|---|---|
| `TurnStateStore` + `TurnRunTransitionPort` | in-memory impl from `ironclaw_turns` test support (the same one `coordinator` tests use) | needs `children_of`, `get_run_record`, `parent_run_id` — real query semantics |
| `SessionThreadService` | in-memory thread service | child threads must be creatable; transcript must be readable |
| `HostManagedModelGateway` | **scripted** gateway — per-thread script keyed by `thread_id` | parent emits `spawn_subagent` tool calls; children emit a reply |
| `BoundedSubagentGoalStore` | **real** | durability + bounded eviction are under test |
| `SubagentCompletionObserver` | **real** | this is the SUT |
| `SpawnCapableLoopCapabilityPort` | **real** | this is the SUT |
| `TurnRunnerWorker` | **real**, started as a background task with a small pool | children must run concurrently |
| cancellation factory | real `TurnStateRunCancellationFactory` | cancellation-subtree test needs it |

The scripted gateway is the key test instrument. It maps a `thread_id` (or run
profile id) to a `VecDeque<ScriptedModelResponse>`:

```rust
struct SubagentTestHarness {
    composition: RebornRuntimeLoopComposition<MemTurnStore, MemThreadService, ScriptedGateway>,
    coordinator: Arc<dyn TurnCoordinator>,
    turn_state: Arc<MemTurnStore>,
    thread_service: Arc<MemThreadService>,
    goal_store: Arc<BoundedSubagentGoalStore>,
    gateway: Arc<ScriptedGateway>,
    _worker: tokio::task::JoinHandle<()>,
}

impl SubagentTestHarness {
    /// Build the real composition + start the runner worker pool.
    fn start(caps: SubagentSpawnCaps) -> Self { /* build_subagent_runtime + spawn worker */ }

    /// Script: when a turn runs on `thread`, the model emits `responses` in order.
    fn script_thread(&self, thread: &ThreadId, responses: Vec<ScriptedModelResponse>) { ... }

    /// Convenience: script a parent turn to emit one spawn_subagent tool call.
    fn script_parent_spawn(&self, thread: &ThreadId, input: SpawnSubagentInput) { ... }

    /// Submit a top-level interactive turn and return its run id.
    async fn submit_root_turn(&self, thread: &ThreadId) -> TurnRunId { ... }

    /// Poll get_run_state until `run` is terminal or `deadline` elapses.
    async fn await_terminal(&self, run: TurnRunId, deadline: Duration) -> TurnRunState { ... }

    /// Poll get_run_state until `run` reaches `status` (e.g. BlockedDependentRun).
    async fn await_status(&self, run: TurnRunId, status: TurnStatus, deadline: Duration)
        -> TurnRunState { ... }

    /// All child run ids of `parent`, via the durable children_of store query.
    async fn children_of(&self, parent: TurnRunId) -> Vec<TurnRunState> { ... }
}
```

All `await_*` helpers use a bounded poll loop (`tokio::time::timeout` +
`get_run_state`) — never an unbounded wait — so a wiring bug fails as a test
*timeout*, not a hang.

---

### 5.1 Background spawn — E2E

**Goal:** a background (`run_in_background = true`) spawn delivers the child
result into the parent thread and triggers a coalescing follow-up parent turn.

**Setup**
- Caps: generous (`depth 4`, `fanout 4`, `tree 16`).
- Parent thread `T_p`. Script parent turn 1: emit one `spawn_subagent`
  (`agent_type="general"`, `goal="summarise X"`, `run_in_background=true`).
- Script the *child* run profile (`reborn-subagent-default`) to reply
  `"child done: summary of X"`.
- Script parent turn 2 (the coalesced follow-up): reply `"parent final"`.

**Assertions**
1. Parent run 1 reaches `TurnStatus::Completed` — it does **not** block.
2. Parent turn 1's tool result for the spawn call carries a child run id
   (`CapabilityOutcome::SpawnedChildRun`). Read it back from the parent
   transcript / result refs.
3. `children_of(parent_run_1)` returns exactly one child; the child's
   `parent_run_id == parent_run_1`, `subagent_depth == 1`.
4. Child run reaches `Completed`.
5. The child `TurnScope` copies `tenant_id` / `agent_id` / `project_id` from the
   parent verbatim and has a **different `thread_id`** (README §6 tenancy
   invariant; also the no-deadlock precondition).
6. The parent thread `T_p` gains an inbound message containing the child's
   result, wrapped in a delimited block (`## Subagent result` or equivalent) —
   not raw.
7. A follow-up parent run exists, is `Completed`, and ran on `T_p`.
8. The goal store entry for the child `TurnRunId` was written before
   `submit_turn` and is still readable (or has been cleaned per P1.C policy).

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn background_spawn_delivers_child_result_and_runs_followup() {
    let h = SubagentTestHarness::start(SubagentSpawnCaps {
        max_subagent_depth: 4, max_spawn_per_turn: 4, max_tree_descendants: 16,
    });
    let t_p = h.thread_service.create_thread(/* parent scope */).await;

    h.script_parent_spawn(&t_p, SpawnSubagentInput {
        agent_type: "general".into(),
        goal: "summarise X".into(),
        seed: SubagentSeed::Fresh,
        run_in_background: true,
    });
    h.script_profile("reborn-subagent-default",
        vec![ScriptedModelResponse::Reply { text: "child done: summary of X".into() }]);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply { text: "parent final".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;

    let parent_state = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(parent_state.status, TurnStatus::Completed);   // never blocked

    let children = h.children_of(parent_run).await;
    assert_eq!(children.len(), 1);
    let child = &children[0];
    assert_eq!(child.scope.tenant_id, parent_state.scope.tenant_id);
    assert_eq!(child.scope.agent_id,  parent_state.scope.agent_id);
    assert_eq!(child.scope.project_id, parent_state.scope.project_id);
    assert_ne!(child.scope.thread_id, parent_state.scope.thread_id);

    let child_state = h.await_terminal(child.run_id, Duration::from_secs(5)).await;
    assert_eq!(child_state.status, TurnStatus::Completed);

    // child result was accepted into the parent thread, delimited
    let inbound = h.thread_service.history(&t_p).await;
    let result_msg = inbound.iter().find(|m| m.kind == MessageKind::User
        && m.content.as_deref().is_some_and(|c| c.contains("summary of X"))).unwrap();
    assert!(result_msg.content.as_deref().unwrap().contains("## Subagent result"));

    // coalescing follow-up parent turn ran
    let parent_runs = h.turn_state.runs_for_thread(&t_p).await;
    assert!(parent_runs.iter().any(|r| r.run_id != parent_run
        && r.status == TurnStatus::Completed));
}
```

---

### 5.2 Blocking spawn — E2E

**Goal:** a blocking spawn parks the parent on an `AwaitDependentRun` gate,
releases the worker, and resumes the parent once with the child result mapped
back to the spawn call.

**Setup**
- Caps generous.
- Parent thread `T_p`. Script parent turn 1: one `spawn_subagent`
  (`run_in_background=false`).
- Child profile: a model script that **does not reply immediately** — gate it
  on a `tokio::sync::Notify` so the test can observe the parent in
  `BlockedDependentRun` before the child finishes.
- Script the parent *resume* turn: reply `"parent final after child"`.

**Assertions**
1. Parent run 1 reaches `TurnStatus::BlockedDependentRun` (not
   `BlockedApproval`/`BlockedAuth`/`BlockedResource`).
2. `parent_state.gate_ref` is `Some(_)` and the gate is the synthetic
   `AwaitDependentRun` gate ref (one `GateRef` for the whole awaited set —
   README §6 resume payload).
3. While the parent is blocked, the runner worker is **released** — assert by
   submitting an unrelated turn on a *different* thread and seeing it run to
   completion while the parent stays blocked.
4. After the child is released and completes, the parent transitions
   `BlockedDependentRun -> Running -> Completed`.
5. The child result is delivered as the **tool result** of the spawn call (not
   an inbound message — that is the background path). Assert the resumed
   parent transcript shows the spawn call's result ref resolves to the
   sanitised child output.

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn blocking_spawn_parks_parent_then_resumes_with_child_result() {
    let h = SubagentTestHarness::start(generous_caps());
    let t_p = h.thread_service.create_thread(/* parent */).await;
    let child_gate = Arc::new(tokio::sync::Notify::new());

    h.script_parent_spawn(&t_p, SpawnSubagentInput {
        agent_type: "general".into(), goal: "deep task".into(),
        seed: SubagentSeed::Fresh, run_in_background: false,
    });
    h.script_profile_gated("reborn-subagent-default", Arc::clone(&child_gate),
        vec![ScriptedModelResponse::Reply { text: "child result payload".into() }]);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply {
        text: "parent final after child".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;

    // 1 + 2: parent parks on the AwaitDependentRun gate
    let blocked = h.await_status(parent_run, TurnStatus::BlockedDependentRun,
        Duration::from_secs(5)).await;
    assert!(blocked.gate_ref.is_some());

    // 3: worker is free — an unrelated turn on another thread still runs
    let t_other = h.thread_service.create_thread(/* other */).await;
    h.script_thread(&t_other, vec![ScriptedModelResponse::Reply { text: "ok".into() }]);
    let other_run = h.submit_root_turn(&t_other).await;
    let other_state = h.await_terminal(other_run, Duration::from_secs(5)).await;
    assert_eq!(other_state.status, TurnStatus::Completed);
    assert_eq!(h.poll_status(parent_run).await, TurnStatus::BlockedDependentRun);

    // 4: release the child; parent resumes to terminal
    child_gate.notify_waiters();
    let parent_final = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(parent_final.status, TurnStatus::Completed);

    // 5: child result is the spawn call's tool result, sanitised + delimited
    let transcript = h.thread_service.history(&t_p).await;
    assert!(transcript.iter().any(|m| m.kind == MessageKind::ToolResultReference
        && m.content.as_deref().is_some_and(|c| c.contains("child result payload"))));
}
```

---

### 5.3 Parallel blocking spawn — E2E

**Goal:** one parent turn spawns N blocking children that run concurrently; the
parent resumes **once**, after the **last** child is terminal, with N results
mapped back to the N spawn calls.

**Setup**
- Caps: `fanout >= 3`.
- Parent turn 1 emits **3** `spawn_subagent` calls (all blocking), distinct
  goals, all `agent_type="general"`.
- Three child gates `g0,g1,g2`; child profile scripts reply distinctly
  (`"c0"`,`"c1"`,`"c2"`) keyed by goal.

**Assertions**
1. `children_of(parent)` returns exactly 3, each `subagent_depth == 1`, each a
   distinct `thread_id`.
2. The 3 children have **distinct idempotency keys** even though two could
   share arguments — verified by giving two of the three *identical* goals and
   asserting 3 distinct child run ids still exist (README §6 ordinal key).
3. Parent is `BlockedDependentRun` after turn 1.
4. Release `g0` then `g1` — parent stays `BlockedDependentRun` (gate waits for
   **all**, no early resume).
5. Release `g2` — parent resumes to `Completed`.
6. The resumed parent turn sees 3 tool results, one per spawn call, in spawn
   ordinal order.

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn parallel_blocking_spawn_resumes_once_after_last_child() {
    let h = SubagentTestHarness::start(generous_caps());
    let t_p = h.thread_service.create_thread(/* parent */).await;
    let gates = [Arc::new(Notify::new()), Arc::new(Notify::new()), Arc::new(Notify::new())];

    // turn 1: three blocking spawns — note goals 0 and 2 are identical on purpose
    h.script_parent_multi_spawn(&t_p, vec![
        spawn("general", "shared goal", false),
        spawn("general", "unique goal", false),
        spawn("general", "shared goal", false),
    ]);
    h.script_children_by_goal(/* goal -> (gate, reply) */ &gates);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply { text: "merged".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;

    let children = {
        h.await_status(parent_run, TurnStatus::BlockedDependentRun, Duration::from_secs(5)).await;
        h.children_of(parent_run).await
    };
    assert_eq!(children.len(), 3);
    assert_eq!(children.iter().map(|c| c.run_id).collect::<HashSet<_>>().len(), 3); // distinct
    assert_eq!(children.iter().map(|c| c.scope.thread_id.clone())
        .collect::<HashSet<_>>().len(), 3);

    gates[0].notify_waiters();
    gates[1].notify_waiters();
    // gate waits for ALL — still blocked
    h.settle().await;
    assert_eq!(h.poll_status(parent_run).await, TurnStatus::BlockedDependentRun);

    gates[2].notify_waiters();
    let parent_final = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(parent_final.status, TurnStatus::Completed);

    let tool_results = h.spawn_call_tool_results(parent_run).await; // ordinal-ordered
    assert_eq!(tool_results.len(), 3);
}
```

---

### 5.4 Early completion — all children finish before the parent blocks

**Goal:** if every awaited child is already terminal at the moment the parent
would block, the `AwaitDependentRun` gate **resolves inline** — the parent never
emits `Blocked` (README §9).

**Setup**
- Caps generous.
- Parent turn 1: 2 blocking spawns.
- Child profile scripts reply **immediately and synchronously** (no gate). To
  make the race deterministic, the harness uses a `single_threaded` runner pool
  and a scheduling hook so the children are driven to terminal *before* the
  parent's gate-entry reconciliation runs — or, more robustly, the test asserts
  the *outcome* (no `Blocked` lifecycle event ever observed for the parent)
  rather than trying to control the race.

**Assertions**
1. The parent run's lifecycle event log (`InMemoryTurnEventSink`, attached as a
   second event sink in the harness) contains **no** `TurnEventKind::Blocked`
   event for `parent_run`.
2. Parent reaches `Completed` directly from `Running`.
3. Both children are `Completed`, both results land as the spawn calls' tool
   results in the same turn.

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn early_completion_resolves_gate_inline_without_blocked() {
    let h = SubagentTestHarness::start_single_threaded(generous_caps());
    let events = h.attach_event_recorder(); // extra InMemoryTurnEventSink
    let t_p = h.thread_service.create_thread(/* parent */).await;

    h.script_parent_multi_spawn(&t_p, vec![
        spawn("general", "fast a", false),
        spawn("general", "fast b", false),
    ]);
    h.script_profile("reborn-subagent-default",
        vec![ScriptedModelResponse::Reply { text: "fast done".into() }]);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply { text: "done".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;
    let final_state = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(final_state.status, TurnStatus::Completed);

    let parent_events = events.events().into_iter()
        .filter(|e| e.run_id == parent_run).collect::<Vec<_>>();
    assert!(parent_events.iter().all(|e| e.kind != TurnEventKind::Blocked),
        "early-completion parent must never emit a Blocked event");

    for child in h.children_of(parent_run).await {
        assert_eq!(child.status, TurnStatus::Completed);
    }
}
```

---

### 5.5 Child-authority enforcement

**Goal:** a child run starts with an **empty grant/lease set** — it cannot
exercise a privileged lease the parent already holds. The capability allowlist
is a surface *ceiling*, not authority (README §8.1).

**Setup**
- Give the *parent* run a host-issued grant for a privileged capability
  (e.g. `demo.write` with `EffectKind::WriteFilesystem`) — i.e. the parent's
  `ExecutionContext.grants` carries a real `CapabilityGrant`.
- Spawn one blocking child whose flavor surface *includes* `demo.write` in its
  allowlist ceiling.
- Script the child to invoke `demo.write`.

**Assertions**
1. The child's `invoke_capability("demo.write")` yields a **suspension** — an
   `Approval` gate (`CapabilityOutcome::ApprovalRequired`) — *not* a `Completed`
   outcome. The child must re-acquire the lease through its own `Approval` gate
   on its own thread; it cannot inherit the parent's grant.
2. Equivalently at the `invocation_grants_from_visible` layer: the child's
   `ExecutionContext.grants` is empty for `demo.write` (no inherited grant). The
   capability port's `invocation_grants_from_visible` filters by grantee
   principal and `issued_by == HostRuntime`; a child context with no copied
   grants produces an empty `CapabilitySet`.
3. The child run reaches `BlockedApproval` (its own gate) — it does **not**
   complete `demo.write` silently.

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn child_cannot_use_lease_the_parent_holds() {
    let h = SubagentTestHarness::start(generous_caps());
    let t_p = h.thread_service.create_thread(/* parent */).await;

    // parent holds a real host-issued grant for a privileged capability
    h.grant_parent_capability(&t_p, "demo.write", &[EffectKind::WriteFilesystem]);

    // child flavor's surface ceiling includes demo.write
    h.script_parent_spawn(&t_p, SpawnSubagentInput {
        agent_type: "general".into(), goal: "write a file".into(),
        seed: SubagentSeed::Fresh, run_in_background: false,
    });
    h.script_profile("reborn-subagent-default",
        vec![ScriptedModelResponse::ToolCall { capability: "demo.write", input: json!({}) }]);

    let parent_run = h.submit_root_turn(&t_p).await;
    h.await_status(parent_run, TurnStatus::BlockedDependentRun, Duration::from_secs(5)).await;

    let child = &h.children_of(parent_run).await[0];
    let child_state = h.await_status(child.run_id, TurnStatus::BlockedApproval,
        Duration::from_secs(5)).await;

    // child blocked on ITS OWN approval gate — it did not inherit the parent's grant
    assert_eq!(child_state.status, TurnStatus::BlockedApproval);
    assert!(h.child_execution_context(child.run_id).grants.grants.is_empty(),
        "child run must start with an empty grant set");
}
```

---

### 5.6 Fork-bomb caps reject

**Goal:** all three caps reject **before `submit_turn`** — no child is queued.

**Setup:** three sub-cases, one assertion block each, all in one
`#[tokio::test]` for compactness.

- **Depth cap.** `max_subagent_depth = 1`. Parent (depth 0) spawns a child
  (depth 1, OK). The child's flavor has `allow_nesting = true` and is scripted
  to spawn a grandchild (depth 2, must reject).
- **Fan-out cap.** `max_spawn_per_turn = 2`. Parent turn 1 emits **3**
  `spawn_subagent` calls. The 3rd is rejected; calls 1 and 2 succeed.
- **Tree-descendant cap.** `max_tree_descendants = 2`. Parent spawns 2 children
  (OK); a child (nesting allowed) tries to spawn a 3rd descendant in the same
  run-tree — rejected.

**Assertions (per sub-case)**
1. The rejected `spawn_subagent` call returns
   `CapabilityOutcome::Denied(CapabilityDenied { reason_kind, .. })` with the
   matching typed reason (`subagent_depth_exceeded` / `subagent_fanout_exceeded`
   / `subagent_tree_exceeded`).
2. **No child run** corresponding to the rejected call exists — `children_of`
   never returns it; the `TurnStateStore` has no record for it. The awaited set
   has no entry. Nothing was queued.
3. The spawning run still completes (a rejected spawn is a normal tool result,
   not a run failure).

**Test body (pseudo code, fan-out sub-case shown)**

```rust
#[tokio::test]
async fn fork_bomb_caps_reject_before_submit_turn() {
    // --- fan-out cap ---
    let h = SubagentTestHarness::start(SubagentSpawnCaps {
        max_subagent_depth: 4, max_spawn_per_turn: 2, max_tree_descendants: 16,
    });
    let t_p = h.thread_service.create_thread(/* parent */).await;
    h.script_parent_multi_spawn(&t_p, vec![
        spawn("general", "g0", true),
        spawn("general", "g1", true),
        spawn("general", "g2", true),   // 3rd — over the fan-out cap
    ]);
    h.script_profile("reborn-subagent-default",
        vec![ScriptedModelResponse::Reply { text: "ok".into() }]);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply { text: "done".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;
    let parent_state = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(parent_state.status, TurnStatus::Completed);

    let outcomes = h.spawn_call_outcomes(parent_run).await; // ordinal-ordered
    assert!(matches!(outcomes[0], CapabilityOutcome::SpawnedChildRun { .. }));
    assert!(matches!(outcomes[1], CapabilityOutcome::SpawnedChildRun { .. }));
    match &outcomes[2] {
        CapabilityOutcome::Denied(d) =>
            assert_eq!(d.reason_kind.as_str(), "subagent_fanout_exceeded"),
        other => panic!("3rd spawn must be Denied, got {other:?}"),
    }
    assert_eq!(h.children_of(parent_run).await.len(), 2); // exactly 2 — 3rd never queued

    // --- depth cap and tree-descendant cap: analogous blocks ---
}
```

> Each cap is also unit-tested at the capability-port layer in Phase 2 (the cap
> arithmetic). This E2E test is the §"Test Through the Caller" requirement from
> the root `CLAUDE.md`: it drives the cap check through `submit_turn`-adjacent
> code and proves nothing was queued.

---

### 5.7 Cancellation subtree

**Goal:** cancelling the parent recursively cancels the whole lineage subtree;
a worker-released `Blocked` parent is driven to terminal `Cancelled` via the
gate-abort path (README §7.3).

**Setup**
- Caps generous; cancellation factory wired (real
  `TurnStateRunCancellationFactory`).
- Parent spawns 2 blocking children; one child (nesting allowed) spawns a
  grandchild — a 3-level lineage. All descendants gated on `Notify` so they
  stay non-terminal.
- Parent reaches `BlockedDependentRun`.

**Assertions**
1. After `coordinator.cancel_run(parent)`, the parent reaches
   `TurnStatus::Cancelled` even though it had no claiming worker (it was
   `Blocked`) — driven via gate-abort.
2. Every descendant (`children_of` transitively over `parent_run_id`) reaches
   `TurnStatus::Cancelled`.
3. If a child happens to complete mid-cancel, its result is **discarded** —
   the parent thread gains no inbound subagent-result message after the cancel.

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn cancelling_parent_cancels_whole_lineage_subtree() {
    let h = SubagentTestHarness::start(generous_caps_with_cancellation());
    let t_p = h.thread_service.create_thread(/* parent */).await;
    let gate = Arc::new(Notify::new()); // descendants stay non-terminal

    h.script_parent_multi_spawn(&t_p, vec![
        spawn("general", "child a (nests)", false),
        spawn("general", "child b", false),
    ]);
    h.script_child_a_spawns_grandchild(Arc::clone(&gate));
    h.script_gated_children(Arc::clone(&gate));

    let parent_run = h.submit_root_turn(&t_p).await;
    h.await_status(parent_run, TurnStatus::BlockedDependentRun, Duration::from_secs(5)).await;

    let lineage = h.lineage_subtree(parent_run).await; // BFS over parent_run_id
    assert!(lineage.len() >= 3);

    h.coordinator.cancel_run(CancelRunRequest::for_run(parent_run)).await.unwrap();

    let parent_final = h.await_terminal(parent_run, Duration::from_secs(5)).await;
    assert_eq!(parent_final.status, TurnStatus::Cancelled);
    for run in &lineage {
        let s = h.await_terminal(*run, Duration::from_secs(5)).await;
        assert_eq!(s.status, TurnStatus::Cancelled, "descendant {run:?} must be cancelled");
    }

    // a child that finishes mid-cancel must not leak a result into the parent thread
    let inbound = h.thread_service.history(&t_p).await;
    assert!(inbound.iter().all(|m| !m.content.as_deref()
        .is_some_and(|c| c.contains("## Subagent result"))));
}
```

---

### 5.8 No-deadlock regression

**Goal:** the child runs on a **distinct `thread_id`** from the parent, so a
blocking spawn never self-deadlocks on the parent thread's active-run lock.
`TurnScope` active-run exclusivity is per-scope, and scope includes `thread_id`
(README §3 glossary). If a child ever shared the parent's thread, the parent
(holding the thread lock while `Blocked`) would deadlock its own child.

**Setup**
- Caps generous.
- Parent spawns one blocking child; the child is scripted to reply immediately
  (no gate).

**Assertions**
1. `child.scope.thread_id != parent.scope.thread_id` (the structural invariant).
2. `child.scope.{tenant_id,agent_id,project_id} == parent.scope.{...}` (tenancy
   invariant — the *only* field that may differ is `thread_id`).
3. The whole flow completes within a tight deadline (e.g. 3s) — a deadlock
   surfaces as a timeout.
4. Negative guard: assert the spawn capability port, given a contrived child
   scope whose `thread_id` equals the parent's, **rejects** the spawn
   (`CapabilityOutcome::Denied` with `reason_kind == "subagent_scope_invariant"`)
   — i.e. the invariant is enforced, not merely observed. (If P2.A names this
   differently, adapt.)

**Test body (pseudo code)**

```rust
#[tokio::test]
async fn child_thread_distinct_from_parent_no_deadlock() {
    let h = SubagentTestHarness::start(generous_caps());
    let t_p = h.thread_service.create_thread(/* parent */).await;

    h.script_parent_spawn(&t_p, SpawnSubagentInput {
        agent_type: "general".into(), goal: "quick".into(),
        seed: SubagentSeed::Fresh, run_in_background: false,
    });
    h.script_profile("reborn-subagent-default",
        vec![ScriptedModelResponse::Reply { text: "child quick done".into() }]);
    h.script_thread(&t_p, vec![ScriptedModelResponse::Reply { text: "parent done".into() }]);

    let parent_run = h.submit_root_turn(&t_p).await;
    // tight deadline — a self-deadlock would time out here
    let parent_final = h.await_terminal(parent_run, Duration::from_secs(3)).await;
    assert_eq!(parent_final.status, TurnStatus::Completed);

    let child = &h.children_of(parent_run).await[0];
    assert_ne!(child.scope.thread_id, parent_final.scope.thread_id);
    assert_eq!(child.scope.tenant_id, parent_final.scope.tenant_id);
    assert_eq!(child.scope.agent_id, parent_final.scope.agent_id);
    assert_eq!(child.scope.project_id, parent_final.scope.project_id);
}
```

---

## 6. Composition / wiring tests — `tests/subagent_runtime_wiring.rs`

These are cheaper, non-runner tests that pin the wiring itself.

| Test | Asserts |
|---|---|
| `family_registry_binds_default_and_subagent` | `build_loop_family_registry()` resolves both `LoopFamilyId::DEFAULT` and `"subagent"`; `ids().count() == 2`. |
| `subagent_driver_registers_with_distinct_key` | `register_subagent_planned_driver` succeeds; its `LoopDriverRegistryKey.id == "reborn:subagent-default"`, distinct from `reborn:planned-default` and `reborn:text-only-model-reply`; no `DuplicateRegistration`. |
| `subagent_profile_resolves_only_when_requested` | `default_planned_run_profile_resolver()` resolves `reborn-subagent-default` for an explicit `RunProfileRequest`; the *implicit default* still resolves to `reborn-planned-default`. |
| `subagent_profile_uses_subagent_capability_surface` | the resolved subagent profile's `capability_surface_profile_id == "subagent_tools"`, never `interactive_tools`. |
| `build_default_planned_runtime_without_subagent_parts_still_builds` | `subagent_goal_store: None`, `subagent_completion_observer: None` → `build_default_planned_runtime` succeeds (helper-test back-compat). |
| `build_product_live_runtime_fails_closed_without_goal_store` | product-live builder with `subagent_goal_store: None` → `Err(ProductLiveRuntimeBuildError::Missing(SubagentGoalStore))`. |
| `build_product_live_runtime_fails_closed_without_observer` | same with `subagent_completion_observer: None` → `Missing(SubagentCompletionObserver)`. |
| `observer_registered_as_event_sink` | after `build_subagent_runtime`, submit a turn → cancel it → assert the observer received the lifecycle event (observe via a spy wrapper or a `children_of`-side-effect probe). |
| `subagent_runtime_passes_production_readiness` | `validate_reborn_loop_production_readiness` with the subagent profile selected, `RebornLoopComponentGraphReadiness::production_verified()` extended with `production_verified` subagent components → `ProductionReady`, `issues.is_empty()`. |
| `subagent_runtime_not_ready_with_test_only_goal_store` | same but `subagent_goal_store` component marked `test_only` → `NotReady` with `SubagentGoalStore` + `TestOnlyImplementation`. |

---

## 7. `production_readiness.rs` additions

The subagent family adds two production-relevant components — the durable goal
store and the completion observer. Both must be subject to the same safety-class
gate as `checkpoint_state_store` and `wake_notifier`.

```rust
// crates/ironclaw_reborn/src/production_readiness.rs  (additions)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornLoopProductionComponent {
    // ... existing 14 variants ...
    SubagentGoalStore,
    SubagentCompletionObserver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornLoopComponentGraphReadiness {
    // ... existing 11 fields ...
    pub subagent_goal_store: RebornComponentReadiness,
    pub subagent_completion_observer: RebornComponentReadiness,
}
```

- `production_verified()` sets both new fields to
  `RebornComponentReadiness::production_verified(Required)`.
- `components()` yields the two new `(component, readiness)` pairs.
- `component_subject()` maps them to `"subagent_goal_store"` /
  `"subagent_completion_observer"`.
- The `host_graph_for` mapping is **not** extended — the goal store and
  observer are not host-graph ports the driver requires; the `subagent`
  `PlannedDriver` requires only the standard seven (`planned_driver_requirements()`
  — model/prompt/transcript/checkpoint/input/capabilities/progress). They are
  validated purely through `push_component_issues`.

`subagent_driver_requirements()` is added as a convenience equal to
`tool_capable_driver_requirements()` (the subagent driver needs a real
capability port — it must surface `spawn_subagent` and the flavor tools):

```rust
pub fn subagent_driver_requirements() -> DriverRequirements {
    tool_capable_driver_requirements()
}
```

Reasoning for **why the new family/driver must satisfy
`production_readiness.rs`** (§9 risk):

1. The `subagent` `PlannedDriver` is registered as `DriverKind::Production`. If
   it were `Reference`, `validate_entry_readiness` would push
   `ReferenceDriverNotProductionReady` and block startup. It is a real driver
   over a real family → `Production` is correct.
2. The subagent **run profile** must appear in the configured-profiles list fed
   to `validate_reborn_loop_production_readiness`. Its
   `driver_identity` (`LoopDriverRegistryKey`) must match the registered
   subagent driver's key, and its `checkpoint_schema_id` /
   `checkpoint_schema_version` must match the descriptor — otherwise
   `push_profile_identity_issues` raises `CheckpointSchema / VersionMismatch`.
   Because the subagent driver reuses the canonical `CHECKPOINT_SCHEMA_ID` /
   `CHECKPOINT_SCHEMA_VERSION`, this holds by construction (same as the default
   planned driver).
3. The subagent driver's `DriverRequirements` is `subagent_driver_requirements()`
   = all seven `Required`. `missing_requirements` therefore demands a present,
   production-verified `capability_port` in the host graph — which the
   spawn-capable port satisfies. A composition that wired the subagent profile
   but left `capability_port` missing would (correctly) fail readiness.
4. The two new components (`SubagentGoalStore`, `SubagentCompletionObserver`)
   fail closed in `Production` mode unless `ProductionVerified`. The bounded
   in-process goal store is **durable enough for restart** only if it persists
   across process restart; the README §13 explicitly defers a "durable
   goal-store backend beyond the bounded in-process store". **Therefore v1 marks
   the goal store `RebornComponentSafetyClass::NonDurable`** until P1.C's store
   is genuinely restart-durable — which makes a strict `Production` readiness
   check **fail**. This is intentional and is the headline risk in §9.

---

## 8. Quality gate

Run from the workspace root. All three must pass with zero warnings / zero
failures before the Phase 3 PR is mergeable.

```bash
cargo fmt --all -- --check
cargo clippy --all --benches --tests --examples --all-features    # zero warnings
cargo test -p ironclaw_reborn                                     # crate unit + integration
cargo test -p ironclaw_turns -p ironclaw_agent_loop -p ironclaw_loop_support
cargo test                                                        # full workspace
```

What must pass specifically:

- **`cargo fmt`** — clean. All new files (`subagent_runtime.rs`, the two test
  files) and all edits formatted.
- **`cargo clippy`** — zero warnings. Watch for unused `Arc::clone`s in the
  wiring and for exhaustive matches on the new contract variants from README
  §10 (`CapabilityOutcome`, `LoopGateKind`, `LoopBlockedKind`, `BlockedReason`,
  `TurnStatus`). `CapabilityOutcome`, `BlockedReason`, and `TurnStatus` remain
  deliberately exhaustive; do not paper over them with catch-all arms.
- **`cargo test -p ironclaw_reborn`** — the eight E2E tests (§5), the ten
  wiring tests (§6), and the existing `production_readiness.rs` /
  `planned_driver_e2e.rs` / `driver_registry.rs` tests still pass. The
  pre-existing `production_registry_binds_default_family_only` test is *renamed*
  and updated (it asserted `ids().count() == 1`).
- **Full `cargo test`** — the new `TurnStatus::BlockedDependentRun` persisted
  enum variant has a legacy-value deserialization test (README §10); the
  raw-JSON round-trip tests for the four other wire-stable enums pass; no
  in-workspace match arm over those enums is left non-exhaustive.
- **`cargo test --features integration`** — if any PostgreSQL-backed turn store
  test exists for the new `parent_run_id` / `subagent_depth` columns and
  `children_of` query (Phase 1 P1.A territory), it passes here.

---

## 9. Wiring checklist (ordered)

Execute strictly top-to-bottom — each step depends on the prior. Steps marked
**[P2]** verify a Phase 2 deliverable is present before Phase 3 can proceed.

1. **[P2]** Confirm `ironclaw_agent_loop::families::subagent()` exists and
   `LoopFamilyId::new("subagent")` validates. *(P1.B / P2.C)*
2. **[P2]** Confirm the Phase 1 contract additions are merged:
   `CapabilityOutcome::{SpawnedChildRun, AwaitDependentRun}`,
   `LoopGateKind::AwaitDependentRun`, `LoopBlockedKind::AwaitDependentRun`,
   `BlockedReason::DependentRun`, `TurnStatus::BlockedDependentRun`,
   `SubmitTurnRequest` / `TurnRunRecord` lineage fields,
   `TurnStateStore::{children_of, get_run_record}`. *(P1.A, P1.B)*
3. **[P2]** Confirm `DefaultTurnCoordinator::with_event_sink` exists (or get it
   added to P1.A — it is a coordination contract, not Reborn composition).
4. **[P2]** Confirm the spawn-capable capability port + factory and the
   `SPAWN_SUBAGENT_CAPABILITY_ID` constant exist. *(P2.A)*
5. **[P2]** Confirm `BoundedSubagentGoalStore` + `SubagentGoalStore` trait +
   `SubagentFlavorTable` + direction `.md` files exist. *(P1.C)*
6. **[P2]** Confirm `SubagentCompletionObserver` implements `TurnEventSink` and
   exposes `bind_coordinator`. *(P2.D)*
7. `app_loop_family.rs`: register `families::subagent()` (§3.1); rename + update
   the family-count test.
8. `planned_driver_factory.rs`: add `subagent_planned_driver*`,
   `register_subagent_planned_driver`, `subagent_planned_profile_definition`;
   fold the subagent profile into `default_planned_run_profile_resolver` (§3.2).
9. `production_readiness.rs`: add the two `RebornLoopProductionComponent`
   variants, the two `RebornLoopComponentGraphReadiness` fields, the
   `components()` / `component_subject()` arms, `subagent_driver_requirements()`
   (§7).
10. `runtime.rs`: add the two `subagent_*` `Option` fields to
    `DefaultPlannedRuntimeParts` (§3.4); register the subagent driver in
    `build_default_planned_runtime` (§3.3); register the observer as the
    coordinator's event sink (§3.5); add the two
    `ProductLiveRuntimeReadinessComponent` variants + fail-closed checks (§3.6).
11. `subagent_runtime.rs` (new): `SubagentRuntimeParts`, `SubagentSpawnCaps`,
    `build_subagent_runtime`, `build_product_live_subagent_runtime` (§3.7).
12. `lib.rs`: `pub mod subagent_runtime;` (and any other new modules P1.C/P2.D
    introduced if they live in `ironclaw_reborn`).
13. `tests/subagent_runtime_wiring.rs`: the ten composition tests (§6).
14. `tests/subagent_spawn_e2e.rs`: the `SubagentTestHarness` (§5.0) + the eight
    E2E tests (§5.1–5.8).
15. Run the quality gate (§8) until green.

**Dependency on every Phase 2 workstream — explicit:** Phase 3 cannot begin
until **all four** Phase 2 workstreams (P2.A spawn handling, P2.B prompt
composition, P2.C subagent driver, P2.D completion observer) are merged.
Steps 1, 4–6, 8, 11 each consume a different one; there is no partial-Phase-2
start. P2.B (prompt composition) has no *direct* Phase-3 wiring step — it is
internal to the capability/context ports — but §5.1 and §5.2 *assert* its
effect (the child sees the goal as a user message), so a P2.B regression fails
Phase 3 tests.

---

## 10. Risks

### 10.1 Production-readiness: the goal store is not restart-durable (HIGH)

The README (§13) explicitly defers "a durable goal-store backend beyond the
bounded in-process store". §9 of the README also mandates: "Process restart:
Goal store is durable ... A goal-store miss fails the child loudly." These are
in tension. The v1 `BoundedSubagentGoalStore` is in-process; it does **not**
survive a restart.

Consequence for `production_readiness.rs`: the `SubagentGoalStore` component
must be classified `RebornComponentSafetyClass::NonDurable` (not
`ProductionVerified`). In `RebornLoopReadinessMode::Production`,
`push_component_issues` will then emit a **blocking**
`NonDurableImplementation` issue → `RebornLoopProductionStatus::NotReady`.

**Mitigation / decision required:** either (a) ship subagent spawn as a
`LocalDevTest`-degraded feature in v1 (readiness reports it as a warning, not a
block), gated behind an explicit config flag, until a durable backend lands; or
(b) make the bounded store write-through to the durable turn-state DB so it is
genuinely `ProductionVerified`. The Phase 3 PR must pick one and the
`subagent_runtime_passes_production_readiness` test (§6) encodes the choice. If
(a): the test asserts `LocalDevDegraded`, not `ProductionReady`. **This is the
single most important open decision for Phase 3 and must be resolved with the
reviewer before merge.**

### 10.2 `with_event_sink` may not exist on `DefaultTurnCoordinator` (MEDIUM)

`DefaultTurnCoordinator` today has `with_admission_policy`,
`with_run_profile_resolver`, `with_wake_notifier` — **no event sink setter**,
and `TurnEventSink` lives in `ironclaw_turns::events` but the coordinator does
not consume it. If P1.A did not add `with_event_sink` + the publish call sites
inside `submit_turn` / `resume_turn` / `cancel_run` transition handling, the
observer will never receive lifecycle events and every §5 test hangs to
timeout.

**Mitigation:** wiring checklist step 3 makes this a hard Phase-2 gate. If P1.A
scoped the event-sink plumbing out, it is pulled forward — it is a coordination
contract (`ironclaw_turns`), so it belongs in P1.A, never in Phase 3's
`ironclaw_reborn` crate.

### 10.3 Wire-stable enum variants must be added atomically (MEDIUM)

README §10 lists five enums gaining variants. `clippy`/`rustc` exhaustive-match
checks will fail the build if any in-workspace `match` over
`CapabilityOutcome` / `LoopBlockedKind` / `BlockedReason` / `TurnStatus` /
`LoopGateKind` is not updated. The capability port's `is_suspension()`,
`runtime_outcome_to_loop`, the loop-exit applier's `LoopBlockedKind` mapping,
and `production_readiness.rs`'s `TurnStatus` uses are the known call sites.

**Mitigation:** these arms are Phase 1/2's to add, but Phase 3's full
`cargo clippy --all` + `cargo test` (§8) is the catch-net. If Phase 3 finds an
un-updated arm, it is a missed Phase 1/2 deliverable — fix in the owning crate,
not by a local `_ =>` wildcard (a wildcard would silently swallow the new
variant and is forbidden).

### 10.4 Driver-registry key collision (LOW)

`DriverRegistry::register_driver` rejects duplicate `LoopDriverRegistryKey`s.
The subagent driver reuses the canonical `CHECKPOINT_SCHEMA_ID` and
`CHECKPOINT_SCHEMA_VERSION`, so the key differs from `reborn:planned-default`
**only** on `LoopDriverId`. As long as `SUBAGENT_DRIVER_ID` is literally
distinct (`"reborn:subagent-default"` ≠ `"reborn:planned-default"`), no
collision. The `subagent_driver_registers_with_distinct_key` test (§6) pins
this.

### 10.5 Runner-pool starvation under blocking spawn (LOW–MEDIUM)

A blocking parent releases its worker on `Blocked` (README §7.2), so it does not
hold a worker slot while waiting. But the §5.3 parallel test submits N children
that each need a worker slot. With a 1-slot `TurnRunnerWorkerConfig` the
children serialise — correct, but the test's `await_terminal` deadlines must be
generous enough to absorb serialisation. The harness (§5.0) uses a small
multi-slot pool for the parallel tests and a 1-slot pool only for the
early-completion test (§5.4, where serialisation makes the race deterministic).

**Mitigation:** harness exposes pool size as a `SubagentTestHarness::start`
parameter; each test picks the size its assertions need. No production risk —
this is purely a test-determinism concern.

### 10.6 Observer event ordering vs. coordinator transition durability (MEDIUM)

The observer reacts to a child's terminal `TurnLifecycleEvent`. For a blocking
parent it must `resume_turn(parent)`; for background it must
`accept_inbound_message` + coalescing `submit_turn`. If the event fires *before*
the child's terminal transition is durably committed to the `TurnStateStore`,
the observer's `children_of` reconciliation could see a stale non-terminal
child and never resume the parent — a lost wakeup.

The README §9 mitigation (awaited set recorded *before* `submit_turn`; gate-entry
reconciliation against `get_run_state`) covers the *executor* side. The
*observer* side relies on the coordinator publishing the lifecycle event only
**after** the transition is committed. This is a P1.A coordinator-contract
guarantee; §5.2 and §5.4 are the Phase-3 catch-net (a violation surfaces as a
parent that never leaves `BlockedDependentRun` → test timeout).

**Mitigation:** if §5.2 flakes, the root cause is event-before-commit ordering
in P1.A's coordinator publish path — fix there. Phase 3 must not paper over it
with a sleep in the observer.
