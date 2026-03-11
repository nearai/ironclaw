# Temporal Adapter Template

This document defines the preferred first Temporal integration for IronClaw:
an **external Go adapter** that adds durable orchestration without embedding
Temporal directly into the Rust runtime. It is the next step after the local
IronClaw stack is green through `ironclaw-mcp`, the loopback gateway, the Go
router, and the local model tier.

## Goals

- Keep IronClaw as the runtime, tool executor, and gateway
- Add durable retries, approvals, and long-running workflow state outside the Rust core
- Keep the integration optional and portable across future IronClaw installs
- Avoid coupling the current Rust codebase to Temporal-specific SDK types or replay rules

## Default Architecture

```text
Cursor / CLI / External Trigger
        |
        v
Temporal Workflow (Go worker)
        |
        +--> Activity: call IronClaw gateway
        +--> Activity: call ironclaw-mcp / MCP servers
        +--> Activity: poll job status / collect events
        +--> Activity: await human approval
        |
        v
IronClaw Gateway + Runtime
        |
        +--> local tools
        +--> MCP extensions
        +--> llm-cluster-router
```

## Why External Go First

Use the external Go adapter as the default because it provides the cleanest boundary:

- Temporal workflows and activities are naturally implemented in Go with strong SDK support
- IronClaw stays focused on agent execution, channel handling, secrets, safety, and local orchestration
- Future installs only need the same adapter package plus configuration, rather than a Rust-core fork
- Failure domains stay separated: a Temporal worker restart does not require changing the IronClaw process model

## Boundary Rules

The adapter may depend on IronClaw's public gateway and MCP surfaces, but IronClaw should not depend on Temporal.

Allowed boundary:

- HTTP calls to the IronClaw gateway
- MCP calls through `ironclaw-mcp` or directly to other MCP servers
- polling for job, chat, routine, and memory state via public APIs
- durable approvals and retries in Temporal

Avoid in the first version:

- embedding Temporal SDK calls into Rust `src/`
- making Rust workflow state replay-aware
- direct database coupling from Temporal workers into IronClaw internals
- cross-process shared mutable state beyond public APIs

## Template Package Layout

Recommended portable package layout:

```text
temporal-ironclaw-adapter/
  cmd/temporal-ironclaw-worker/
    main.go
  internal/config/
    config.go
  internal/workflows/
    orchestrator.go
    approval.go
    heartbeat.go
  internal/activities/
    ironclaw_gateway.go
    mcp.go
    health.go
    storage.go
  internal/contracts/
    requests.go
    results.go
  internal/clients/
    ironclaw.go
    mcp.go
  deployments/
    docker-compose.yaml
    config.example.yaml
  Makefile
  README.md
```

## Minimal Workflow Set

Start with a small durable surface:

1. `IronclawTaskWorkflow`
   - accepts a task request
   - invokes IronClaw via gateway or bridge
   - tracks retries and final outcome

2. `IronclawApprovalWorkflow`
   - pauses when confidence, cost, or risk exceeds threshold
   - resumes only after a user signal or timeout path

3. `IronclawHeartbeatWorkflow`
   - runs as a periodic monitor using `ContinueAsNew`
   - checks gateway health, router health, and queue depth

## Activity Design

Keep all I/O in activities:

- `SendChatActivity`
  - call `/api/chat/send` or the bridge equivalent
  - return `message_id`, `thread_id`, and accepted metadata

- `WaitForChatCompletionActivity`
  - poll IronClaw chat history until completion or timeout

- `CallMCPActivity`
  - call stateful or read-only MCP tools with idempotency keys

- `GatewayHealthActivity`
  - validate `/api/health`
  - capture auth, latency, and failure reason

- `RouterHealthActivity`
  - validate `llm-cluster-router` health and model availability

## Task Queue Topology

Recommended initial queues:

| Queue | Purpose |
|---|---|
| `ironclaw-orchestrator` | main workflow execution |
| `ironclaw-gateway` | gateway-bound activities |
| `ironclaw-mcp` | MCP-bound activities |
| `ironclaw-health` | health and monitoring activities |
| `ironclaw-approval` | approval and signal workflows |

Queue design principles:

- split queues by failure domain, not by every tiny action
- keep gateway and MCP activities separate so one unhealthy integration does not starve the others
- reserve a small queue for health checks and watchdog work

## Configuration Contract

The adapter should be install-portable and environment-driven:

```yaml
temporal:
  host_port: "127.0.0.1:7233"
  namespace: "default"
  task_queue: "ironclaw-orchestrator"

ironclaw:
  base_url: "http://127.0.0.1:3000"
  api_key: "replace-me"
  timeout: "120s"

mcp:
  bridge_command: "/path/to/ironclaw-mcp"
  bridge_base_url: "http://127.0.0.1:3000"

limits:
  activity_timeout: "2m"
  workflow_timeout: "30m"
  max_chat_poll_attempts: 120
```

Portability rules:

- no absolute machine-specific paths in code
- keep deployment examples parameterized via env or YAML
- assume loopback-first defaults for both Temporal dev mode and IronClaw gateway access

## Deployment Modes

### Local development

- Temporal dev server on loopback
- IronClaw local gateway on loopback
- `ironclaw-mcp` in stdio mode

### Portable production template

- Temporal server externalized behind its own deployment unit
- Go worker deployed separately from IronClaw
- IronClaw gateway remains the single execution entrypoint for the runtime

## Success Criteria

The template is ready for implementation when:

- the adapter talks only to public IronClaw surfaces
- all durable state lives in Temporal, not in ad hoc polling scripts
- queue topology and config are reusable on a new machine with path and token changes only
- IronClaw can still run without Temporal enabled

## Recommended Next Implementation Step

Once the local IronClaw path is green:

1. scaffold the Go worker package
2. implement `GatewayHealthActivity`
3. implement `SendChatActivity` plus `WaitForChatCompletionActivity`
4. implement `RouterHealthActivity` so the worker can assert model readiness before dispatch
5. wire a single `IronclawTaskWorkflow`
6. validate against the existing smoke harness before adding approvals or child workflows
