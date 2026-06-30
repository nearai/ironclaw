//! Hook-framework scaffolding for the reborn integration-test harness
//! (E-HOOK-INFRA seam).
//!
//! Reborn turns wire the hook framework `None` (dormant) by default. This
//! module provides the *recording* test double plus the factory that installs
//! it, so a later coverage PR (C-HOOKS) can flip the
//! `hook_dispatcher_builder_factory` on `assemble_thread_runtime` from `None`
//! to `Some(build_hook_dispatcher_builder_factory(hook))` and assert which
//! observer points fired.
//!
//! Shape mirrors the production-side `first_party_only_hook_factory` test
//! double in `crates/ironclaw_reborn/tests/loop_driver_host.rs` and the
//! recording-double conventions in `process.rs` / `delivery.rs`.

// Not every test binary that mounts this support tree exercises the hook
// scaffolding — mirrors the `#![allow(dead_code)]` used in sibling modules.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::{ObservedKind, ObserverHookContext};
use ironclaw_hooks::registry::{HookPointSpec, HookRegistry};
use ironclaw_hooks::sink::{ObserverHook, ObserverSink};
use ironclaw_hooks::{HookId, HookVersion};
use ironclaw_reborn::loop_driver_host::HookDispatcherBuilderFactory;

/// Stable builtin path for the recording observer's `HookId`.
const RECORDING_OBSERVER_PATH: &str = "tests/support/reborn/harness_hooks::RecordingObserverHook";

/// Observer hook that records every `ObservedKind` it is notified about, in
/// call order, for test assertions. Observers cannot alter outcomes, so this is
/// pure capture (no `sink.note(..)`).
///
/// Conventions match the sibling recording doubles (`RecordingProcessPort`,
/// `RecordingOutboundDeliverySink`): `Arc<Mutex<Vec<T>>>` interior, a plural
/// snapshot accessor, and a `"... lock poisoned"` expect message.
#[derive(Debug, Clone, Default)]
pub struct RecordingObserverHook {
    observed_kinds: Arc<Mutex<Vec<ObservedKind>>>,
}

impl RecordingObserverHook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every observed kind recorded so far, in dispatch order.
    pub fn observed_kinds(&self) -> Vec<ObservedKind> {
        self.observed_kinds
            .lock()
            .expect("recording observer hook lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ObserverHook for RecordingObserverHook {
    async fn observe(&self, ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
        self.observed_kinds
            .lock()
            .expect("recording observer hook lock poisoned")
            .push(ctx.observed_kind);
    }
}

/// Build a `HookDispatcherBuilderFactory` that installs `hook` as a Builtin
/// Telemetry-phase observer at `AfterCapability`.
///
/// Unused in PR-E1 (no harness seam wires it yet); consumed by C-HOOKS once the
/// trivial `with_hook_dispatcher_builder_factory` setter lands and flips the
/// `assemble_thread_runtime` factory from `None` to `Some(..)`. The caller
/// retains a `hook.clone()` (cheap inner-`Arc` clone) to read `observed_kinds()`
/// after the turn. Mirrors `first_party_only_hook_factory`
/// (`crates/ironclaw_reborn/tests/loop_driver_host.rs`).
pub(super) fn build_hook_dispatcher_builder_factory(
    hook: RecordingObserverHook,
) -> HookDispatcherBuilderFactory {
    Arc::new(move || {
        let hook_id = HookId::for_builtin(RECORDING_OBSERVER_PATH, HookVersion::ONE);
        Ok(HookDispatcherBuilder::new(HookRegistry::new())
            .install_builtin_observer(
                hook_id,
                HookPhase::Telemetry,
                HookPointSpec::AfterCapability,
                Box::new(hook.clone()),
            )
            .expect("install recording observer hook"))
    })
}
