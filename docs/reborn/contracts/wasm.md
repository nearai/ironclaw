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
- invocation carries an active resource reservation

The runtime must not accept arbitrary capability IDs or extension IDs that bypass extension registry validation.

---

## 4. Resource protocol

Before invoking a WASM capability:

```text
ironclaw_resources.reserve(scope, estimate)
```

Then:

```text
ironclaw_wasm.invoke(..., reservation, ...)
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

The existing IronClaw codebase already uses Wasmtime with fuel, epoch interruption, and fresh instantiation patterns. Reborn should reuse those lessons but keep this crate narrower and contract-driven.

---

## 6. Host imports

Initial V1 may expose no privileged imports beyond the minimal test/demo ABI.

Future imports should be grouped by service:

```text
host.fs.read/write/list/stat
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

## 7. Minimum V1 API sketch

```rust
pub struct WasmRuntime;

pub struct WasmModuleSpec {
    pub provider: ExtensionId,
    pub capability: CapabilityId,
    pub export: String,
    pub bytes: Vec<u8>,
}

impl WasmRuntime {
    pub fn prepare(&self, spec: WasmModuleSpec) -> Result<PreparedWasmModule, WasmError>;
    pub fn invoke_i32(
        &self,
        module: &PreparedWasmModule,
        descriptor: &CapabilityDescriptor,
        reservation: &ResourceReservation,
        input: i32,
    ) -> Result<WasmInvocationResult<i32>, WasmError>;
}
```

The initial `invoke_i32` shape is only a tiny vertical-slice ABI. It can later be replaced or supplemented by component-model/WIT invocation once the host ABI is locked.

---

## 8. Error contract

Minimum errors:

```rust
WasmError::InvalidModule
WasmError::DescriptorMismatch
WasmError::MissingReservation
WasmError::MissingExport
WasmError::FuelExhausted
WasmError::OutputLimitExceeded
WasmError::Trap
```

Errors must not include raw host paths or secret material.

---

## 9. Minimum TDD coverage

Local contract tests should prove:

- valid module validates/prepares
- invalid module fails
- descriptor must use `RuntimeKind::Wasm`
- descriptor provider/capability must match the prepared module
- invocation requires a reservation
- exported function is invoked successfully
- output byte limit is enforced
- fuel limit stops a runaway module
- invocation returns actual usage suitable for resource reconciliation
- no privileged host imports are available unless explicitly registered

---

## 10. Non-goals

Do not add in this first crate:

- full WASI filesystem access
- network host imports
- secret host imports
- extension discovery
- manifest parsing
- Docker/script execution
- MCP client handling
- kernel dispatch
- marketplace behavior
- agent loop behavior
