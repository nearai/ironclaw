# WS-14 — `PlannedDriver` Registration + Run-Profile Selection

**Workstream:** WS-14 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_reborn` + `ironclaw_turns`
**Depends on:** WS-7 (`PlannedDriver` adapter), WS-8 (skeleton green).
**Coordinated after adapter contracts from:** WS-9
([capability-host-wiring](capability-host-wiring.md)), WS-10
([checkpoint-store-and-resume](checkpoint-store-and-resume.md)),
WS-11 ([loop-input-port](loop-input-port.md)), WS-12
([loop-progress-port](loop-progress-port.md)), WS-13
([host-cancellation-accessor](host-cancellation-accessor.md)), and
WS-15 ([prompt-context-assembly](prompt-context-assembly.md)).
**Feeds:** WS-16 ([live-runtime-wiring](live-runtime-wiring.md)) and
WS-17 ([product-live-cutover](product-live-cutover.md)).
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

After the skeleton (WS-0..WS-8) lands, `PlannedDriver` exists but is
unreachable from a real submitted turn — `TurnRunner` looks up
drivers in `DriverRegistry`
([`crates/ironclaw_reborn/src/driver_registry.rs:164`](../../../crates/ironclaw_reborn/src/driver_registry.rs))
by the `(driver_id, version, checkpoint_schema_id, schema_version)`
key, and only `TextOnlyModelReplyDriver` is registered there today
([`crates/ironclaw_reborn/src/text_loop_driver.rs:20`](../../../crates/ironclaw_reborn/src/text_loop_driver.rs)).
Profiles emitted by `InMemoryRunProfileResolver`
([`crates/ironclaw_turns/src/run_profile/resolver.rs:115`](../../../crates/ironclaw_turns/src/run_profile/resolver.rs))
all point at `reborn:text-only-model-reply`.

WS-14 closes the loop:

1. **Stable `LoopDriverId` constants** for the default planned driver
   and the v1 checkpoint schema id reserved by WS-0.
2. **Factory** `default_planned_driver()` that composes a
   non-generic `PlannedDriver` (holding `families::default()` resolved
   via `Arc<LoopFamilyRegistry>` plus `Arc<CanonicalAgentLoopExecutor>`)
   from a fully-wired `PlannedDriverConfig`.
3. **Registration helper** `register_default_planned_driver(registry,
   config)` that calls `DriverRegistry::register_driver` with the
   right `DriverRequirements` flags.
4. **Profile resolver entry** — a new builtin profile
   `reborn-planned-default` that resolves to the new driver, with a
   sensible default capability-surface profile id.
5. **Implicit-default selector support** — expose the resolver hook or
   constructor that lets a runtime composition choose
   `reborn-planned-default` as the no-profile default. WS-16 consumes
   that hook in the Reborn runtime; WS-17 consumes the WS-16
   composition from the product path.
6. **Coexistence** — `TextOnlyModelReplyDriver` stays registered for
   explicit legacy/profile-selected runs.

This is not the live default end-to-end cutover. WS-14 makes the
planned driver and planned profile available. WS-16 proves the Reborn
runtime can compose them with real adapters. WS-17 wires the normal
product no-profile path to that composition.

## 2. Files

### NEW
- `crates/ironclaw_reborn/src/planned_driver_factory.rs` —
  `default_planned_driver()`, `register_default_planned_driver()`,
  driver-id / checkpoint-schema-id constants.
- `crates/ironclaw_reborn/tests/planned_driver_e2e.rs` — the smoke
  tests in §5.

### MODIFIED
- `crates/ironclaw_reborn/src/lib.rs` — module declaration +
  re-export of the constants.
- `crates/ironclaw_reborn/src/planned_driver.rs` (WS-7 file) — no
  type alias needed; `PlannedDriver` is non-generic. The factory
  returns `PlannedDriver` directly.
- Wherever `InMemoryRunProfileResolver` seeds its builtin profiles
  (per the resolver layout near
  [`resolver.rs:115`](../../../crates/ironclaw_turns/src/run_profile/resolver.rs))
  — registration of the `reborn-planned-default` profile happens in
  `ironclaw_reborn` (not `ironclaw_turns`), via a `register_default_profiles(resolver)`
  helper that lives next to the registration helper above.

### MODIFIED (additive trait extension)
- `crates/ironclaw_turns/src/run_profile/resolver.rs` —
  `InMemoryRunProfileRegistry` gains a public mutator
  `pub fn register(&mut self, definition: RunProfileDefinition) -> Result<(), …>`.
  Today the registry's `with_builtin_profiles()` constructor at
  [`resolver.rs:115`](../../../crates/ironclaw_turns/src/run_profile/resolver.rs)
  is the only way to populate it, and the field is private. The
  follow-up cannot register a new builtin from `ironclaw_reborn`
  without this small contract addition. Strictly additive; existing
  `default()` constructor stays unchanged.
- `crates/ironclaw_turns/src/run_profile/resolver.rs` —
  the resolver gains an explicit implicit-default selector, e.g.
  `InMemoryRunProfileResolver::new_with_implicit_default(registry,
  RunProfileId)` or an equivalent builder method. The no-profile
  branch MUST read this selector instead of hardcoding
  `interactive_default`.

### NOT TOUCHED
- `crates/ironclaw_turns/src/run_profile/host.rs`,
  `driver.rs`, `refs.rs` — the contract surface for the trait /
  descriptor / id types is unchanged.
- `crates/ironclaw_reborn/src/text_loop_driver.rs` — TextOnly stays
  registered as-is. The constants at lines 20–56 are read-only for
  this brief.

## 3. Specification

### 3.1 Driver-id and checkpoint-schema constants

```rust
//! crates/ironclaw_reborn/src/planned_driver_factory.rs

use ironclaw_turns::run_profile::{CheckpointSchemaId, LoopDriverId, RunProfileVersion};

/// Stable id for the default planned driver.
///
/// Format follows the existing convention at
/// `crates/ironclaw_reborn/src/text_loop_driver.rs:20` —
/// `"reborn:"` prefix, lowercase + dashes. Persisted in
/// `LoopDriverRegistryKey`; do NOT rename without a migration.
pub const PLANNED_DRIVER_DEFAULT_ID: &str = "reborn:planned-default";

pub const PLANNED_DRIVER_DEFAULT_VERSION: RunProfileVersion = RunProfileVersion::new(1);

/// Reserved by WS-0 (`agent-loop-briefs/state-and-checkpoints.md:127`).
/// `LoopExecutionState::from_checkpoint_payload` decodes only this
/// schema id today.
pub const PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v1";

pub const PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION: RunProfileVersion = RunProfileVersion::new(1);

pub fn planned_driver_default_id() -> LoopDriverId {
    // newtype constructor validates the static literal
    LoopDriverId::from_trusted_static(PLANNED_DRIVER_DEFAULT_ID)
}

pub fn planned_driver_checkpoint_schema_id() -> CheckpointSchemaId {
    CheckpointSchemaId::from_trusted_static(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
}
```

`LoopDriverId` and `CheckpointSchemaId` are newtypes generated by the
`profile_ref!` macro at
[`crates/ironclaw_turns/src/run_profile/refs.rs:54`](../../../crates/ironclaw_turns/src/run_profile/refs.rs).
`from_trusted_static` is the existing legacy constructor used by
TextOnly at `text_loop_driver.rs:51`; per `.claude/rules/types.md`
"Legacy exception" note, it is acceptable for these existing
identity types.

### 3.2 Factory `default_planned_driver`

```rust
//! crates/ironclaw_reborn/src/planned_driver_factory.rs

use std::sync::Arc;
use ironclaw_agent_loop::{
    canonical_executor::CanonicalAgentLoopExecutor,
    default_planner::DefaultPlanner,
};
use ironclaw_turns::run_profile::{
    AgentLoopDriverDescriptor, CheckpointSchemaIdOptional, RunProfileVersionOptional,
};
use crate::planned_driver::{PlannedDriver, PlannedDriverConfig};
use ironclaw_agent_loop::family::{LoopFamilyId, LoopFamilyRegistry};

pub struct DefaultPlannedDriverBuild {
    pub driver: Arc<dyn ironclaw_turns::run_profile::AgentLoopDriver>,
    pub descriptor: AgentLoopDriverDescriptor,
}

pub fn default_planned_driver(
    config: PlannedDriverConfig,
    family_registry: Arc<LoopFamilyRegistry>,
) -> DefaultPlannedDriverBuild {
    let family = family_registry
        .get(&LoopFamilyId::DEFAULT)
        .expect("LoopFamilyRegistry::builtin always binds DEFAULT");
    let executor = Arc::new(CanonicalAgentLoopExecutor::default());
    let driver = PlannedDriver::from_family(
        planned_driver_default_id(),
        family,
        executor,
        PLANNED_DRIVER_DEFAULT_VERSION,
    )
        .expect("default family + framework checkpoint schema validate");

    let descriptor = AgentLoopDriverDescriptor::from_trusted_static_with_checkpoint(
        planned_driver_default_id(),
        PLANNED_DRIVER_DEFAULT_VERSION,
        Some(planned_driver_checkpoint_schema_id()),
        Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION),
    );

    DefaultPlannedDriverBuild {
        driver: Arc::new(driver) as Arc<dyn _>,
        descriptor,
    }
}
```

`AgentLoopDriverDescriptor::from_trusted_static_with_checkpoint` is
the four-arg variant of the existing `from_trusted_static` at
`crates/ironclaw_turns/src/run_profile/driver.rs:13`. The brief
spells out the constructor in case the actual codebase uses a
builder shape — the implementer matches the API at PR time.

### 3.3 Registration helper

```rust
//! crates/ironclaw_reborn/src/planned_driver_factory.rs

use ironclaw_reborn::driver_registry::{
    DriverKind, DriverRegistry, DriverRegistryError, DriverRequirements,
    LoopDriverRegistryKey,
};

pub fn register_default_planned_driver(
    registry: &mut DriverRegistry,
    config: PlannedDriverConfig,
) -> Result<LoopDriverRegistryKey, DriverRegistryError> {
    let build = default_planned_driver(config);
    // DriverRequirements uses RequirementLevel — Required / Optional /
    // Unsupported — not booleans. See
    // `crates/ironclaw_reborn/src/driver_registry.rs:56,64`.
    // PlannedDriver consults every port the canonical tick touches,
    // so every dimension is Required.
    //
    // Note: there is no `cancellation` dimension on DriverRequirements
    // today. WS-13's cancellation observation is wired below the
    // registry-readiness contract — `RebornLoopDriverHost` always
    // exposes the `LoopCancellationPort` once WS-13 lands, so the
    // driver simply assumes it. If the existing `DriverRequirements`
    // surface ever grows a `cancellation` field, this constructor
    // gains a matching `RequirementLevel::Required` line in a strict
    // follow-up.
    let requirements = DriverRequirements {
        model: RequirementLevel::Required,
        prompt: RequirementLevel::Required,
        transcript: RequirementLevel::Required,
        checkpoint: RequirementLevel::Required,
        input_polling: RequirementLevel::Required,
        capabilities: RequirementLevel::Required,
        progress_events: RequirementLevel::Required,
    };
    registry.register_driver(build.driver, requirements, DriverKind::Production)
}
```

Each dimension is explicit `RequirementLevel::Required` rather than
the equivalent `DriverRequirements::all_required()` helper at
[`driver_registry.rs:87`](../../../crates/ironclaw_reborn/src/driver_registry.rs)
to keep the rationale visible at the call site. Future tightening
(e.g. `progress_events: RequirementLevel::Optional` once we accept
that drop-on-fail is acceptable) becomes a one-line change with
clear blame.

### 3.4 Profile registry entry

```rust
//! crates/ironclaw_reborn/src/planned_driver_factory.rs

use ironclaw_turns::run_profile::{
    CapabilitySurfaceProfileId, InMemoryRunProfileRegistry, ModelProfileId,
    RunProfileDefinition, RunProfileId,
};

pub const PLANNED_DEFAULT_PROFILE_ID: &str = "reborn-planned-default";

pub fn register_default_planned_profile(
    registry: &mut InMemoryRunProfileRegistry,
) -> Result<(), RunProfileRegistryError> {
    // The builder/constructor shape for RunProfileDefinition matches
    // whatever the helpers `interactive_profile()` / `long_running_mission_profile()`
    // at the bottom of `crates/ironclaw_turns/src/run_profile/resolver.rs` use.
    // The implementer mirrors the existing builtin's construction.
    let definition = RunProfileDefinition::builder()
        .profile_id(RunProfileId::from_trusted_static(PLANNED_DEFAULT_PROFILE_ID))
        .profile_version(PLANNED_DRIVER_DEFAULT_VERSION)
        .loop_driver(AgentLoopDriverDescriptor::from_trusted_static_with_checkpoint(
            planned_driver_default_id(),
            PLANNED_DRIVER_DEFAULT_VERSION,
            Some(planned_driver_checkpoint_schema_id()),
            Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION),
        ))
        .checkpoint_schema_id(planned_driver_checkpoint_schema_id())
        .checkpoint_schema_version(PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION)
        // Match the surface profile id used by TextOnly's builtin
        // (interactive_profile) so the first E2E run is apples-to-apples.
        // WS-9's CapabilitySurfaceProfileResolver decides the actual surface.
        .capability_surface_profile_id(default_text_capability_surface())
        .model_profile_id(default_text_model_profile())
        // Steering/cancellation/checkpoint/budget policies and
        // required_privileges all match interactive_profile()'s
        // defaults; copy them at PR time.
        .build()?;
    // `register` is the new public mutator added to
    // InMemoryRunProfileRegistry in `ironclaw_turns` per §2 MODIFIED.
    registry.register(definition)
}
```

`InMemoryRunProfileResolver` is constructed from a registry plus an
explicit implicit-default profile id; WS-14 adds that construction
path, and WS-16 decides where the planned default is used in runtime
composition.

### 3.5 Coexistence with `TextOnlyModelReplyDriver`

The TextOnly driver stays registered. Three resolution cases co-exist:

- **Implicit no-profile request** — can resolve to
  `reborn-planned-default` when a runtime composition chooses the
  planned profile as its implicit default. WS-16 does this for the
  Reborn runtime smoke; WS-17 does this for the normal product
  channel/gateway submission path.
- `interactive_default` (the existing builtin at
  [`crates/ironclaw_turns/src/run_profile/resolver.rs:210`](../../../crates/ironclaw_turns/src/run_profile/resolver.rs))
  → `reborn:text-only-model-reply` v1, no checkpoint schema. It stays
  available when explicitly requested by profile id; it is no longer
  the implicit fallback inside the WS-16 planned runtime composition.
- `reborn-planned-default` (this brief) → `reborn:planned-default` v1,
  `reborn:default-loop-v1` v1.

`LoopDriverRegistryKey` includes the checkpoint schema id+version,
so the two keys are distinct even though both pin driver version 1.
No collision risk.

Migration policy is split:

- **Runtime planned default:** the resolver's no-profile branch can be
  constructed with `reborn-planned-default`. WS-16 owns the runtime
  composition that chooses it; WS-17 owns the product cutover.
- **Named profiles:** callers that explicitly request another profile
  still get that profile. No automated shimming or environment-flag
  switch rewrites explicit requests.

### 3.6 Runtime wiring handoff

The brief documents the composition call sequence that WS-16 consumes
so a reviewer can trace the bring-up:

```rust
// inside the WS-16 Reborn runtime composition helper

// Driver registry — register both drivers.
let mut driver_registry = DriverRegistry::default();
register_default_text_only_driver(&mut driver_registry, text_only_config)?;  // existing
register_default_planned_driver(&mut driver_registry, planned_driver_config)?; // NEW

// Profile registry — start with the existing builtins, then add ours.
let mut profile_registry = InMemoryRunProfileRegistry::with_builtin_profiles();
register_default_planned_profile(&mut profile_registry)?;                     // NEW
let resolver = InMemoryRunProfileResolver::new_with_implicit_default(          // NEW
    profile_registry,
    RunProfileId::from_trusted_static(PLANNED_DEFAULT_PROFILE_ID),
);

let coordinator = TurnCoordinator::new(/* … */).with_resolver(resolver);
let runner = TurnRunner::new(/* … */).with_registry(driver_registry);
```

`planned_driver_config: PlannedDriverConfig` is composed by WS-16 from
the follow-up adapters:

- WS-9: `capability_host`, `surface_resolver`
- WS-10: `checkpoint_state_store`, `checkpoint_metadata_store`
- WS-11: `input_queue`
- WS-12: `event_sink: Some(...)`, `progress_port`
- WS-13: `cancellation_factory`
- WS-15: `identity_source`, `identity_budget`

WS-14 provides the registration and resolver helpers. WS-16 is the
merge gate for proving those helpers work with real adapters.
`identity_source = None` is allowed only for helper-level WS-14 tests;
it is not valid for the WS-16 runtime smoke or WS-17 product cutover.

## 4. Composition diagram

```text
                 TurnCoordinator
                       │
                       ▼
                   TurnRunner ── DriverRegistry
                       │              │
                       ▼              ▼
              ResolvedRunProfile   LoopDriverRegistryKey lookup
                       │              │
                       ▼              ▼
                  RunProfileResolver   ┌─────────────────────────┐
                       │               │   PlannedDriver         │
                       └──────────────►│   (DefaultPlanner +     │
                                       │    CanonicalExecutor)   │
                                       └────────────┬────────────┘
                                                    ▼
                                    AgentLoopDriverHost facade
                                       │
                  ┌────────────────────┼──────────────────────────┐
                  ▼                    ▼                          ▼
       HostRuntimeLoopCapabilityPort  HostManagedLoopProgressPort   RunStateLoopCancellationPort
       + CapabilitySurfaceProfileFilter   (extended match, WS-12)            (WS-13)
                  (WS-9)
                  ▼
            HostQueueLoopInputPort
                  (WS-11)
                  ▼
       HostManagedLoopCheckpointPort
         (extended with load_checkpoint_payload, WS-10)
                  ▼
       ThreadBackedLoopContextPort + HostManagedLoopPromptPort
         (extended with WorkspaceIdentityContextSource, WS-15)
                  ▼
             (model/transcript ports stay host-managed)
```

## 5. Verification

Unit tests (in `crates/ironclaw_reborn`):

- `planned_driver_factory::tests::register_default_planned_driver_uses_v1_schema`
  — assert `LoopDriverRegistryKey` matches the four constants in
  §3.1.
- `planned_driver_factory::tests::descriptor_carries_checkpoint_schema`
  — assert
  `descriptor.checkpoint_schema_id() == Some("reborn:default-loop-v1")`.
- `planned_driver_factory::tests::key_collision_with_textonly_is_impossible`
  — register both drivers; assert both `LoopDriverRegistryKey`s are
  distinct.
- `planned_driver_factory::tests::profile_resolves_to_planned_driver`
  — register the profile; resolve `RunProfileId("reborn-planned-default")`;
  assert resolved `driver_id == "reborn:planned-default"` v1.
- `planned_driver_factory::tests::implicit_default_resolves_to_planned_driver`
  — construct the resolver with no requested profile and assert the
  resolved profile id is `reborn-planned-default`, not
  `interactive_default`.
- `planned_driver_factory::tests::explicit_text_only_profile_still_resolves_textonly`
  — explicitly resolve `interactive_default` and assert it still maps
  to `reborn:text-only-model-reply`.

Integration tests (in `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`):

- `planned_driver_profile_smoke` — registers both drivers and the
  planned profile, resolves `reborn-planned-default`, invokes
  `PlannedDriver` through `TurnRunner`, and asserts a completed loop
  with a reply ref. This may use in-memory ports and local test
  fixtures; it does not claim product live readiness.
- `planned_driver_no_profile_uses_configured_implicit_default` —
  constructs the resolver with `reborn-planned-default` as the
  implicit default and proves a no-profile request selects the planned
  driver.
- `planned_driver_text_only_profile_still_resolves_textonly` —
  regression guard: registering both drivers does not affect the
  TextOnly profile's explicit resolution.

The real-host no-profile smoke moves to WS-16. The product-facing live
cutover test moves to WS-17.

## 6. Out of scope (for this brief)

- **Rewriting explicit named profiles** to the new driver. Each named
  profile still picks its driver by name; operators decide when to
  flip those ids. The implicit no-profile default is in scope above.
- **Driver kill-switch / rollout flags.** Registry membership is
  the toggle; deregister the planned driver to disable. No additional
  flag plumbing.
- **Loop-family planners** (`coding`, `routine`, …) — master doc
  §11/§12 / §12.5 explicitly defers; WS-14 ships `families::default()`
  only, resolved via the `LoopFamilyRegistry` (WS-3.5).
- **Versioning the family separately from the driver.** `LoopFamily.version`
  (`ComponentIdentity`) lands in the checkpoint payload metadata (WS-0
  §3.2), not in the registry key. A new family shape (or a strategy
  composition change inside an existing family) bumps `ComponentIdentity.digest`
  and may ship under a new driver id.
- **Schema migration from `reborn:default-loop-v1` → `v2`.** Strict
  schema match per WS-10 §6; v2 is a future PR.
- **Claiming runtime or product cutover without WS-15.** A WS-14
  helper smoke may use `identity_source = None`; the WS-16 runtime
  smoke and WS-17 product cutover may not.
- **Tracing / OpenTelemetry beyond `RuntimeEvent`.** WS-12 ships the
  loop progress surface; metric exporters are sink-side concerns.
