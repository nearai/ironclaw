# Loop worker wire (sandbox membrane)

The private, same-build frame protocol between the loop host (in-process) and
an out-of-process loop worker (Rust canonical worker or the Pi worker) inside
a persistent per-user sandbox. Both sides are built from one commit; the wire
is **not** a public API. Declared in
`crates/loop/ironclaw_loop_host/src/remote_host/{protocol,server,client}.rs`;
re-exported from the `ironclaw_loop_host` crate root and from
`ironclaw_loop_host::remote_host`.

## Framing

- Transport is the worker's stdin/stdout pipes, owned by
  `ironclaw_sandbox`'s `SandboxLoopWorkerSession` (host side) and read by
  `remote_host_from_stdio` (worker side).
- Every frame: `u32` big-endian length prefix, then UTF-8 JSON.
- Frames larger than `LOOP_WORKER_MAX_FRAME_BYTES` (= 
  `ironclaw_host_api::process::MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES`, 1 MiB)
  fail on both encode and decode with
  `AgentLoopHostErrorKind::InvalidInvocation`.
- The host sends `HostFrame`s; the worker sends `WorkerFrame`s. Diagnostics
  go to stderr only.
- `wire_version` lives on the bootstrap; workers reject a bootstrap whose
  version they do not know. Current version: `2`.

## Version rules

- `LOOP_WORKER_WIRE_VERSION = 2`.
- v2 adds `LoopWorkerBootstrap.content_visibility`. An omitted field defaults
  to `Blind`. This default does not provide version compatibility: both
  workers reject a bootstrap with an unsupported version.
- v2 adds `HostCall::ResolveMessages`. A v1 host would answer it with a
  deserialization failure; a v2 host denies it with `PolicyDenied` unless the
  worker was bootstrapped `Resolved` (see below).

## Content visibility rule

| Bootstrap `content_visibility` | Worker may resolve message content | `HostCall::ResolveMessages` result |
|---|---|---|
| `blind` (default) | No | `WireError::Host`, `AgentLoopHostErrorKind::PolicyDenied`; the content port is never called |
| `resolved` and a `LoopMessageContentPort` is configured | Yes, for refs the run already holds | host-resolved role/content/tool-result text |
| `resolved` but no port configured | No | `WireError::Host`, `AgentLoopHostErrorKind::PolicyDenied` |

Only the canonical Rust worker boots `Blind`. The Pi worker boots `Resolved`:
it cannot own context or compaction on placeholders, so it resolves the
user's own thread content inside the user's own container. Secrets,
authorization, other tenants, and stores remain host-side; resolved content is
always the same resolver the model gateway uses, scoped to the run.

`ResolveMessages` uses a dedicated counter capped at the saturating sum of
the run profile's `max_model_calls` and `max_capability_invocations`.
Exceeding it returns `AgentLoopHostErrorKind::BudgetExceeded`.
The host tracks up to 4,096 issued `(reference, role)` pairs. Empty requests
are invalid. Unissued references and changed roles are denied before the
content port is called.

## Host → worker frames (`HostFrame`, externally tagged)

```json
{"Bootstrap": {
  "wire_version": 2,
  "run_context": { /* LoopRunContext */ },
  "invocation": {"Run": { /* AgentLoopDriverRunRequest */ }} | {"Resume": { /* AgentLoopDriverResumeRequest */ }},
  "settings": {"default_iteration_limit": null, "model_availability_attempts": null},
  "tool_definitions": [ /* ProviderToolDefinition */ ],
  "current_visible_capabilities": null,          // Option<WireVisibleCapabilitySurface> as JSON
  "content_visibility": "blind"                   // "blind" | "resolved", defaults to "blind"
}}
{"HostResponse": {"id": 7, "result": {"Ok": <any>} | {"Err": {"Host": {...}} | {"Compaction": {...}} | {"Protocol": "..."}}}}
{"Cancel": {"reason_kind": "...", "requested_at": "..."}}
"OutcomeAck"
```

`HostResponse.result` is serde's externally tagged `Result`:
success serializes as `{"Ok": ...}`, failure as `{"Err": ...}` where the
error is the externally tagged `WireError` enum (`{"Host": ...}` |
`{"Compaction": ...}` | `{"Protocol": "..."}`).

## Worker → host frames (`WorkerFrame`, externally tagged)

```json
{"HostRequest": {"id": 7, "call": { /* HostCall, below */ }}}
{"Outcome": {"Exit": { /* LoopExit */ }} | {"Failed": {"kind": "...", "detail": "..."}}}
```

## Host calls (`HostCall`, externally tagged; variant names PascalCase, fields snake_case)

| Variant | Payload | JSON shape |
|---|---|---|
| `ResolveMessages` | `ResolveMessagesRequest` | `{"ResolveMessages":{"messages":[{"role":"user","content_ref":"msg:..."}]}}` |
| `LoadContext` | `LoopContextRequest` | `{"LoadContext":{"after":...,"limit":...,"mode":"text_only"}}` |
| `BuildPrompt` | `LoopPromptBundleRequest` | `{"BuildPrompt":{...}}` |
| `PollInputs` | struct variant | `{"PollInputs":{"after":...,"limit":...}}` |
| `AckInputs` | tuple | `{"AckInputs":[...]}` |
| `StreamModel` | `LoopModelRequest` | `{"StreamModel":{"messages":[...],"inline_messages":[...],"iteration":...}}` |
| `RegisterProviderToolCall` | `RegisterProviderToolCallRequest` | `{"RegisterProviderToolCall":{"tool_call":{...}}}` |
| `VisibleCapabilities` | `VisibleCapabilityRequest` | `{"VisibleCapabilities":{...}}` |
| `InvokeCapability` | `LoopRequest` | `{"InvokeCapability":{"activity_id":...,"surface_version":...,"capability_id":...,"input_ref":...}}` |
| `InvokeCapabilityBatch` | `LoopRequestBatch` | `{"InvokeCapabilityBatch":{...}}` |
| `BeginAssistantDraft` | `BeginAssistantDraft` | `{"BeginAssistantDraft":{...}}` |
| `UpdateAssistantDraft` | `UpdateAssistantDraft` | `{"UpdateAssistantDraft":{"message_ref":"msg:...","reply":{"content":"..."}}}` |
| `FinalizeAssistantMessage` | `FinalizeAssistantMessage` | `{"FinalizeAssistantMessage":{...}}` |
| `AppendCapabilityResultRef` | boxed `AppendCapabilityResultRef` | `{"AppendCapabilityResultRef":{...}}` |
| `Checkpoint` | `LoopCheckpointRequest` | `{"Checkpoint":{"kind":"before_model","state_ref":"checkpoint:..."}}` |
| `StageCheckpointPayload` | `StageCheckpointPayloadRequest` | `{"StageCheckpointPayload":{...}}` |
| `LoadCheckpointPayload` | `LoadCheckpointPayloadRequest` | `{"LoadCheckpointPayload":{...}}` |
| `EmitProgress` | `LoopProgressEvent` | `{"EmitProgress":{...}}` |
| `Compact` | `LoopCompactionRequest` | `{"Compact":{...}}` |

Success responses are the call's natural response type serialized as JSON
(`LoadContext`, `LoadCheckpointPayload`, `VisibleCapabilities` go through
their wire mirrors — `WireLoopContextBundle`,
`WireLoadedCheckpointPayload`, `WireVisibleCapabilitySurface` — which keep
raw context/checkpoint bytes out of the public contracts tier).

### `ResolveMessages` response

`{"Ok":[ ... ]}` where each element is a `WireResolvedModelMessage`:

```json
{
  "role": "user",                          // "system" | "user" | "assistant" | "tool_result_reference"
  "content_ref": "msg:...",
  "content": "host-resolved model-visible text",
  "tool_result": {                          // Option, omitted when absent
    "provider_call_id": "call_1",           // Option, omitted when absent
    "content": "tool result text"
  }
}
```

The host maps each message through its `LoopMessageContentPort`
(`ironclaw_loop_contracts::host::model`), implemented by
`ThreadBackedLoopModelPort` (same resolver the model gateway uses). Role
strings mirror `HostManagedModelMessageRole`: `system`, `user`,
`assistant`, `tool_result_reference`. Tool-result content comes from the
result reference envelope's model-visible content (safe summary as
fallback); `Resolved`-variant tool results replay the message content.

The production run host exposes its content port through the optional
`LoopRunInfoPort::worker_content_port` accessor. The port shares the exact
instruction-materialization store used by that run's prompt builder. This
keeps identity and hook instruction references resolvable without a global
run registry. The `AgentLoopDriverHost` supertrait list is unchanged.

## Session lifecycle

1. Host → `Bootstrap`. Worker validates `wire_version` and reads
   `content_visibility` (absent ⇒ `Blind`).
2. Worker → `HostRequest`s; host answers each `HostResponse` with the same
   `id`. The host may inject `Cancel` at any point.
3. Worker → `Outcome` (`Exit` with a host-validated `LoopExit`, or `Failed`).
   For an `Exit`, the host replaces reported usage with host-observed usage
   and drops worker-authored failure summaries. A worker-level `Failed`
   becomes a driver error; it is not a successful run or a `LoopExit` claim.
4. Host → `OutcomeAck`. Worker exits 0.

## Trust boundary delta (vs #7908)

| Invariant | #7908 (wire v1) | This wire (v2) |
|---|---|---|
| Worker holds credentials, DB, authorizer, Docker | No | No |
| Model call crosses the host (`StreamModel`) | Yes | Yes, unchanged |
| Every capability call crosses host authorization | Yes | Yes, unchanged |
| Worker sees transcript text | No | Only when bootstrapped `Resolved`; only refs the host already issued to this run; denied otherwise with `PolicyDenied` |
| Worker can author prompt text | Via `BuildPrompt.inline_messages` (already) | Same mechanism; no new write path |
| Host validates `LoopExit` | Yes | Yes |
| Wire is private, same-build | Yes | Frame types are `pub`, documented, `wire_version` 2; still not a public API |

## Related

- Plan: `docs/internal/reborn/2026-09-pi-loop-worker-plan.md`
- Host port: `LoopMessageContentPort` in `ironclaw_loop_contracts::host::model`
- Session/transport: `ironclaw_sandbox::sandbox_process::loop_worker`
