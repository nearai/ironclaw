# WASM Hook Runtime Threat Model

## Scope

This document covers Installed-tier hooks whose manifest body is
`HookManifestBody::Wasm { export, budget }`. Builtin and Trusted WASM
hooks are out of scope and are not loadable through this path.

The runtime executes a core WebAssembly module inside wasmtime. The
registrar resolves installed module bytes, compiles them, derives a
module-digest-pinned `HookId`, and installs a typed dispatcher wrapper.
Every invocation creates a fresh `wasmtime::Store`; no guest linear
memory, globals, tables, or fuel state are reused across invocations.

## Boundary

The only guest-to-host boundary is a `wasmtime::Linker` surface. WASM
hooks receive no WASI, filesystem, network, environment, wall-clock,
randomness, process, or secret imports. Guest memory is read only by
host imports that explicitly accept `(ptr, len)` pairs and immediately
copy UTF-8 into host-owned strings.

The hook runtime reuses the tool-WASM resource-limiter primitive:
`WasmResourceLimiter` from `crates/ironclaw_wasm/src/limiter.rs`. The
hook runtime also follows the tool-WASM per-call sandbox pattern:
wasmtime engine-level fuel/epoch support plus a fresh store with a
store limiter for each call.

## Host Import Surface

### `ic:hooks/before-capability@1`

These imports are available only to `before_capability` hooks:

| Import | Signature | Budget | Effect |
| --- | --- | --- | --- |
| `deny` | `(reason_code: i32) -> i32` | sink call + decision call | Mints a restricted Deny decision. |
| `pause_approval` | `(reason_code: i32) -> i32` | sink call + decision call | Mints a restricted approval pause. |
| `pause_auth` | `(reason_code: i32) -> i32` | sink call + decision call | Mints a restricted auth pause. |
| `pass` | `() -> i32` | sink call + decision call | Declares no opinion. |

Decision-call budget is exactly 1. A second decision call returns a
rejected status and marks the invocation `FailureCategory::Malformed`.
Installed WASM cannot mint Allow because no Allow import exists.

### `ic:hooks/before-prompt@1`

These imports are available only to `before_prompt` hooks:

| Import | Signature | Budget | Effect |
| --- | --- | --- | --- |
| `add_envelope_snippet` | `(ptr: i32, len: i32, ordinal: i32) -> i32` | sink call + total patch bytes | Copies a UTF-8 snippet from guest memory and emits an Installed-tier prompt patch. |
| `add_milestone_metadata` | `(key: i32, ptr: i32, len: i32) -> i32` | sink call | Copies a UTF-8 metadata value from guest memory and emits a milestone metadata patch. |

Total emitted snippet bytes are capped at 4 KiB per invocation, matching
the existing prompt envelope budget. Invalid pointers, invalid UTF-8,
invalid ordinal codes, invalid metadata keys, or any budget overflow
mark the invocation `FailureCategory::Malformed`.

### `ic:hooks/observer@1`

These imports are available only to observer hooks:

| Import | Signature | Budget | Effect |
| --- | --- | --- | --- |
| `note` | `(category: i32, summary: i32) -> i32` | sink call + observer fact | Emits a sanitized observer fact from fixed code tables. |

Observer facts are capped at 32 per invocation. Unknown category or
summary codes do not expose guest memory; category must be valid and
summary falls back to a fixed sanitized string.

## Budgets

| Budget | Value | Enforced By |
| --- | ---: | --- |
| Fuel | manifest `WasmBudget.fuel`, default 100,000 | wasmtime fuel metering |
| Memory | manifest `WasmBudget.memory_mb`, default 4 MiB | `WasmResourceLimiter` |
| Wall-clock | manifest `WasmBudget.wall_ms`, default 50 ms | wasmtime epoch deadline + dispatcher timeout |
| Sink calls | 64 per invocation | host import shim |
| Prompt patch bytes | 4 KiB per invocation | host import shim |
| Observer facts | 32 per invocation | host import shim |
| Gate decisions | 1 per invocation | host import shim |

## Failure Mapping

| Failure | FailureCategory | Gate / Mutator Disposition | Observer Disposition |
| --- | --- | --- | --- |
| Guest trap, fuel exhaustion, or memory exhaustion trap | `Panic` | `FailClosed` | `FailIsolated` |
| Wall-clock / epoch deadline timeout | `Timeout` | `FailClosed` | `FailIsolated` |
| Unsupported import / link mismatch | `Malformed` | `FailClosed` | `FailIsolated` |
| Missing export or wrong export type | `Malformed` | `FailClosed` | `FailIsolated` |
| Missing gate decision | `Malformed` | `FailClosed` | `FailIsolated` |
| Host-import budget overflow | `Malformed` | `FailClosed` | `FailIsolated` |
| Invalid guest pointer, invalid UTF-8, or invalid enum code | `Malformed` | `FailClosed` | `FailIsolated` |
| Host wrapper panic | `Panic` | `FailClosed` | `FailIsolated` |

The dispatcher applies the existing failure-policy matrix and poisons
the hook slot for the rest of the run when any failure is recorded.

## Module Substitution

The registrar resolves installed module bytes before registration and
computes a BLAKE3 digest over the byte string. It reuses
`HookId::derive` and folds the module digest into the version material
passed to that existing constructor. Replacing the module while keeping
the same extension id, extension version, local hook id, and hook
version therefore produces a different `HookId`.

Registry insertion does not see module substitution as a collision.
The durable safety property is at checkpoint replay: a checkpoint pinned
to `HookId_A` refuses against a registry containing only `HookId_B`
with `UnknownHook` / `unknown_hook_id_at_replay`.

## Side Channels

The runtime removes ambient time, randomness, filesystem, network,
process, and secret channels. Residual side channels remain:

- Execution time can reveal bounded predicate work despite fuel and
  wall-clock caps.
- Memory growth patterns are visible to host tracing at aggregate
  granularity.
- Host-import call counts are observable through failure categories and
  audit milestones.

These are accepted for v1 because Installed WASM hooks are already
extension-authored policy code and receive only sanitized hook context.
Future richer context imports must update this model before landing.

## Findings Covered

1. W1: no ambient host authority crosses the wasmtime boundary.
2. W2: fuel, memory, and wall-clock exhaustion map through the existing
   failure-policy matrix.
3. W3: host-import calls and outputs have independent sink-side budgets.
4. W4: Installed WASM cannot mint Allow because no host import exposes it.
5. W5: module substitution is detected at checkpoint replay through
   digest-pinned `HookId` derivation.
6. W6: residual side channels are documented and bounded for v1.
