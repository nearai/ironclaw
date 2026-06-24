# Reborn Operator Observability Backend Contracts

Issues: #4595, #4596, #4597, #4598

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

## Operator doctor diagnostics (#4596)

`GET /api/webchat/v2/operator/diagnostics` is the canonical Reborn doctor
surface. It must aggregate existing typed service evidence rather than
implementing a separate diagnostic command plane.

Required behavior:

- Include the current operator status payload when status is available.
- Convert non-ready status checks into stable diagnostic reason codes under the
  `status` owning area. Status check ids may enter the public reason-code suffix
  only when they match lowercase snake-case `[a-z][a-z0-9_]{0,63}` and do not
  look secret/path-bearing; otherwise use a stable status/state fallback reason
  code and sanitize display fields.
- Include setup diagnostics for provider/model/profile/WebUI access without
  echoing secrets or provider/backend error details.
- Include effective-config diagnostics for unsupported, immutable, deprecated,
  secret-backed, or unknown settings.
- Continue returning a typed diagnostics payload when one subsystem is
  unavailable; a missing setup/status service should become a sanitized
  diagnostic instead of failing the entire doctor route.
- CLI doctor commands, when retained, should be wrappers around this same
  service/API evidence and not a parallel diagnostic implementation.

## Operator logs (#4597)

`GET /api/webchat/v2/operator/logs` should return bounded, cursor-paginated,
redacted log entries nested under the `logs` field of the root
`RebornOperatorCommandPlaneResponse`.

Required behavior:

- Respect the facade limit clamp before hitting the backend.
- Support optional level and target filters using stable enum/string values.
- Support `tail=true` for newest entries in chronological order and
  `follow=true` with an opaque cursor for newer entries when the backend can
  retain an in-process cursor window.
- Reject requests that set both `tail=true` and `follow=true`.
- Return opaque cursors only; clients must not parse cursor internals.
- Redact secrets, tokens, credentials, raw request bodies, host-sensitive paths
  with either slash or backslash separators, and provider payload details.
- Return `service_unavailable` or an unavailable command-plane payload when no concrete backend is wired.

Initial backend options:

- In-process bounded ring buffer for current-process structured events,
  including bounded tail/follow over retained entries.
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

Initial backend options:

- Systemd service manager implementation behind an allowlisted unit name.
- Local development no-op/status implementation that reports unsupported instead of pretending success.

## Test requirements

Implementation PRs must drive the caller path:

- Service-facade tests for status/logs/service lifecycle mapping.
- Router tests for query/body validation and rate-limit descriptor stability.
- Redaction tests for log messages and lifecycle diagnostics.
- Failure-path tests for unwired, unsupported, malformed, and backend-error cases.
- Backend-specific tests for limit clamping, cursor stability, and command allowlisting.
