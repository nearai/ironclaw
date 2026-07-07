# Spec: custom-build-tools - credentialed HTTP API wrapper builder

Sources: `lfd/_briefs/custom-build-tools.md`, lane-09 `LANE-ADDENDA.md`, `docs/lfd/roadmap-blue-lanes-2026-07-07/09-custom-build-tools/goal.md`, v1 `src/tools/builder/{core.rs,templates.rs,validation.rs,testing.rs}` as behavioral reference only, and Reborn crates `ironclaw_wasm*`, `ironclaw_extensions`, `ironclaw_host_runtime`, and authorization/secret mediation crates.

## 1. Supported shape

Build exactly one custom-tool family: a credentialed HTTP API wrapper generated from a user request plus lightweight API documentation. The wrapper accepts typed JSON input, performs one or more HTTP requests to a declared fake/provider host, transforms the provider response into typed JSON output, and is installed as a Reborn extension/tool in the same session.

Supported auth methods for this wave:

- bearer token in `Authorization` header, bound by credential name
- API key in a named header
- API key in a named query parameter
- basic auth from a bound secret
- HMAC header with timestamp, where signing uses a bound secret lease and never exposes the raw key

Unsupported requests must fail closed with typed diagnostics: arbitrary shell commands, arbitrary user-supplied code, browser automation, local filesystem tools, generic plugin marketplaces, unbounded network access, OAuth device/browser flows, raw-secret embedding, imported binary artifacts with no provenance, and malformed docs with no method/path.

## 2. Typed tool spec extraction

The builder converts user request + docs into a deterministic `HttpWrapperSpec` before any artifact is generated. Required fields:

```jsonc
{
  "request_id": "case-local stable id",
  "tool_name": "snake_case identifier",
  "description": "short user-facing description",
  "input_schema": {"type": "object", "properties": {}},
  "output_schema": {"type": "object", "properties": {}},
  "auth": {"method": "bearer|api_key|basic|hmac", "credential_name": "...", "placement": "header|query"},
  "operation": {"method": "GET|POST|PATCH", "base_url": "https://host", "path_template": "/..."},
  "transform": {"kind": "typed transform id", "params": {}},
  "sandbox": {"network_allowlist": ["host"], "fuel_limit": 10000000, "memory_limit_bytes": 67108864},
  "provenance": {"request_id": "...", "docs_hash": "sha256:...", "template_version": "..."}
}
```

Validation rejects bad identifiers, unsupported auth, wildcard hosts, non-HTTPS provider URLs outside fake test hosts, unsupported methods, missing input/output schemas, unknown transform ids, and any request that needs raw secrets in generated code. Model extraction may propose a spec, but deterministic validation decides whether it is accepted.

## 3. Generated artifact and registry conventions

The accepted spec produces a reproducible WASM component or equivalent sandboxed Reborn extension artifact under a temp build directory or target scratch path, never as a committed binary blob. The artifact package contains:

- `manifest.json` with extension id, tool name, semantic version, entrypoint, declared capabilities, requested secret names, and network allowlist.
- generated source or WAT/Rust template inputs sufficient to rebuild the component.
- `provenance.json` with `request_id`, docs hash, template version, build timestamp, source file hashes, and builder version.
- contract tests against the fake API, including at least one happy path and one provider error path when the docs define errors.

Registry install uses the normal Reborn extension lifecycle. Update increments version and preserves provenance history. Remove revokes the extension/tool registration and leaves audit state; invoking after removal must fail closed.

## 4. Sandbox, secret, and HTTP behavior

Generated tools run through the normal WASM/extension sandbox with fuel, memory, and network allowlist enforcement. They cannot open host files, execute shell commands, read arbitrary secrets, or call hosts outside the declared allowlist. Credential material is leased/injected by the host runtime at invocation time and must not appear in generated source, artifacts, state projections, replies, events, diagnostics, or fake API logs.

HTTP behavior is deterministic:

- auth placement is derived from `HttpWrapperSpec.auth`
- retries/status-code mapping are ordinary code, not model decisions
- provider 4xx/5xx responses become typed output or actionable diagnostics according to the spec
- fake API tests verify method, path, query/body, auth presence without raw secret disclosure, timeout policy, and transformed output

## 5. Failure diagnostics

Every denial returns a typed diagnostic with `class`, `message`, `actionable`, and `safe_to_retry`. Required classes include:

- `raw_secret_requested`
- `echo_tool_only`
- `missing_provenance`
- `committed_binary_blob`
- `off_allowlist_egress`
- `unsupported_shell`
- `unsupported_auth`
- `malformed_docs`
- `unsupported_code_generation`

Diagnostics must describe what the user can change without printing secrets or sealed eval data.

## 6. Eval runner profile

Profile file: `tests/integration/lfd/profiles/custom-build-tools.rs`. Profile name in every case: `custom_build_tools`. The profile interprets `setup.profile_extra.task`:

```jsonc
{
  "request_id": "cbt_dev_...",
  "shape": "credentialed_http_api_wrapper",
  "mode": "create|update|remove_after_invoke|deny",
  "user_request": "...",
  "api_docs": {},
  "expected_tool_name": "...",
  "invocation": {"input": {}},
  "fake_api": {"host": "...", "method": "GET", "path": "/...", "response": {}}
}
```

The profile must drive the real product seam, not fabricate outcomes: submit the request/API docs, let the builder create or deny the tool, run validation, install/update/remove through extension lifecycle, invoke the generated tool when applicable, and query persisted state afterward. Until this exists, return `status: "unsupported"` with a clear error.

## 7. Profile state queries

All state queries read persisted build/lifecycle/fake API state after the scenario. They are not derived from the scripted model text.

### kind `custom_tool_spec`

Params: `{ "request_id": string, "tool_name": string }`. Result shape for accepted builds:

```jsonc
{
  "tool_name": "...",
  "auth": {"method": "...", "credential_name": "..."},
  "operation": {"method": "...", "host": "...", "path": "..."},
  "transform": {"kind": "..."},
  "valid": true
}
```

For denials it may contain `valid: false`; contracts primarily read `diagnostic`.

### kind `tool_artifact`

Result shape for accepted builds:

```jsonc
{
  "created": true,
  "kind": "wasm_component",
  "committed_binary_blob": false,
  "echo_only": false,
  "provenance": {"request_id": "...", "docs_hash": "sha256:...", "missing": false},
  "sandbox": {"network_allowlist": ["host"], "fuel_limit_enforced": true, "memory_limit_enforced": true},
  "manifest": {"capabilities": ["http_egress"], "secrets": ["credential_name"], "entrypoint": "..."}
}
```

For denials: `{"created": false}`.

### kind `extension_lifecycle`

Accepted result: `{"installed": true, "version": "1.0.0|2.0.0", "removed_after_test": bool, "extension_id": "..."}`. Denial result: `{"installed": false}`.

### kind `generated_tool_invocation`

Accepted result: `{"tool_name": "...", "ok": true, "output": <typed output>}`. Provider failures that are valid wrapper behavior may set `ok: true` with typed error output. Denials should omit output or return `ok: false`.

### kind `tool_builder_diagnostic`

Denial result: `{"class": "...", "actionable": true, "safe_to_retry": false, "message": "..."}`. Accepted builds may return `{"class": "none", "actionable": false}`.

### kind `fake_api_egress`

Result: `{"call_count": n, "hosts": ["..."], "requests": [{"method": "GET", "url": "https://host/path", "auth_present": true, "raw_secret_seen": false}]}`. The runner also emits scorer `egress` events for each fake API request.

## 8. Non-goals

- Generic tool marketplaces or arbitrary plugin authoring.
- Unsandboxed shell/process tools.
- Live third-party API calls in the eval.
- OAuth browser/device flows or refresh-token storage.
- Prompt-only skills that claim to be tools without an artifact, manifest, lifecycle state, and invocation path.
- Hand-editing generated artifacts as the normal user flow.

## 9. Rollback and risk notes

Generated tools are additive extension installs. Rollback is remove/revoke the generated extension and delete scratch build artifacts. The high-risk paths are credential leakage, network allowlist widening, provenance bypass, committed binary artifacts, and lifecycle state claiming install success without a real invocable tool; every one is represented in the eval contracts.
