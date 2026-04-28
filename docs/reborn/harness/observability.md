# Reborn Observability Harness

Reborn observability should be agent-readable. An agent debugging a failure should inspect structured logs, durable events, audit records, process state, and UI artifacts before proposing a fix.

This document describes the target shape. It does not mean every field or command exists today.

## Goals

- Make Reborn failures explainable from local artifacts.
- Preserve enough IDs to correlate a user action with runtime effects.
- Keep sensitive values out of logs, events, snapshots, and user-facing errors.
- Support replay and resume debugging.
- Support #3020 compatibility evidence and #3031 product-surface migration evidence.

## Common correlation fields

Every Reborn log, event, audit record, trace, and process record should include these fields where applicable:

```text
tenant_id
user_id
project_id
agent_id
thread_id
turn_id
run_id
invocation_id
process_id
extension_id
capability_id
runtime_kind
approval_id
lease_id
```

## Evidence surfaces

Reborn debugging should be possible through:

| Surface | Purpose |
| --- | --- |
| Logs | Human/agent-readable runtime diagnostics |
| Durable events | Product-visible state changes and replay source |
| Audit records | Security/control-plane decisions |
| Process records | Background lifecycle and result state |
| Replay snapshots | Deterministic compatibility evidence |
| E2E artifacts | Browser-visible behavior |
| Doctor bundles | Portable redacted debug context |

## Redaction rules

The following must not appear in user-facing errors, events, logs, audit records, snapshots, or doctor bundles:

- raw secrets;
- bearer tokens;
- OAuth codes or refresh tokens;
- host filesystem paths when a virtual path is available;
- approval lease contents;
- unapproved input/output;
- backend error details that expose secrets or infrastructure internals;
- private network details beyond approved policy diagnostics.

## Failure classification

Observable failures should classify the failing layer when possible:

```text
authorization
approval
auth_blocked
resource_limit
network_policy
secret_unavailable
filesystem
memory
runtime_dispatch
process
provider
event_sink
projection
transport
```

## Doctor bundle target

The local harness should eventually provide:

```bash
scripts/reborn-dev doctor
```

or an equivalent `ironclaw reborn doctor --bundle` command.

The bundle should contain:

```text
config-redacted.json
logs.jsonl
events.jsonl
audit.jsonl
process-tree.json
failed-invocations.json
screenshots/
replay-command.txt
```

## Best-effort vs fail-closed observability

Observability failures must not silently change security semantics.

General rule:

- event/log sink delivery failure can be best-effort only when the owning contract says so;
- audit/persistence failure must follow the domain contract;
- unsupported obligations still fail closed;
- redaction failure is a hard failure before exposing output.
