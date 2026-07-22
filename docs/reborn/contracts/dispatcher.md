# IronClaw Reborn dispatcher contract

Date: 2026-04-24
Status: V1 contract slice
Crate: `crates/ironclaw_dispatcher`

---

## 1. Purpose

`ironclaw_dispatcher` is the composition-only runtime dispatch layer for Reborn.

It connects already-validated extension capabilities to prebound runtime
bindings:

```text
ToolResolver + ResourceGovernor
  -> RuntimeDispatcher::dispatch_json(Authorized)
  -> resolved BoundCapabilityAdapter
  -> normalized CapabilityDispatchResult
```

The dispatcher does not discover extensions, parse manifests, implement policy,
open files directly, resolve secrets, or execute product workflows. Binding
construction happens before dispatch in resolver owners such as
`ironclaw_host_runtime` and `ironclaw_extension_host`; the dispatcher consumes a
sealed `Authorized` witness, resolves the prebound binding by capability id, and
fails closed when the resolved runtime does not match the sealed lane.

The dispatch port contracts live in `ironclaw_host_api`:

```rust
Authorized
CapabilityDispatchRequest
CapabilityDispatchResult
CapabilityDispatcher
DispatchError
RuntimeDispatchErrorKind
```

`ironclaw_dispatcher` implements that neutral port. Higher-level workflow crates such as `ironclaw_capabilities` depend on `ironclaw_host_api`, not on the concrete dispatcher crate in production code.

---

## 2. Inputs

The dispatcher receives an already-authorized sealed `Authorized` witness:

```rust
pub struct Authorized {
    /* private: sealed invocation + RuntimeLane + mounts + reservation + deadline */
}

pub trait CapabilityDispatcher {
    async fn dispatch_json(
        &self,
        authorized: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError>;
}
```

The dispatcher unpacks the witness at execution time, rejects expired witnesses,
derives the internal `CapabilityDispatchRequest` handed to the binding, resolves
the capability through `ToolResolver`, and verifies that
`RuntimeLane::from_runtime_kind(resolved.runtime)` matches the sealed lane before
any backend call. `RuntimeKind::System` maps to no lane and is therefore rejected
before binding execution.

The dispatcher can be constructed from borrowed service boundaries for request-scoped composition:

```rust
RuntimeDispatcher::new(&tool_resolver, &resource_governor)
    .with_event_sink(&event_sink)
```

For detached background execution, it can also own shared service handles:

```rust
RuntimeDispatcher::from_arcs(tool_resolver, resource_governor)
    .with_event_sink_arc(event_sink)
```

The owned form keeps dispatcher composition-only while allowing detached
processes to run capability-backed work without leaking borrowed app state into
a spawned task.

`ToolResolver` remains the authority for what can run. Runtime adapter owners
remain the authority for how a lane runs. The concrete WASM, Script, MCP, and
first-party adapters live outside `ironclaw_dispatcher`, so this crate has no
normal dependencies on concrete runtime crates.

Implementation evidence:

- `crates/ironclaw_dispatcher/src/lib.rs` defines `RuntimeDispatcher`,
  `ToolResolver`, and `BoundCapabilityAdapter`.
- `crates/ironclaw_host_api/src/dispatch.rs` defines the sealed
  `CapabilityDispatcher::dispatch_json(Authorized)` port and internal
  `CapabilityDispatchRequest`.
- `crates/ironclaw_dispatcher/tests/dispatch_contract.rs` covers sealed-lane
  mismatch rejection, `None` mount preservation, resolver misses, and prepared
  reservation validation.

---

## 3. Dispatch algorithm

V1 `dispatch_json` performs only routing and consistency checks:

```text
1. consume the sealed `Authorized` witness and reject expired witnesses
2. derive the internal adapter request from the witness parts
3. resolve a prebound binding by capability id through `ToolResolver`
4. re-derive `RuntimeLane` from the resolved runtime and compare it to the sealed lane
5. validate any prepared `ResourceReservation` before binding execution
6. call the resolved `BoundCapabilityAdapter`, forwarding actor, mounts, run id, estimate, reservation, and input unchanged
7. return normalized result or typed failure with a stable redacted `RuntimeDispatchErrorKind`
```

`BoundCapabilityAdapter` is the open extension seam:

```rust
#[async_trait]
pub trait BoundCapabilityAdapter {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}
```

Each runtime adapter owns its local reserve/prepare/invoke/reconcile/release lifecycle. The dispatcher validates a prepared reservation is still active before adapter execution, but it does not own the full resource-governor protocol and does not import concrete runtime crates.

`CapabilityDispatchRequest.authenticated_actor_user_id` is copied directly from
the already-authorized dispatch request. It is not recomputed from
`ResourceScope.user_id`; a shared subject may be acted on by a separately
authenticated human actor.

---

## 4. Runtime lane status

V1 routes by the prebound adapter returned by `ToolResolver`, after rechecking
that the resolved runtime maps to the sealed `RuntimeLane` carried by
`Authorized`:

| Runtime kind | Dispatch behavior |
| --- | --- |
| `Wasm` | Resolved runtime must map to sealed `RuntimeLane::Wasm`; executes through the resolved adapter, usually composed by `ironclaw_host_runtime` |
| `Script` | Resolved runtime must map to sealed `RuntimeLane::Process`; executes through the resolved adapter, usually composed by `ironclaw_host_runtime` |
| `Mcp` | Resolved runtime must map to sealed `RuntimeLane::Mcp`; executes through the resolved adapter, usually composed by `ironclaw_host_runtime` |
| `FirstParty` | Resolved runtime must map to sealed `RuntimeLane::FirstParty`; requires a registered host-service adapter |
| `System` | Rejected as `MissingRuntimeBackend` before backend calls |

If the capability id has no resolved binding, dispatch returns
`UnknownCapability` before adapter execution.

Runtime-specific failures are collapsed to stable categories (`Backend`, `ExitFailure`, `OutputDecode`, `Resource`, and similar) before crossing the dispatch port. Raw backend strings, stderr, host paths, and internal runtime detail strings stay inside the runtime crate.

---

## 5. Fail-closed rules

The dispatcher fails before execution when:

- capability ID is not registered
- resolved runtime does not map to the sealed lane
- prepared reservation validation fails
- selected binding returns a typed dispatch failure

Configured event sink failures are not dispatch failures. Event emission is best-effort observability and must not alter the success value or mask the original runtime/control-plane error.

These failures must not reserve resources or perform external effects. If a caller supplies a prepared `ResourceReservation` from obligation handling and dispatcher validation fails before a runtime adapter takes ownership, the dispatcher releases that reservation before returning the failure so pre-dispatch handoff cannot leak budget.

---

## 6. Result shape

A successful dispatch returns a normalized result:

```rust
pub struct CapabilityDispatchResult {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub output: serde_json::Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
}
```

The shape intentionally exposes common host-level facts and avoids leaking WASM-specific internals as the generic contract.
`display_preview` is an optional, model-hidden presentation side channel for already-sanitizable UI material such as unified diffs; callers must keep the canonical capability output in `output`.

---

## 7. Non-goals

This PR does not add:

- authorization/grant evaluation
- approval prompts
- full audit/event projection persistence
- script filesystem mounts, artifact export, network access, or secret injection
- MCP protocol handshake/lifecycle management beyond a resolved binding contract
- host service dispatch for first-party/system capabilities
- filesystem mount selection
- network or secret injection
- background `spawn` / process lifecycle
- agent-loop behavior

Those belong in dedicated service crates or later narrow dispatcher composition slices.

---

## 8. Contract tests

The crate test suite covers:

- WASM capability dispatch through a resolved adapter
- unknown capability failure before resource reservation
- sealed lane mismatch failure before execution
- Script capability dispatch through a resolved adapter
- MCP capability dispatch through a resolved adapter
- first-party and system lane behavior at the resolver/binding seam
- event sink failures ignored on both success and failure paths
- runtime failure details redacted to `RuntimeDispatchErrorKind`

These tests are intentionally caller-level: they drive `RuntimeDispatcher::dispatch_json`, not only helper functions.


---

## Contract freeze addendum — first-class runtime lanes (2026-04-25)

WASM, Script, and MCP are all first-class V1 runtime lanes.

`ironclaw_dispatcher` still remains an already-authorized router only. It must not take dependencies on authorization, approvals, run-state, memory, secrets, network workflow, process lifecycle, or concrete host-runtime composition. Runtime lanes are registered through `RuntimeAdapter`.

Because Script and MCP are first-class, their adapters must satisfy the same redaction, resource, process, event, and network-enforcement contracts as WASM. If a required obligation cannot be enforced for a lane, that invocation fails closed before dispatch.
