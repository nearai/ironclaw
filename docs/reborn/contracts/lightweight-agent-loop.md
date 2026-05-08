# IronClaw Reborn lightweight agent loop contract

**Date:** 2026-04-26
**Status:** Decision guide / reference loop contract
**Inspired by:** `badlogic/pi-mono/packages/agent` loop mechanics
**Depends on:** `docs/reborn/contracts/agent-loop-protocol.md`, `docs/reborn/contracts/runtime-workflows.md`, `docs/reborn/contracts/capabilities.md`, `docs/reborn/contracts/run-state.md`, `docs/reborn/contracts/approvals.md`, `docs/reborn/contracts/events.md`, `docs/reborn/contracts/runtime-selection.md`, `docs/reborn/contracts/runtime-profiles.md`

---

## 1. Purpose

Define the default shipped lightweight agent loop for Reborn as a reference loop implementation, not as kernel behavior.

This loop borrows the useful mechanics of the `pi-mono` agent loop:

```text
stream assistant
-> execute tool/capability calls
-> append tool results
-> repeat until reply, blocked, failed, or interrupted
```

But it is not a dependency on `pi-mono`, and it does not import `pi-mono` authority semantics. It is a Reborn-native loop that runs inside IronClaw host contracts.

The loop may be packaged as a bundled `agent_loop` extension or implemented in a shipped reference-loop crate. In both cases, it receives only a narrow kernel-mediated host facade, never raw service managers, and never a bypass around `CapabilityHost`/policy checks.

---

## 2. Core invariant

```text
The lightweight loop decides when to ask the model and when to request visible capabilities.
The host decides whether and how those capabilities execute.
```

The loop must not bypass:

```text
CapabilityHost
RunStateManager
ConversationManager
ApprovalManager
AuthFlowManager
RuntimeDispatcher
EventStreamManager
ResourceGovernor
```

The loop is the parent-loop mechanic, not the authority/runtime layer. Being shipped by the project may affect its trust ceiling through host policy, but it does not grant authority by itself.

---

## 3. Parent protocol

The host-facing parent protocol remains:

```text
Reply | CapabilityCalls
```

Provider-native tool calling can encode `CapabilityCalls`, but the loop must normalize provider tool calls into IronClaw capability calls before execution.

The following are not top-level parent protocol branches:

```text
CodeAct
QuickJS
script
shell
job
subagent
experiment
```

They are explicit capabilities such as:

```text
action_script.run(...)
script.run(...)
experiment.exec(...)
spawn_subagent(...)
create_job(...)
```

---

## 4. Loop mechanics

The intended mechanics are:

```text
begin run
load durable thread snapshot
build instruction bundle
load visible capability surface

while run is active:
  process queued steering/follow-up messages
  stream one assistant response
  persist assistant milestone when finalized

  if response is Reply:
    complete run
    stop

  if response has CapabilityCalls:
    execute call batch sequentially or in parallel according to policy
    append capability-result messages
    continue

  if a call requires approval/auth/resource wait:
    checkpoint loop
    block run
    stop until resumed

  if interrupted/cancelled/failed:
    transition run state
    stop
```

Equivalent pseudocode:

```rust
loop {
    let pending = host.take_pending_messages(run).await?;
    context.append(pending);

    let bundle = host.build_instruction_bundle(run).await?;
    let surface = host.visible_capabilities(run).await?;

    let assistant = loop_impl.stream_assistant(context, bundle, surface).await?;
    host.append_milestone(assistant.clone()).await?;

    match assistant.output {
        Reply(reply) => {
            host.complete_run(reply).await?;
            break;
        }
        CapabilityCalls(calls) => {
            let results = loop_impl.execute_batch(run, calls).await?;
            context.append(results.messages);
            host.checkpoint(run, context.summary()).await?;
        }
    }
}
```

---

## 5. Host facade

The loop receives an `AgentLoopHost` facade, not raw managers:

```rust
pub trait AgentLoopHost {
    async fn load_thread_snapshot(&self, run: RunHandle) -> Result<ThreadSnapshot>;
    async fn build_instruction_bundle(&self, run: RunHandle) -> Result<InstructionBundle>;
    async fn visible_capabilities(&self, run: RunHandle) -> Result<VisibleCapabilitySurface>;
    async fn stream_model(&self, request: ModelStreamRequest) -> Result<ModelStream>;

    async fn invoke_capability(&self, request: CapabilityInvocation) -> Result<CapabilityOutcome>;

    async fn append_milestone(&self, milestone: TranscriptMilestone) -> Result<()>;
    async fn publish_event(&self, event: RuntimeEvent) -> Result<()>;
    async fn checkpoint(&self, checkpoint: LoopCheckpoint) -> Result<()>;

    async fn block_run(&self, blocked: BlockedRun) -> Result<()>;
    async fn complete_run(&self, output: LoopOutput) -> Result<()>;
    async fn fail_run(&self, error: LoopError) -> Result<()>;
}
```

This facade composes lower-level services but does not move ownership into the loop.

---

## 6. Capability tool wrappers

Visible capabilities become model-visible tool schemas for the current run.

```rust
CapabilityDescriptor
  -> model-visible name
  -> description
  -> input schema
  -> concurrency policy
  -> result shaping hints
```

Tool execution is a wrapper around `CapabilityHost`:

```text
model tool call
  -> normalize to CapabilityInvocation
  -> CapabilityHost.invoke_json(...)
  -> CapabilityAccessManager action-time authorization
  -> Approval/Auth/Resource gates if needed
  -> RuntimeDispatcher.dispatch_json(...)
  -> capability result
  -> toolResult/capability-result message
```

The loop must not call `RuntimeDispatcher` directly. `RuntimeDispatcher` only receives already-authorized invocations.

---

## 7. Batch execution

The loop may execute a batch sequentially or in parallel.

Rules:

- host capability policy may force a call or whole batch to sequential execution
- each call is authorized independently
- parallel batch execution is not batch authorization
- approval prompts should preserve source order when multiple calls need approval
- filesystem writes, shell/process calls, and other exclusive resources may be sequential by descriptor/profile policy
- result messages should be appended in assistant source order unless a later contract explicitly chooses completion order

Suggested selection:

```text
if any call requires exclusive/sequential execution:
  execute whole batch sequentially
else:
  preflight/authorize each call and execute approved calls concurrently
```

---

## 8. Suspension and resume

The shipped reference loop must support structured suspension. Approval/auth/resource waits are not normal tool errors.

Capability outcomes:

```rust
pub enum CapabilityOutcome {
    Completed(CapabilityResult),
    ApprovalRequired(ApprovalGate),
    AuthRequired(AuthGate),
    ResourceBlocked(ResourceGate),
    Failed(CapabilityError),
}
```

On `ApprovalRequired`:

```text
checkpoint loop state
-> ApprovalManager.open_pending_gate
-> RunStateManager.blocked(approval)
-> EventStreamManager.publish(approval_needed)
-> stop loop until resume
```

On `AuthRequired`:

```text
checkpoint loop state
-> AuthFlowManager.begin
-> RunStateManager.blocked(auth)
-> TransportAdapter presents auth flow
-> stop loop until secret lease/auth completion
```

On resume:

```text
RunStateManager.resume
-> reload checkpoint and durable transcript snapshot
-> rebuild instruction/capability surface
-> continue or replay the pending invocation using idempotent invocation fingerprinting
```

MVP local-only implementations may block a promise during short approvals, but hosted and durable sessions require explicit suspension.

---

## 9. Working context vs durable transcript

The loop may keep an in-memory working context, similar to `AgentMessage[]`, but it is not the source of truth.

```text
working context = turn-local projection
ConversationManager = durable transcript source of truth
RunStateManager = run lifecycle source of truth
EventStreamManager = realtime delivery source
ProjectionReducer = derived read models
```

Persistence guidance:

| Loop event | Durable behavior |
|---|---|
| assistant stream start/update | live event only, optional ephemeral partial |
| assistant finalized | transcript milestone |
| capability batch start | run/event milestone |
| capability call start/update/end | capability audit + live progress |
| capability result message | transcript milestone |
| turn boundary | checkpoint/milestone |
| blocked approval/auth/resource | run-state transition + event |
| final reply | transcript milestone + run complete |

Realtime stream loss must not corrupt durable transcript state.

---

## 10. Steering and follow-up queues

The loop may support two queues:

- **steering messages**: injected before the next assistant response while the loop is active
- **follow-up messages**: consumed after the loop would otherwise stop, causing another assistant turn

These queues are host-owned inputs to the loop. They must preserve scope and ordering and must not bypass run-state rules.

```text
active run receives steering
  -> append as pending message before next model call

agent would stop, follow-up exists
  -> append follow-up and continue
```

Remote/hosted deployments must still honor one-active-run-per-thread and transport authorization.

---

## 11. Dynamic capability surfaces

The visible capability surface may change after extension activation, auth completion, grant changes, or profile changes.

The loop should request a versioned capability surface before each model call:

```text
visible_capabilities(run) -> { version, descriptors }
```

If the version changes, the loop regenerates model-visible tool schemas. The loop does not discover extensions directly.

Action-time authorization is still required even for visible capabilities.

---

## 12. Affordance-only selection and visible-surface gating

The loop should avoid hidden strategy heuristics such as:

```text
if estimated tool calls >= 5, force ActionScript
if task mentions tests, force shell
```

Instead, the host should shape the model-visible surface before each model call. The model chooses from clear semantic affordances, and the host executes or blocks those requests through normal capability policy.

```text
visible surface = CapabilityCatalog
  filtered by DeploymentMode
  filtered by RuntimeProfile
  filtered by tenant/org/user/project grants
  filtered by auth/installation state
  filtered by run/thread policy
  rendered as LlmToolViews
```

If a capability is impossible or categorically disallowed for the caller/profile, it should usually be absent from the visible surface rather than exposed and denied repeatedly at call time. Examples:

| Context | Hide or expose? | Reason |
|---|---|---|
| hosted multi-tenant session | hide `LocalHost` shell/file capabilities | provider host access is never valid |
| local safe profile before write approval | expose write capability with ask policy | user may approve writes |
| no GitHub extension installed | hide GitHub provider capabilities or expose install/auth capability only | avoid pointless API retries |
| GitHub installed but token expired | expose GitHub capability as auth-blockable if auth flow can resume | model can request semantic action; host opens auth gate |
| ActionScript disabled by tenant policy | hide `action_script.run` | no amount of retrying can make it valid |
| Experiment sandbox unavailable due quota | expose only if resource-blocked resume is supported; otherwise hide or ask user | avoid loop churn |

The visible surface is therefore a UX and reasoning aid, not an authorization shortcut. Every visible capability still receives action-time authorization because:

- grants/leases may expire between prompt and call
- parameters affect risk and approval requirements
- resource quotas may change
- auth may be missing or revoked
- concurrent runs may consume shared limits
- profile or tenant policy may change before execution

The model may choose `action_script.run`, `shell.run`, or `experiment.*` when those affordances are visible. It does not choose the runtime backend. Backend selection remains host/profile owned.

Use structured denial as feedback only for choices that were visible but rejected due to parameter-specific or time-varying conditions. Do not rely on denial/retry loops for static profile constraints.

---

## 13. Relationship to QuickJS / ActionScript

QuickJS is not the parent loop. It is a capability that the lightweight loop can call when dynamic capability composition is useful.

```text
lightweight loop
  -> action_script.run(code, allowed_capabilities)
  -> QuickJS executes real JS with no ambient fs/net/env/process
  -> QuickJS ic.call(...)
  -> CapabilityHost for every internal call
```

Use `action_script.run` for:

- loops
- fan-out/fan-in
- pagination
- filtering/sorting/grouping
- structured JSON transformation
- dynamic calls based on previous results

Use direct capability calls for simple/static tool use. Use `experiment.*` or `script.run` for shell/package/build/test work.

---

## 14. Relationship to runtime profiles

The same lightweight loop runs under every profile.

Examples:

```text
LocalDev profile:
  filesystem.read/write -> HostWorkspace
  shell.run             -> LocalHost

HostedMultiTenant profile:
  filesystem.read/write -> tenant workspace
  shell.run             -> tenant-scoped sandbox

EnterpriseDedicated profile:
  filesystem.read/write -> org-dedicated workspace
  shell.run             -> org-dedicated runner/container/VM
```

The loop only sees visible capability descriptors. The profile resolver and runtime backends decide where/how they execute.

---

## 15. Reference loop extension posture

This loop may be bundled with first-party package metadata, but the metadata is not authority:

```text
extension role: agent_loop
trust ceiling: assigned by host policy
host surface: AgentLoopHost facade only
```

Generated extensions cannot create new parent-loop authority surfaces. They may provide capabilities that this loop can call, subject to normal capability registration, grants, approvals, and runtime dispatch.

`ironclaw_extensions` may register bundled package metadata if useful, but it must not execute the loop. Loop execution belongs to the configured loop runner/service that owns the `AgentLoopHost` facade and remains subject to kernel-mediated policy.

---

## 16. Minimal implementation targets

The first implementation should include:

- provider-agnostic `Reply | CapabilityCalls` normalization
- streaming assistant message events
- capability wrapper generation from visible descriptors
- sequential/parallel batch execution
- structured approval/auth/resource suspension
- checkpoints at assistant-finalized, batch-start, result-appended, and blocked states
- steering and follow-up queue hooks
- event mapping into `EventStreamManager`
- durable milestone writes through `ConversationManager`
- runtime-profile-agnostic behavior

It should not include:

- direct filesystem/shell/HTTP calls
- extension discovery
- secret resolution
- runtime dispatch bypass
- product-specific coding-agent behavior beyond instruction/tool selection
- QuickJS embedded as parent protocol branch

---

## 17. Contract tests to add later

When implemented, add caller-level tests that drive the loop through the host facade:

- final reply completes run without side effects
- provider-native tool call normalizes into a capability invocation
- visible capability still receives action-time authorization
- sequential capability forces sequential batch execution
- parallel read-only calls execute concurrently but results append in source order
- approval-required call blocks run without appending fake error tool result
- auth-required call blocks run separately from approval
- resume from approval checkpoint continues or replays with invocation fingerprint
- steering message injects before next model call
- follow-up message restarts after natural stop
- capability surface version change rebuilds tool schemas before next model call
- profile-disallowed capabilities are absent from the visible surface before model call
- visible but parameter-denied capability returns structured denial without changing backend selection
- local and hosted profiles run the same loop while resolving different backends
