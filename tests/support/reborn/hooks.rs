//! E-HOOK-INFRA: recording hook doubles + per-run `HookDispatcherBuilderFactory`
//! builders, so C-HOOKS can observe hook dispatch on a coordinator-path turn.
//!
//! Wired into a harness/group via `with_hook_factory` /
//! `RebornIntegrationGroupBuilder::hook_dispatcher_builder_factory`. The factory
//! mints a fresh [`HookDispatcherBuilder`] per host build (per-run isolation),
//! installing recording hooks that write every fire into a shared
//! [`RecordingHookLog`]. A test reads that log back after the turn to prove the
//! wired factory actually fires hooks at the expected lifecycle points.
//!
//! These are hand-built first-party hooks (like `ironclaw_reborn`'s
//! `first_party_only_hook_factory` host-plumbing double), NOT composition
//! activation coverage: the production `build_hook_dispatcher_builder_factory`
//! ships an EMPTY first-party catalog, and its activation/projection path is
//! covered at crate tier in `ironclaw_reborn_composition::hooks::tests`. The
//! int-tier gap this fills is the end-to-end turn wire — that a wired
//! `hook_dispatcher_builder_factory` dispatches hooks through the real
//! coordinator → loop → host path — without re-authoring that crate-tier
//! activation coverage.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module, so its symbols read as dead there
// under the all-features `-D warnings` lane.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::identity::{HookId, HookVersion};
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::{BeforeCapabilityHookContext, ObserverHookContext};
use ironclaw_hooks::registry::{HookPointSpec, HookRegistry};
use ironclaw_hooks::sink::{
    ObserverHook, ObserverSink, PrivilegedBeforeCapabilityHook, PrivilegedGateSink,
};
use ironclaw_reborn::loop_driver_host::{HookDispatcherBuilderFactory, RebornLoopDriverHostError};

/// Canonical identity paths for the TEST-ONLY recording hooks. Kept distinct so
/// both a `BeforeCapability` gate hook and an `AfterModel` observer can coexist
/// in one dispatcher.
const RECORDING_OBSERVER_PATH: &str = "tests::reborn::hooks::RecordingObserverHook";
const RECORDING_BEFORE_CAPABILITY_PATH: &str =
    "tests::reborn::hooks::RecordingBeforeCapabilityHook";
/// The `&'static` reason a `DenyBeforeCapabilityHook` mints (gate-sink reasons
/// must be `&'static str`, so no `format!`-built string can leak through).
pub const HOOK_TEST_DENY_REASON: &str = "hook_test_deny";

/// Shared, cloneable log of hook fires. Each installed hook double clones this
/// handle and appends an entry on every fire; a test reads the entries back.
#[derive(Clone, Default)]
pub struct RecordingHookLog {
    fires: Arc<Mutex<Vec<String>>>,
}

impl RecordingHookLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, entry: impl Into<String>) {
        self.fires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(entry.into());
    }

    /// A snapshot of every recorded fire, in order.
    pub fn fires(&self) -> Vec<String> {
        self.fires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether any recorded fire contains `needle`.
    pub fn fired(&self, needle: &str) -> bool {
        self.fires().iter().any(|f| f.contains(needle))
    }
}

/// A recording `AfterModel` observer. Observers cannot affect outcomes; this one
/// records the observed kind so a test can prove the observer fired.
struct RecordingObserverHook {
    log: RecordingHookLog,
}

#[async_trait]
impl ObserverHook for RecordingObserverHook {
    async fn observe(&self, ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
        self.log.record(format!("observer:{:?}", ctx.observed_kind));
    }
}

/// A recording `BeforeCapability` gate hook that records the capability name and
/// then `pass()`es (no opinion) — it observes the point without altering the
/// composed decision, so the capability still runs.
struct RecordingBeforeCapabilityHook {
    log: RecordingHookLog,
}

#[async_trait]
impl PrivilegedBeforeCapabilityHook for RecordingBeforeCapabilityHook {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn PrivilegedGateSink) {
        self.log
            .record(format!("before_capability:{}", ctx.capability_name));
        sink.pass();
    }
}

/// A `BeforeCapability` gate hook that DENIES `deny_target` (records the fire,
/// then `deny()`s) and `pass()`es every other capability. Drives the C-HOOKS
/// error path: a hook deny must block the capability without wedging the run.
struct DenyBeforeCapabilityHook {
    log: RecordingHookLog,
    deny_target: String,
}

#[async_trait]
impl PrivilegedBeforeCapabilityHook for DenyBeforeCapabilityHook {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn PrivilegedGateSink) {
        if ctx.capability_name == self.deny_target {
            self.log
                .record(format!("before_capability_deny:{}", ctx.capability_name));
            sink.deny(HOOK_TEST_DENY_REASON);
        } else {
            sink.pass();
        }
    }
}

fn hook_install_err(context: &str, error: impl std::fmt::Display) -> RebornLoopDriverHostError {
    RebornLoopDriverHostError::InvalidRequest {
        reason: format!("failed to install {context} recording hook: {error}"),
    }
}

/// A `HookDispatcherBuilderFactory` that installs a recording `AfterModel`
/// observer plus a recording (passing) `BeforeCapability` gate hook, both
/// writing to `log`. The observer fires on every model call; the gate hook fires
/// before every capability invocation — so a turn that invokes a capability
/// records BOTH points. Mints a fresh dispatcher per call (per-run isolation).
pub fn recording_hook_factory(log: RecordingHookLog) -> HookDispatcherBuilderFactory {
    Arc::new(move || {
        let log = log.clone();
        let observer_id = HookId::for_builtin(RECORDING_OBSERVER_PATH, HookVersion::ONE);
        let before_capability_id =
            HookId::for_builtin(RECORDING_BEFORE_CAPABILITY_PATH, HookVersion::ONE);
        HookDispatcherBuilder::new(HookRegistry::new())
            .install_builtin_observer(
                observer_id,
                HookPhase::Telemetry,
                HookPointSpec::AfterModel,
                Box::new(RecordingObserverHook { log: log.clone() }),
            )
            .map_err(|error| hook_install_err("observer", error))?
            .install_builtin_before_capability(
                before_capability_id,
                HookPhase::Policy,
                Box::new(RecordingBeforeCapabilityHook { log }),
            )
            .map_err(|error| hook_install_err("before_capability", error))
    })
}

/// A `HookDispatcherBuilderFactory` that installs a recording `AfterModel`
/// observer plus a `BeforeCapability` gate hook that DENIES `deny_target`. Used
/// to prove a hook deny blocks the capability without wedging the run.
pub fn denying_hook_factory(
    log: RecordingHookLog,
    deny_target: impl Into<String>,
) -> HookDispatcherBuilderFactory {
    let deny_target = deny_target.into();
    Arc::new(move || {
        let log = log.clone();
        let deny_target = deny_target.clone();
        let observer_id = HookId::for_builtin(RECORDING_OBSERVER_PATH, HookVersion::ONE);
        let before_capability_id =
            HookId::for_builtin(RECORDING_BEFORE_CAPABILITY_PATH, HookVersion::ONE);
        HookDispatcherBuilder::new(HookRegistry::new())
            .install_builtin_observer(
                observer_id,
                HookPhase::Telemetry,
                HookPointSpec::AfterModel,
                Box::new(RecordingObserverHook { log: log.clone() }),
            )
            .map_err(|error| hook_install_err("observer", error))?
            .install_builtin_before_capability(
                before_capability_id,
                HookPhase::Policy,
                Box::new(DenyBeforeCapabilityHook { log, deny_target }),
            )
            .map_err(|error| hook_install_err("deny before_capability", error))
    })
}
