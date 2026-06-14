# Reborn Operator Observability Backend Contracts

Issues: #4595, #4597, #4598

PR #4608 added the WebUI v2 route shells and product-workflow DTOs for operator status, logs, and service lifecycle. The remaining work is to wire concrete backend behavior behind those stable surfaces.

## Operator status (#4595)

`GET /api/webchat/v2/operator/status` should report a bounded, redacted snapshot of the current Reborn host.

Required payload shape, nested under the `operator_status` field of the root
`RebornOperatorCommandPlaneResponse`:

- `state`: one of the stable `RebornOperatorStatusState` values.
- `checks`: individual readiness checks with stable ids, severity, status, message, and remediation.
- No raw host paths, secrets, tokens, command lines, provider payloads, or unbounded logs.

Minimum backend inputs:

- Reborn composition readiness state.
- Runtime process binding/readiness state.
- Storage/secrets configuration readiness, reported as booleans or stable diagnostic ids only.
- LLM config availability, reported without API keys or provider error strings.

## Operator logs (#4597)

`GET /api/webchat/v2/operator/logs` should return bounded, cursor-paginated,
redacted log entries nested under the `logs` field of the root
`RebornOperatorCommandPlaneResponse`.

Required behavior:

- Respect the facade limit clamp before hitting the backend.
- Support optional level and target filters using stable enum/string values.
- Return opaque cursors only; clients must not parse cursor internals.
- Redact secrets, tokens, credentials, raw request bodies, host-sensitive paths, and provider payload details.
- Return `service_unavailable` or an unavailable command-plane payload when no concrete backend is wired.

Initial backend options:

- In-process bounded ring buffer for current-process structured events.
- Optional journald/systemd integration behind a separate service implementation.
- File-tail support only if path allowlisting and redaction are enforced before response construction.

## Service lifecycle (#4598)

`POST /api/webchat/v2/operator/service` should expose lifecycle status and
control only where the host deployment owns a manageable service unit, with the
payload nested under the `service_lifecycle` field of the root
`RebornOperatorCommandPlaneResponse`.

Required behavior:

- `status` is safe in every deployment and may report `unsupported`.
- `install`, `start`, and `stop` fail closed unless a concrete host service manager is explicitly wired.
- Commands must not shell out through untrusted input.
- Responses include stable state, message, and diagnostics without raw command output.

Initial backend:

- Local composition wires an allowlisted launchd/systemd user-service manager for
  the fixed `com.ironclaw.reborn` / `ironclaw-reborn.service` unit.
- Unsupported OS targets return `unsupported` with remediation instead of
  pretending success.
- The backend returns stable typed state only; raw command output and host paths
  are not surfaced to the browser.

## Test requirements

Implementation PRs must drive the caller path:

- Service-facade tests for status/logs/service lifecycle mapping.
- Router tests for query/body validation and rate-limit descriptor stability.
- Redaction tests for log messages and lifecycle diagnostics.
- Failure-path tests for unwired, unsupported, malformed, and backend-error cases.
- Backend-specific tests for limit clamping, cursor stability, and command allowlisting.
