use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wasmtime::{Caller, Config, Engine, Instance, Linker, Module, Store};

// Shared resource limiter extracted into `ironclaw_wasm_limiter` so both
// `ironclaw_hooks` and `ironclaw_wasm` depend on it through Cargo rather
// than a `#[path = ...]` file import (henrypark133 must-fix #1 on PR
// #3634). The micro-crate sits below both consumers and doesn't pull in
// the rest of the tool-WASM surface, so the architecture rule against
// `ironclaw_hooks -> ironclaw_wasm` is preserved.
use ironclaw_wasm_limiter::WasmResourceLimiter;

use crate::failure_policy::FailureCategory;
use crate::identity::HookLocalId;
use crate::kinds::mutator::{HookPatch, PatchOrdinalHint};
use crate::kinds::observer::{NoteCategory, ObserverFact};
use crate::manifest::{HookManifestKind, WasmBudget};
use crate::points::{BeforeCapabilityHookContext, BeforePromptHookContext, ObserverHookContext};
use crate::registry::HookPointSpec;
use crate::trust::HookTrustClass;
use crate::{dispatch::GateHookOutcome, error::SanitizedReason};

const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(10);
const MAX_SINK_CALLS_PER_INVOCATION: u32 = 64;
const MAX_TOTAL_PATCH_BYTES: u32 = 4 * 1024;
const MAX_GUEST_STRING_BYTES: usize = MAX_TOTAL_PATCH_BYTES as usize;
const MAX_OBSERVER_FACTS_PER_INVOCATION: u32 = 32;
const MAX_DECISION_CALLS_PER_INVOCATION: u32 = 1;

const STATUS_OK: i32 = 0;
const STATUS_REJECTED: i32 = -1;

const BEFORE_CAPABILITY_IMPORT_MODULE: &str = "ic:hooks/before-capability@1";
const BEFORE_PROMPT_IMPORT_MODULE: &str = "ic:hooks/before-prompt@1";
const OBSERVER_IMPORT_MODULE: &str = "ic:hooks/observer@1";

/// Runtime-visible request for a hook WASM module.
#[derive(Debug)]
pub struct WasmHookModuleRequest<'a> {
    pub extension_id: &'a ironclaw_host_api::ExtensionId,
    pub extension_version: &'a str,
    pub hook_local_id: &'a HookLocalId,
    pub kind: HookManifestKind,
    pub export: &'a str,
}

/// Resolves the already-installed module bytes for a manifest hook body.
///
/// The manifest's `HookManifestBody::Wasm` names only the export and budget;
/// the physical module belongs to the extension install/registry layer. This
/// trait is the narrow handoff from that layer into the hook registrar.
pub trait WasmHookModuleResolver: Send + Sync {
    fn resolve_module(
        &self,
        request: &WasmHookModuleRequest<'_>,
    ) -> Result<Vec<u8>, WasmHookRuntimeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHookFailure {
    pub category: FailureCategory,
    pub reason: &'static str,
}

impl WasmHookFailure {
    fn trap() -> Self {
        Self {
            category: FailureCategory::Panic,
            reason: "wasm hook trapped",
        }
    }

    fn timeout() -> Self {
        Self {
            category: FailureCategory::Timeout,
            reason: "wasm hook exceeded dispatch timeout",
        }
    }

    fn malformed(reason: &'static str) -> Self {
        Self {
            category: FailureCategory::Malformed,
            reason,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WasmHookRuntimeError {
    #[error("wasm hook runtime engine creation failed: {0}")]
    Engine(String),
    #[error("wasm hook module unavailable: {0}")]
    ModuleUnavailable(String),
    #[error("wasm hook module compilation failed: {0}")]
    Compilation(String),
    #[error("wasm hook budget is invalid: {0}")]
    Budget(String),
    #[error("wasm hook export `{export}` is invalid: {reason}")]
    InvalidExport { export: String, reason: String },
    /// Module's imports don't match the host surface for the target hook
    /// point. Surfaced at install time (`prepare()`) by a scratch
    /// instantiation so bad modules never reach live dispatch.
    #[error("wasm hook imports do not match host surface for {point}: {reason}")]
    InvalidImports { point: &'static str, reason: String },
    #[error("wasm hook execution failed: {0}")]
    Execution(String),
    #[error("wasm hook runtime cache lock poisoned")]
    CachePoisoned,
}

impl WasmHookRuntimeError {
    pub fn module_unavailable(reason: impl Into<String>) -> Self {
        Self::ModuleUnavailable(reason.into())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedWasmHook {
    module: Arc<Module>,
    module_digest: blake3::Hash,
    export: String,
    limits: WasmHookLimits,
}

impl PreparedWasmHook {
    pub(crate) fn module_digest_hex(&self) -> String {
        self.module_digest.to_hex().to_string()
    }
}

#[derive(Debug, Clone)]
struct WasmHookLimits {
    fuel: u64,
    memory_bytes: u64,
    wall: Duration,
}

impl TryFrom<WasmBudget> for WasmHookLimits {
    type Error = WasmHookRuntimeError;

    fn try_from(value: WasmBudget) -> Result<Self, Self::Error> {
        let memory_bytes = u64::from(value.memory_mb)
            .checked_mul(1024)
            .and_then(|bytes| bytes.checked_mul(1024))
            .ok_or_else(|| WasmHookRuntimeError::Budget("memory budget overflow".to_string()))?;
        Ok(Self {
            fuel: value.fuel,
            memory_bytes,
            wall: Duration::from_millis(u64::from(value.wall_ms)),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WasmHookPoint {
    BeforeCapability,
    BeforePrompt,
    Observer(HookPointSpec),
}

impl WasmHookPoint {
    fn label(self) -> &'static str {
        match self {
            Self::BeforeCapability => "before_capability",
            Self::BeforePrompt => "before_prompt",
            Self::Observer(HookPointSpec::AfterModel) => "after_model",
            Self::Observer(HookPointSpec::AfterCapability) => "after_capability",
            Self::Observer(HookPointSpec::AfterCheckpoint) => "after_checkpoint",
            Self::Observer(_) => "observer",
        }
    }
}

/// Shared runtime for WASM hooks. A compiled-module cache lives on the runtime,
/// while each hook invocation receives a fresh `wasmtime::Store`.
pub struct WasmHookRuntime {
    engine: Engine,
    resolver: Arc<dyn WasmHookModuleResolver>,
    modules: Mutex<HashMap<blake3::Hash, Arc<Module>>>,
}

impl WasmHookRuntime {
    pub fn new(resolver: Arc<dyn WasmHookModuleResolver>) -> Result<Self, WasmHookRuntimeError> {
        let mut config = Config::new();
        config.wasm_threads(false);
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.debug_info(false);
        let engine = Engine::new(&config)
            .map_err(|error| WasmHookRuntimeError::Engine(error.to_string()))?;
        spawn_epoch_ticker(engine.clone())?;
        Ok(Self {
            engine,
            resolver,
            modules: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn prepare(
        &self,
        request: &WasmHookModuleRequest<'_>,
        budget: WasmBudget,
    ) -> Result<PreparedWasmHook, WasmHookRuntimeError> {
        let bytes = self.resolver.resolve_module(request)?;
        let digest = blake3::hash(&bytes);
        let cached = self
            .modules
            .lock()
            .map_err(|_| WasmHookRuntimeError::CachePoisoned)?
            .get(&digest)
            .cloned();
        let module = match cached {
            Some(module) => module,
            None => {
                let module = Arc::new(
                    Module::new(&self.engine, &bytes)
                        .map_err(|error| WasmHookRuntimeError::Compilation(error.to_string()))?,
                );
                let mut modules = self
                    .modules
                    .lock()
                    .map_err(|_| WasmHookRuntimeError::CachePoisoned)?;
                modules
                    .entry(digest)
                    .or_insert_with(|| Arc::clone(&module))
                    .clone()
            }
        };
        let limits: WasmHookLimits = budget.try_into()?;
        // Install-time ABI validation. Scratch-instantiate the module against
        // the linker for this hook point so any unsupported import, missing
        // export, or wrong export signature surfaces here — at registration —
        // rather than deferring to first live dispatch. Per-prepare cost is
        // small (one extra `Linker::instantiate`); module-compile output is
        // already cached above.
        let point = wasm_point_for_kind(request.kind);
        validate_module_abi(&self.engine, &module, &limits, point, request.export)?;
        Ok(PreparedWasmHook {
            module,
            module_digest: digest,
            export: request.export.to_string(),
            limits,
        })
    }

    pub(crate) fn execute_gate(
        &self,
        prepared: &PreparedWasmHook,
        _ctx: &BeforeCapabilityHookContext,
    ) -> Result<GateHookOutcome, WasmHookFailure> {
        let mut output = self.execute(prepared, WasmHookPoint::BeforeCapability)?;
        match output.gate.take() {
            Some(GateWasmDecision::Pass) => Ok(GateHookOutcome::Pass),
            Some(GateWasmDecision::Deny(reason)) => Ok(GateHookOutcome::Decision {
                decision: crate::kinds::gate::BeforeCapabilityHookDecision::deny(
                    SanitizedReason::from_static(reason),
                ),
                audit_reason: None,
            }),
            Some(GateWasmDecision::PauseApproval(reason)) => Ok(GateHookOutcome::Decision {
                decision: crate::kinds::gate::BeforeCapabilityHookDecision::pause_approval(
                    SanitizedReason::from_static(reason),
                ),
                audit_reason: None,
            }),
            Some(GateWasmDecision::PauseAuth(reason)) => Ok(GateHookOutcome::Decision {
                decision: crate::kinds::gate::BeforeCapabilityHookDecision::pause_auth(
                    SanitizedReason::from_static(reason),
                ),
                audit_reason: None,
            }),
            None => Err(WasmHookFailure::malformed(
                "wasm gate hook completed without minting a decision",
            )),
        }
    }

    pub(crate) fn execute_prompt(
        &self,
        prepared: &PreparedWasmHook,
        _ctx: &BeforePromptHookContext,
    ) -> Result<Vec<HookPatch>, WasmHookFailure> {
        let output = self.execute(prepared, WasmHookPoint::BeforePrompt)?;
        Ok(output.patches)
    }

    pub(crate) fn execute_observer(
        &self,
        prepared: &PreparedWasmHook,
        point: HookPointSpec,
        _ctx: &ObserverHookContext,
    ) -> Result<Vec<ObserverFact>, WasmHookFailure> {
        let output = self.execute(prepared, WasmHookPoint::Observer(point))?;
        Ok(output.facts)
    }

    fn execute(
        &self,
        prepared: &PreparedWasmHook,
        point: WasmHookPoint,
    ) -> Result<WasmHookOutput, WasmHookFailure> {
        let started = Instant::now();
        let mut store = Store::new(
            &self.engine,
            HookStoreData::new(prepared.limits.memory_bytes, prepared.limits.wall, point),
        );
        configure_store(&mut store, &prepared.limits).map_err(|_| WasmHookFailure::trap())?;
        let linker = create_linker(&self.engine, point).map_err(|_| WasmHookFailure::trap())?;
        let instance = linker
            .instantiate(&mut store, &prepared.module)
            .map_err(|_| {
                WasmHookFailure::malformed("wasm hook imports do not match host surface")
            })?;
        let export = get_export(&mut store, &instance, &prepared.export)?;
        match export.call(&mut store, ()) {
            Ok(()) => {}
            Err(_) if store.data().deadline_exceeded() => return Err(WasmHookFailure::timeout()),
            Err(_) => return Err(WasmHookFailure::trap()),
        }
        if store.data().deadline_exceeded() {
            return Err(WasmHookFailure::timeout());
        }
        if let Some(failure) = store.data_mut().failure.take() {
            return Err(failure);
        }
        let mut output = WasmHookOutput::default();
        std::mem::swap(&mut output, &mut store.data_mut().output);
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            point = point.label(),
            "wasm hook invocation completed"
        );
        Ok(output)
    }
}

impl std::fmt::Debug for WasmHookRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHookRuntime").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct WasmBeforeCapabilityHook {
    runtime: Arc<WasmHookRuntime>,
    prepared: PreparedWasmHook,
}

impl WasmBeforeCapabilityHook {
    pub(crate) fn new(runtime: Arc<WasmHookRuntime>, prepared: PreparedWasmHook) -> Self {
        Self { runtime, prepared }
    }

    pub(crate) fn evaluate(
        &self,
        ctx: &BeforeCapabilityHookContext,
    ) -> Result<GateHookOutcome, WasmHookFailure> {
        self.runtime.execute_gate(&self.prepared, ctx)
    }
}

#[derive(Debug, Clone)]
pub struct WasmBeforePromptHook {
    runtime: Arc<WasmHookRuntime>,
    prepared: PreparedWasmHook,
}

impl WasmBeforePromptHook {
    pub(crate) fn new(runtime: Arc<WasmHookRuntime>, prepared: PreparedWasmHook) -> Self {
        Self { runtime, prepared }
    }

    pub(crate) fn evaluate(
        &self,
        ctx: &BeforePromptHookContext,
    ) -> Result<Vec<HookPatch>, WasmHookFailure> {
        self.runtime.execute_prompt(&self.prepared, ctx)
    }
}

#[derive(Debug, Clone)]
pub struct WasmObserverHook {
    runtime: Arc<WasmHookRuntime>,
    prepared: PreparedWasmHook,
    point: HookPointSpec,
}

impl WasmObserverHook {
    pub(crate) fn new(
        runtime: Arc<WasmHookRuntime>,
        prepared: PreparedWasmHook,
        point: HookPointSpec,
    ) -> Self {
        Self {
            runtime,
            prepared,
            point,
        }
    }

    pub(crate) fn observe(
        &self,
        ctx: &ObserverHookContext,
    ) -> Result<Vec<ObserverFact>, WasmHookFailure> {
        self.runtime
            .execute_observer(&self.prepared, self.point, ctx)
    }
}

#[derive(Debug)]
struct HookStoreData {
    limiter: WasmResourceLimiter,
    deadline: Option<Instant>,
    point: WasmHookPoint,
    budget: HostImportBudget,
    output: WasmHookOutput,
    failure: Option<WasmHookFailure>,
}

impl HookStoreData {
    fn new(memory_limit: u64, timeout: Duration, point: WasmHookPoint) -> Self {
        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            deadline: Instant::now().checked_add(timeout),
            point,
            budget: HostImportBudget::default(),
            output: WasmHookOutput::default(),
            failure: None,
        }
    }

    fn deadline_exceeded(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn fail(&mut self, failure: WasmHookFailure) -> i32 {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
        STATUS_REJECTED
    }

    fn count_sink_call(&mut self) -> bool {
        self.budget.sink_calls = self.budget.sink_calls.saturating_add(1);
        self.budget.sink_calls <= MAX_SINK_CALLS_PER_INVOCATION
    }

    fn count_decision_call(&mut self) -> bool {
        self.budget.decision_calls = self.budget.decision_calls.saturating_add(1);
        self.budget.decision_calls <= MAX_DECISION_CALLS_PER_INVOCATION
    }
}

#[derive(Debug, Default)]
struct HostImportBudget {
    sink_calls: u32,
    total_patch_bytes: u32,
    observer_facts: u32,
    decision_calls: u32,
}

#[derive(Debug, Default)]
struct WasmHookOutput {
    gate: Option<GateWasmDecision>,
    patches: Vec<HookPatch>,
    facts: Vec<ObserverFact>,
}

#[derive(Debug, Clone, Copy)]
enum GateWasmDecision {
    Pass,
    Deny(&'static str),
    PauseApproval(&'static str),
    PauseAuth(&'static str),
}

fn spawn_epoch_ticker(engine: Engine) -> Result<(), WasmHookRuntimeError> {
    std::thread::Builder::new()
        .name("reborn-hook-wasm-epoch-ticker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK_INTERVAL);
                engine.increment_epoch();
            }
        })
        .map(|_| ())
        .map_err(|error| WasmHookRuntimeError::Engine(error.to_string()))
}

fn configure_store(
    store: &mut Store<HookStoreData>,
    limits: &WasmHookLimits,
) -> Result<(), WasmHookRuntimeError> {
    store
        .set_fuel(limits.fuel)
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    store.epoch_deadline_trap();
    let ticks = (limits.wall.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);
    Ok(())
}

fn create_linker(
    engine: &Engine,
    point: WasmHookPoint,
) -> Result<Linker<HookStoreData>, WasmHookRuntimeError> {
    let mut linker = Linker::new(engine);
    match point {
        WasmHookPoint::BeforeCapability => add_gate_imports(&mut linker)?,
        WasmHookPoint::BeforePrompt => add_prompt_imports(&mut linker)?,
        WasmHookPoint::Observer(_) => add_observer_imports(&mut linker)?,
    }
    Ok(linker)
}

fn wasm_point_for_kind(kind: HookManifestKind) -> WasmHookPoint {
    match kind {
        HookManifestKind::BeforeCapability => WasmHookPoint::BeforeCapability,
        HookManifestKind::BeforePrompt => WasmHookPoint::BeforePrompt,
        HookManifestKind::AfterModel => WasmHookPoint::Observer(HookPointSpec::AfterModel),
        HookManifestKind::AfterCapability => {
            WasmHookPoint::Observer(HookPointSpec::AfterCapability)
        }
        HookManifestKind::AfterCheckpoint => {
            WasmHookPoint::Observer(HookPointSpec::AfterCheckpoint)
        }
    }
}

/// Scratch-instantiate `module` against the linker for `point` and resolve
/// the named export. Surfaces ABI mismatches at `prepare()` time rather than
/// deferring to first live dispatch. Returns `Ok(())` if the module satisfies
/// the host surface contract; otherwise [`WasmHookRuntimeError::InvalidImports`]
/// or [`WasmHookRuntimeError::InvalidExport`].
fn validate_module_abi(
    engine: &Engine,
    module: &Module,
    limits: &WasmHookLimits,
    point: WasmHookPoint,
    export: &str,
) -> Result<(), WasmHookRuntimeError> {
    let mut store = Store::new(
        engine,
        HookStoreData::new(limits.memory_bytes, limits.wall, point),
    );
    // Configure fuel/epoch the same way `execute()` does. Instantiation does
    // not consume fuel, but the limiter API rejects stores without resource
    // bounds installed.
    configure_store(&mut store, limits)?;
    let linker = create_linker(engine, point)?;
    let instance = linker.instantiate(&mut store, module).map_err(|error| {
        WasmHookRuntimeError::InvalidImports {
            point: point.label(),
            reason: error.to_string(),
        }
    })?;
    instance
        .get_typed_func::<(), ()>(&mut store, export)
        .map_err(|error| WasmHookRuntimeError::InvalidExport {
            export: export.to_string(),
            reason: error.to_string(),
        })?;
    Ok(())
}

fn get_export(
    store: &mut Store<HookStoreData>,
    instance: &Instance,
    export: &str,
) -> Result<wasmtime::TypedFunc<(), ()>, WasmHookFailure> {
    instance
        .get_typed_func::<(), ()>(&mut *store, export)
        .map_err(|_| WasmHookFailure::malformed("wasm hook export has wrong type"))
}

fn add_gate_imports(linker: &mut Linker<HookStoreData>) -> Result<(), WasmHookRuntimeError> {
    linker
        .func_wrap(
            BEFORE_CAPABILITY_IMPORT_MODULE,
            "deny",
            |mut caller: Caller<'_, HookStoreData>, reason: i32| -> i32 {
                record_gate_decision(&mut caller, GateWasmDecision::Deny(reason_for_code(reason)))
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    linker
        .func_wrap(
            BEFORE_CAPABILITY_IMPORT_MODULE,
            "pause_approval",
            |mut caller: Caller<'_, HookStoreData>, reason: i32| -> i32 {
                record_gate_decision(
                    &mut caller,
                    GateWasmDecision::PauseApproval(reason_for_code(reason)),
                )
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    linker
        .func_wrap(
            BEFORE_CAPABILITY_IMPORT_MODULE,
            "pause_auth",
            |mut caller: Caller<'_, HookStoreData>, reason: i32| -> i32 {
                record_gate_decision(
                    &mut caller,
                    GateWasmDecision::PauseAuth(reason_for_code(reason)),
                )
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    linker
        .func_wrap(
            BEFORE_CAPABILITY_IMPORT_MODULE,
            "pass",
            |mut caller: Caller<'_, HookStoreData>| -> i32 {
                record_gate_decision(&mut caller, GateWasmDecision::Pass)
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    Ok(())
}

fn add_prompt_imports(linker: &mut Linker<HookStoreData>) -> Result<(), WasmHookRuntimeError> {
    linker
        .func_wrap(
            BEFORE_PROMPT_IMPORT_MODULE,
            "add_envelope_snippet",
            |mut caller: Caller<'_, HookStoreData>, ptr: i32, len: i32, ordinal: i32| -> i32 {
                add_envelope_snippet(&mut caller, ptr, len, ordinal)
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    linker
        .func_wrap(
            BEFORE_PROMPT_IMPORT_MODULE,
            "add_milestone_metadata",
            |mut caller: Caller<'_, HookStoreData>, key: i32, ptr: i32, len: i32| -> i32 {
                add_milestone_metadata(&mut caller, key, ptr, len)
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    Ok(())
}

fn add_observer_imports(linker: &mut Linker<HookStoreData>) -> Result<(), WasmHookRuntimeError> {
    linker
        .func_wrap(
            OBSERVER_IMPORT_MODULE,
            "note",
            |mut caller: Caller<'_, HookStoreData>, category: i32, summary: i32| -> i32 {
                add_observer_note(&mut caller, category, summary)
            },
        )
        .map_err(|error| WasmHookRuntimeError::Execution(error.to_string()))?;
    Ok(())
}

fn record_gate_decision(caller: &mut Caller<'_, HookStoreData>, decision: GateWasmDecision) -> i32 {
    let data = caller.data_mut();
    if !matches!(data.point, WasmHookPoint::BeforeCapability) {
        return data.fail(WasmHookFailure::malformed(
            "wasm gate import used from the wrong hook point",
        ));
    }
    if !data.count_sink_call() {
        return data.fail(WasmHookFailure::malformed(
            "wasm hook exceeded sink-call budget",
        ));
    }
    if !data.count_decision_call() {
        return data.fail(WasmHookFailure::malformed(
            "wasm gate hook emitted more than one decision",
        ));
    }
    data.output.gate = Some(decision);
    STATUS_OK
}

fn add_envelope_snippet(
    caller: &mut Caller<'_, HookStoreData>,
    ptr: i32,
    len: i32,
    ordinal: i32,
) -> i32 {
    if !matches!(caller.data().point, WasmHookPoint::BeforePrompt) {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm prompt import used from the wrong hook point",
        ));
    }
    if !caller.data_mut().count_sink_call() {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook exceeded sink-call budget",
        ));
    }
    let Some(body) = read_guest_string(caller, ptr, len) else {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook supplied an invalid string pointer",
        ));
    };
    let Some(ordinal_hint) = ordinal_hint_for_code(ordinal) else {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook supplied an invalid ordinal hint",
        ));
    };
    let patch =
        match HookPatch::add_enveloped_snippet(body, HookTrustClass::Installed, ordinal_hint) {
            Ok(patch) => patch,
            Err(_) => {
                return caller.data_mut().fail(WasmHookFailure::malformed(
                    "wasm hook emitted an invalid prompt patch",
                ));
            }
        };
    let byte_count = patch.snippet_byte_count();
    let data = caller.data_mut();
    let total = data.budget.total_patch_bytes.saturating_add(byte_count);
    if total > MAX_TOTAL_PATCH_BYTES {
        return data.fail(WasmHookFailure::malformed(
            "wasm hook exceeded total prompt-patch byte budget",
        ));
    }
    data.budget.total_patch_bytes = total;
    data.output.patches.push(patch);
    STATUS_OK
}

fn add_milestone_metadata(
    caller: &mut Caller<'_, HookStoreData>,
    key: i32,
    ptr: i32,
    len: i32,
) -> i32 {
    if !matches!(caller.data().point, WasmHookPoint::BeforePrompt) {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm prompt import used from the wrong hook point",
        ));
    }
    if !caller.data_mut().count_sink_call() {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook exceeded sink-call budget",
        ));
    }
    let Some(value) = read_guest_string(caller, ptr, len) else {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook supplied an invalid metadata string pointer",
        ));
    };
    let Some(key) = metadata_key_for_code(key) else {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook supplied an invalid metadata key",
        ));
    };
    let Some(byte_count) = u32::try_from(value.len()).ok() else {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook exceeded total prompt-patch byte budget",
        ));
    };
    let data = caller.data_mut();
    let total = data.budget.total_patch_bytes.saturating_add(byte_count);
    if total > MAX_TOTAL_PATCH_BYTES {
        return data.fail(WasmHookFailure::malformed(
            "wasm hook exceeded total prompt-patch byte budget",
        ));
    }
    data.budget.total_patch_bytes = total;
    data.output
        .patches
        .push(HookPatch::add_milestone_metadata(key, value));
    STATUS_OK
}

fn add_observer_note(caller: &mut Caller<'_, HookStoreData>, category: i32, summary: i32) -> i32 {
    if !matches!(caller.data().point, WasmHookPoint::Observer(_)) {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm observer import used from the wrong hook point",
        ));
    }
    if !caller.data_mut().count_sink_call() {
        return caller.data_mut().fail(WasmHookFailure::malformed(
            "wasm hook exceeded sink-call budget",
        ));
    }
    let data = caller.data_mut();
    data.budget.observer_facts = data.budget.observer_facts.saturating_add(1);
    if data.budget.observer_facts > MAX_OBSERVER_FACTS_PER_INVOCATION {
        return data.fail(WasmHookFailure::malformed(
            "wasm observer exceeded fact budget",
        ));
    }
    let Some(category) = note_category_for_code(category) else {
        return data.fail(WasmHookFailure::malformed(
            "wasm observer supplied an invalid note category",
        ));
    };
    data.output.facts.push(ObserverFact::note(
        category,
        SanitizedReason::from_static(summary_for_code(summary)),
    ));
    STATUS_OK
}

fn read_guest_string(caller: &mut Caller<'_, HookStoreData>, ptr: i32, len: i32) -> Option<String> {
    let ptr = usize::try_from(ptr).ok()?;
    let len = usize::try_from(len).ok()?;
    if len > MAX_GUEST_STRING_BYTES {
        return None;
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    let end = ptr.checked_add(len)?;
    let bytes = memory.data(&*caller).get(ptr..end)?.to_vec();
    String::from_utf8(bytes).ok()
}

fn reason_for_code(code: i32) -> &'static str {
    match code {
        1 => "hook_predicate_denied",
        2 => "hook_wasm_pause_approval",
        3 => "hook_wasm_pause_auth",
        _ => "hook_wasm_denied",
    }
}

fn summary_for_code(code: i32) -> &'static str {
    match code {
        1 => "wasm observer fired",
        2 => "wasm observer skipped",
        3 => "wasm observer slow",
        4 => "wasm observer protocol violation",
        _ => "wasm observer note",
    }
}

fn ordinal_hint_for_code(code: i32) -> Option<PatchOrdinalHint> {
    match code {
        0 => Some(PatchOrdinalHint::Last),
        1 => Some(PatchOrdinalHint::NearTop),
        _ => None,
    }
}

fn metadata_key_for_code(code: i32) -> Option<crate::kinds::mutator::MetadataKey> {
    match code {
        0 => Some(crate::kinds::mutator::MetadataKey::from_static("wasm_hook")),
        1 => Some(crate::kinds::mutator::MetadataKey::from_static(
            "wasm_hook_detail",
        )),
        _ => None,
    }
}

fn note_category_for_code(code: i32) -> Option<NoteCategory> {
    match code {
        0 => Some(NoteCategory::HookFired),
        1 => Some(NoteCategory::HookSkipped),
        2 => Some(NoteCategory::HookSlow),
        3 => Some(NoteCategory::HookProtocolViolation),
        _ => None,
    }
}
