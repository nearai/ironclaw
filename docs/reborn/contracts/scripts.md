# IronClaw Reborn script runner contract

**Date:** 2026-04-25
**Status:** V1 contract slice
**Crate:** `crates/ironclaw_scripts`
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/extensions.md`, `docs/reborn/contracts/resources.md`, `docs/reborn/contracts/dispatcher.md`

---

## 1. Purpose

`ironclaw_scripts` provides the native CLI/software execution lane without requiring every useful tool to be rebuilt in WASM.

The public runtime kind is:

```text
RuntimeKind::Script
```

The V1 Docker/container executor remains available as an implementation backend, but Docker is not the public script contract. Extension manifests declare a semantic runner profile plus command metadata; the host selects the concrete backend for that runner. Manifests do not receive raw Docker flags, host paths, ambient environment, secrets, or network access by default.

---

## 2. Current V1 API

The crate exposes:

```rust
pub struct ScriptRuntime<B: ScriptBackend>;
pub struct ScriptRuntimeConfig;
pub trait ScriptBackend;
pub trait ScriptExecutor;
pub struct DockerScriptBackend;

pub struct ScriptExecutionRequest<'a>;
pub struct ScriptExecutionResult;
pub struct ScriptInvocation;
pub struct ScriptBackendRequest;
pub struct ScriptBackendOutput;
pub enum ScriptError;
```

The dispatcher composes a script runtime with:

```rust
RuntimeDispatcher::new(&registry, &fs, &governor)
    .with_script_runtime(&script_runtime)
```

and dispatches script capabilities through the same `dispatch_json` entry point as WASM capabilities.

---

## 3. Manifest-derived command contract

Script command metadata comes from a validated extension manifest:

```toml
[runtime]
kind = "script"
runner = "sandboxed_process"
command = "script-echo"
args = ["--json"]
```

At execution time, the runtime builds a `ScriptBackendRequest` from the manifest and the invocation:

```text
provider
capability_id
scope
runner
image (optional, only for Docker-backed runners)
command
args
stdin_json
```

Rules:

- runner/command/args come from the manifest, not model/user input
- invocation input is serialized as JSON and passed through stdin
- backend receives normalized runner fields, not raw Docker flags
- capability IDs must be declared by the package
- descriptor runtime must match package runtime
- legacy `backend = "docker"` manifests remain accepted for the optional Docker backend, but new manifests should use `runner = "sandboxed_process"` or another host-defined semantic runner profile

---

## 4. Resource lifecycle

The script runtime owns the script lane reserve/execute/reconcile/release protocol:

```text
validate package/capability/runtime
-> reserve(scope, estimate)
-> backend.execute(request)
-> enforce output limits
-> parse stdout JSON
-> reconcile(reservation_id, actual_usage)
```

Failure cleanup:

```text
validation fails before reserve -> no reservation
reserve fails -> no backend call
backend fails -> release reservation
non-zero exit -> release reservation
output limit fails -> release reservation
invalid JSON stdout -> release reservation
success -> reconcile reservation
```

Actual usage currently records:

- wall-clock milliseconds reported by the backend
- stdout bytes
- one process count per successful backend execution

---

## 5. Optional Docker backend posture

`DockerScriptBackend` invokes Docker only when the manifest resolves to `runner = "docker"` with an image. It uses normalized fields only:

```text
docker run --rm -i --network none <image> <command> <args...>
```

V1 default restrictions:

- no host path mounts
- no host environment forwarding
- no Docker socket exposure to extensions
- network disabled by default
- JSON input over stdin
- stdout is bounded and parsed as JSON
- stderr is bounded before surfacing in errors

Future PRs may add scoped filesystem mounts, artifact export, network policy, and secret-handle injection. Those must be explicit host-mediated additions, not ambient Docker options.

---

## 6. Dispatcher relationship

`ironclaw_dispatcher` selects the script lane when a declared capability has:

```text
RuntimeKind::Script
```

The dispatcher does not execute Docker itself. It calls the configured `ScriptExecutor`.

Fail-closed behavior:

- script capability with no configured script runtime -> `MissingRuntimeBackend { runtime: Script }`
- non-script runtime lanes remain unsupported until their crates land
- unknown capabilities and descriptor/package runtime mismatches fail before reservation

---

## 7. Non-goals

This contract does not add:

- arbitrary host shell access
- host filesystem mounts
- artifact export
- secret injection
- network access
- MCP protocol handling
- long-running process lifecycle
- product workflows

Those are separate slices.

---

## 8. Tests

Current contract tests cover:

- successful script execution reserves, invokes backend, parses stdout JSON, and reconciles
- budget denial prevents backend execution
- backend non-zero exit releases reservation
- output limit failure releases reservation
- non-script packages are rejected before reserving
- undeclared capabilities are rejected before reserving
- runner/command metadata comes from the manifest, not invocation input
- dispatcher routes script capabilities through a configured `ScriptExecutor`
- dispatcher fails before reservation when script backend is missing
