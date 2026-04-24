# Reborn WASM Contract

**Status:** Draft implementation contract  
**Date:** 2026-04-24  
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/extensions.md`, `docs/reborn/contracts/resources.md`

---

## 1. Purpose

`ironclaw_wasm` is the portable installed-capability runtime for IronClaw Reborn.

It owns:

- WASM module validation and compilation
- fresh instance creation per invocation
- mapping an extension-declared `CapabilityDescriptor` to a WASM export
- fuel/time/memory/output limits
- the WASM host ABI/import surface
- conversion of runtime traps into structured errors

It does not discover extensions, manage manifests, own policy, own filesystem persistence, resolve secrets, open raw network clients, or charge budget directly without the resource governor.

---

## 2. Core invariant

WASM modules receive no ambient host authority.

```text
WASM module effect -> host import -> ExecutionContext/Action/Grants/Policy/Resources/Audit -> Decision
```

A module cannot read files, open sockets, invoke tools, or resolve secrets unless the host explicitly exposes an import that checks IronClaw authority. The initial V1 crate may start with no privileged host imports.

---

## 3. Runtime lane boundary

`ironclaw_extensions` declares that a capability uses `RuntimeKind::Wasm`.

`ironclaw_wasm` validates:

- descriptor runtime is `Wasm`
- descriptor provider matches the prepared module provider
- descriptor capability ID matches the prepared module capability/export binding
- invocation carries a governor-issued active reservation guard, not a caller-forgeable reservation record

The runtime must not accept arbitrary capability IDs or extension IDs that bypass extension registry validation.

---

## 4. Resource protocol

Before invoking a WASM capability:

```text
ironclaw_resources.reserve(scope, estimate)
```

Then:

```text
ironclaw_resources.active_reservation(reservation_id) -> active_guard
ironclaw_wasm.invoke(..., active_guard, ...)
```

After invocation:

```text
ironclaw_resources.reconcile(reservation_id, actual_usage)
```

`ironclaw_wasm` should produce measured usage signals where it can:

- wall-clock time
- output bytes
- fuel consumed
- memory limit/trap metadata when available

The resource governor remains the source of truth for reservation/reconciliation.

`ironclaw_wasm` now provides a convenience executor for the V1 WASM lane that owns this narrow lifecycle around a validated extension package:

```text
execute_extension_json
  -> reserve(scope, estimate)
  -> prepare_extension_capability(fs, package, capability)
  -> invoke_json(..., reservation, invocation)
  -> reconcile(reservation_id, actual_usage)
```

Failure cleanup rules:

- if `reserve` fails, no module is prepared or invoked
- if preparation fails after reservation, release the reservation
- if invocation fails after reservation, release the reservation
- if invocation succeeds, reconcile actual usage and return the resource receipt
- if reconciliation fails after successful invocation, release the reservation before returning the resource error
- cleanup failures are surfaced as resource-governor errors

The executor is not the global dispatcher. It only coordinates the WASM lane's reservation lifecycle until `ironclaw_kernel`/dispatch owns cross-runtime routing.

---

## 5. Execution model

V1 rules:

- compile/validate modules before invocation
- instantiate fresh per invocation
- no shared mutable instance state across calls
- fuel metering enabled
- memory growth bounded
- output bounded
- traps discard the instance
- invalid modules fail closed

The existing IronClaw codebase already uses Wasmtime with fuel, epoch interruption, memory limiting, compilation caching, and fresh instantiation patterns. Reborn should reuse those lessons but keep this crate narrower and contract-driven.

---

## 6. Engine runtime mechanics

`ironclaw_wasm` owns the Wasmtime engine mechanics that make the WASM lane safe enough to host untrusted portable code:

- **Fuel:** every store receives configured fuel before guest execution.
- **Epoch timeout:** the engine runs an epoch ticker; each invocation sets an epoch deadline derived from the configured timeout. Non-zero timeout and epoch tick interval are required.
- **Memory limiter:** stores attach a `ResourceLimiter`; memory growth beyond `max_memory_bytes` fails closed as `WasmError::MemoryExceeded`.
- **Module size limiter:** manifest-loaded module assets are checked with `RootFilesystem::stat` before `read_file` and again after read; assets above `max_module_bytes` fail closed as `WasmError::ModuleTooLarge`.
- **Compile cache:** `prepare_cached` caches compiled modules by provider, capability, export name, ABI version, and module content hash.
- **Persistent compilation cache:** an optional `cache_dir` may enable Wasmtime's on-disk compilation cache without changing capability authority. Cache setup errors are sanitized and must not leak raw host paths.
- **Fresh instance guarantee:** cached modules only reuse compiled code. Every invocation still creates a fresh store and instance, so mutable guest globals and memories do not carry across invocations.

The cache key must include the module content hash so modified bytes never reuse stale compiled code. Extension/manifest reload will later decide when to clear cache entries at package boundaries.

---

## 7. Extension package module loading

`ironclaw_extensions` remains the owner of discovery, manifest parsing, package validation, and descriptor extraction. `ironclaw_wasm` may consume a validated `ExtensionPackage` to prepare the module for one declared WASM capability.

Loading flow:

```text
ExtensionPackage
  -> verify package root is /system/extensions/<extension>
  -> verify package runtime is RuntimeKind::Wasm
  -> find requested CapabilityDescriptor in package.capabilities
  -> derive export name from capability suffix (`<extension>.<export>`)
  -> resolve runtime.module under package.root
  -> read bytes via RootFilesystem virtual path
  -> prepare_cached(WasmModuleSpec)
  -> PreparedWasmCapability { descriptor, module, module_path }
```

Rules:

- module assets are read through `RootFilesystem` and `VirtualPath`, never raw host paths
- runtime.module must resolve under the extension package root
- package root must match `/system/extensions/<extension>` even if a caller forges public package fields
- module assets must be size-checked before and after reading
- non-WASM packages fail with `ExtensionRuntimeMismatch`
- undeclared capabilities fail closed
- missing/mismatched WASM exports fail before invocation
- cache reuse is allowed only for matching provider/capability/export/content-hash/ABI-version

---

## 8. Host imports

V1 exposes low-risk core imports by default:

```text
host.log_utf8(level: i32, ptr: i32, len: i32) -> i32 status
host.time_unix_ms() -> i64
```

`host.log_utf8` reads UTF-8 bytes from the guest's exported `memory`, bounds message size/count, additionally caps total captured log bytes by the runtime output-byte budget, and records structured logs in `CapabilityResult.logs`. Over-budget logs return a non-zero status to the guest and are not captured. `host.time_unix_ms` returns host wall-clock milliseconds since Unix epoch. These imports do not grant filesystem, network, secret, process, or dispatch authority.

V1 also defines a scoped filesystem import group. These imports are linked for modules that declare them, but are default-deny unless the invocation uses `invoke_json_with_filesystem` with a `WasmScopedFilesystem` backed by `ScopedFilesystem` and a `MountView`:

```text
host.fs_read_utf8(path_ptr, path_len, out_ptr, out_cap) -> i32 bytes_or_negative_status
host.fs_write_utf8(path_ptr, path_len, data_ptr, data_len) -> i32 status
host.fs_list_utf8(path_ptr, path_len, out_ptr, out_cap) -> i32 bytes_or_negative_status
host.fs_stat_len(path_ptr, path_len) -> i64 len_or_negative_status
```

Filesystem import rules:

- paths are guest-visible `ScopedPath` values such as `/workspace/file.txt`
- `MountView` resolves scoped paths to `VirtualPath` targets
- read/list/stat/write go through `ScopedFilesystem` permission checks
- no raw `HostPath` reaches the guest
- no broad WASI preopens are granted
- missing filesystem context returns a negative guest status instead of ambient access

Unsupported imports fail at module preparation as `WasmError::UnsupportedImport`.

Future privileged imports should be grouped by service and routed through their owning host services:

```text
host.network.request
host.auth.resolve_secret_handle
host.dispatch.capability
host.events.emit
host.audit.emit
```

Rules:

- imports accept scoped/contract types, not raw host paths or raw secrets
- imports call system services instead of duplicating policy
- imports must be auditable
- imports must not bypass resource reservation
- imports must redact raw secrets and host paths from errors/logs

---

## 9. Initial JSON ABI

The first structured invocation ABI is intentionally small and pointer/length based. It is not WASI and does not grant filesystem, network, secret, or process authority.

A JSON-capable guest exports:

```text
memory                         exported linear memory
alloc(len: i32) -> i32          guest allocator for host-written input bytes
<capability_export>(ptr, len) -> i32 status
output_ptr() -> i32             pointer to guest output bytes
output_len() -> i32             length of guest output bytes
```

Host invocation flow:

```text
1. validate CapabilityDescriptor runtime/provider/capability
2. recursively validate CapabilityInvocation.input against descriptor.parameters_schema
3. instantiate a fresh module instance
4. call alloc(input_json_len)
5. write serialized JSON input bytes into guest memory
6. call the configured capability export
7. read output_ptr/output_len
8. enforce max_output_bytes before parsing
9. parse output bytes as JSON
10. return CapabilityResult or structured WasmError
```

Status contract:

- `status == 0`: output bytes are the JSON capability result.
- `status != 0`: output bytes should be a JSON error object; the host surfaces `WasmError::GuestError` with a bounded, control-character-stripped message.

This ABI is a V1 compatibility layer. It can coexist with, or be replaced by, Component Model/WIT once the host ABI is mature enough to freeze.

---

## 10. Minimum V1 API sketch

```rust
pub struct WasmRuntime;

pub struct WasmRuntimeConfig {
    pub fuel: u64,
    pub max_output_bytes: u64,
    pub max_memory_bytes: u64,
    pub max_module_bytes: u64,
    pub timeout: std::time::Duration,
    pub cache_compiled_modules: bool,
    pub cache_dir: Option<std::path::PathBuf>,
    pub epoch_tick_interval: std::time::Duration,
}

pub trait WasmHostFilesystem: Send + Sync {
    fn read_utf8(&self, path: &str) -> Result<String, String>;
    fn write_utf8(&self, path: &str, contents: &str) -> Result<(), String>;
    fn list_utf8(&self, path: &str) -> Result<String, String>;
    fn stat_len(&self, path: &str) -> Result<u64, String>;
}

pub struct WasmScopedFilesystem<F: RootFilesystem>;

pub struct WasmModuleSpec {
    pub provider: ExtensionId,
    pub capability: CapabilityId,
    pub export: String,
    pub bytes: Vec<u8>,
}

pub struct PreparedWasmCapability {
    pub descriptor: CapabilityDescriptor,
    pub module: Arc<PreparedWasmModule>,
    pub module_path: VirtualPath,
}

pub enum WasmLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

pub struct WasmLogEntry {
    pub level: WasmLogLevel,
    pub message: String,
    pub timestamp_unix_ms: u64,
}

pub struct CapabilityInvocation {
    pub input: serde_json::Value,
}

pub struct CapabilityResult {
    pub output: serde_json::Value,
    pub reservation_id: ResourceReservationId,
    pub usage: ResourceUsage,
    pub fuel_consumed: u64,
    pub output_bytes: u64,
    pub logs: Vec<WasmLogEntry>,
}

pub struct WasmExecutionRequest<'a> {
    pub package: &'a ExtensionPackage,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub invocation: CapabilityInvocation,
}

pub struct WasmExecutionResult {
    pub result: CapabilityResult,
    pub receipt: ResourceReceipt,
}

impl WasmRuntime {
    pub fn prepare(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError>;
    pub fn prepare_cached(&self, spec: WasmModuleSpec) -> Result<Arc<PreparedWasmModule>, WasmError>;
    pub async fn prepare_extension_capability<F: RootFilesystem>(
        &self,
        fs: &F,
        package: &ExtensionPackage,
        capability_id: &CapabilityId,
    ) -> Result<PreparedWasmCapability, WasmError>;
    pub async fn execute_extension_json<F: RootFilesystem, G: ResourceGovernor>(
        &self,
        fs: &F,
        governor: &G,
        request: WasmExecutionRequest<'_>,
    ) -> Result<WasmExecutionResult, WasmError>;
    pub fn invoke_json(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ActiveResourceReservation>,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResult, WasmError>;
    pub fn invoke_json_with_filesystem(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: Option<&ResourceReservation>,
        invocation: CapabilityInvocation,
        filesystem: Arc<dyn WasmHostFilesystem>,
    ) -> Result<CapabilityResult, WasmError>;
}
```

`invoke_i32` may remain as a tiny internal/test vertical slice, but user-facing capability execution should move through `invoke_json` until a stronger Component Model ABI replaces it.

---

## 11. Error contract

Minimum errors:

```rust
WasmError::InvalidConfig
WasmError::Cache
WasmError::Extension
WasmError::Filesystem
WasmError::Resource
WasmError::InvalidModule
WasmError::UnsupportedImport
WasmError::DescriptorMismatch
WasmError::PackageRootMismatch
WasmError::ExtensionRuntimeMismatch
WasmError::CapabilityNotDeclared
WasmError::InvalidInvocation
WasmError::ModuleTooLarge
WasmError::MissingReservation
WasmError::MissingExport
WasmError::MissingMemory
WasmError::GuestAllocation
WasmError::GuestError
WasmError::InvalidGuestOutput
WasmError::FuelExhausted
WasmError::MemoryExceeded
WasmError::Timeout
WasmError::OutputLimitExceeded
WasmError::Trap
```

Errors must not include raw host paths or secret material.

---

## 12. Minimum TDD coverage

Local contract tests should prove:

- valid module validates/prepares
- invalid module fails
- descriptor must use `RuntimeKind::Wasm`
- descriptor provider/capability must match the prepared module
- invocation requires a governor-issued active reservation guard
- exported function is invoked successfully
- JSON ABI writes input into guest memory and reads JSON output
- JSON ABI validates invocation input, including nested object properties, against the descriptor schema before guest execution
- guest non-zero status becomes a structured guest error with sanitized message text
- invalid guest JSON output fails closed
- ABI memory/allocator/output accessor exports are required
- output byte limit is enforced
- memory limit is enforced
- fuel limit stops a runaway module
- memory growth beyond the configured limit fails closed
- invalid timeout/epoch configurations fail closed
- epoch timeout interrupts runaway modules even when fuel is large
- cache setup errors do not leak raw host paths
- cached prepared modules reuse identical bytes and split changed content
- cached modules still instantiate fresh per invocation
- extension package root is revalidated before module loading
- extension package module assets are read via `RootFilesystem` virtual paths
- oversized manifest module assets are rejected before reading
- non-WASM package runtimes are rejected by the WASM lane
- undeclared capabilities are rejected before module preparation
- manifest-derived export mismatches are rejected before invocation
- executor reserves before preparation/invocation and reconciles successful usage
- executor releases reservations on preparation, invocation, or reconciliation failure
- resource-denied executions fail before module preparation/invocation
- core `host.log_utf8` captures bounded structured logs in capability results and cannot exceed the runtime output-byte budget
- core `host.time_unix_ms` is available without adding privileged authority
- filesystem imports read/write/list/stat through `ScopedFilesystem` and `MountView`
- filesystem imports deny by default when no filesystem context is provided
- filesystem write respects mount permissions and cannot create host-path access
- unsupported host imports still fail closed during module preparation
- invocation returns actual usage suitable for resource reconciliation
- no privileged host imports are available unless explicitly registered

---

## 13. Non-goals

Do not add in this first crate:

- broad WASI filesystem access or ambient preopens
- network host imports
- secret host imports
- owning extension discovery
- owning manifest parsing or registry validation
- Docker/script execution
- MCP client handling
- kernel dispatch
- marketplace behavior
- agent loop behavior
