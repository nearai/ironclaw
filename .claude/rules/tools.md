---
paths:
  - "crates/ironclaw_capabilities/**"
  - "crates/ironclaw_host_api/**"
  - "crates/ironclaw_host_runtime/**"
  - "crates/ironclaw_dispatcher/**"
  - "crates/ironclaw_extensions/**"
  - "crates/ironclaw_first_party_extensions/**"
  - "crates/ironclaw_mcp/**"
  - "crates/ironclaw_wasm/**"
  - "tools-src/**"
  - "src/tools/**"
  - "src/channels/**"
  - "src/cli/**"
---
# Reborn capability architecture

Capabilities are typed contracts executed through the mediated Reborn host
path. Product callers do not invoke runtime lanes, storage backends, provider
clients, or secret stores directly to perform an action.

The stable ownership split is:

- `ironclaw_host_api`: neutral request, authority, resource, and result types.
- `ironclaw_capabilities`: caller-facing invoke/resume/spawn workflow.
- authorization/approvals/runtime-policy: decisions and leases.
- `ironclaw_host_runtime`: obligations and host-mediated services.
- `ironclaw_dispatcher`: already-authorized routing to runtime adapters.
- WASM/MCP/scripts/first-party adapters: concrete execution lanes.
- `ironclaw_extensions`: declarative manifests and installation records only.

Verify the current call path with targeted symbol search before editing it:

```bash
rg -n "CapabilityHost|RuntimeAdapter|dispatch|invoke|resume|Obligation" \
  crates
```

## Rules

- Actions cross authorization, approvals, resource accounting, obligations,
  dispatch, and runtime execution in their defined order.
- Runtime adapters accept already-authorized typed requests; they do not repeat
  or bypass policy.
- Product workflow and UI handlers call product/capability facades rather than
  reaching into runtime lanes.
- Extension manifests declare surfaces; registries do not execute them.
- Credentials and HTTP remain host-mediated.
- Model/user-correctable failures return model-visible `Failed` or `Denied`
  outcomes. Reserve host errors for faults that make the run unable to continue.
- Results are bounded and redacted. External effects require authoritative
  evidence plus read-back verification; claim-only results are explicitly
  marked unverified as defined in `tool-evidence.md`.

Built-in capabilities are appropriate for host-coupled product behavior. WASM
is the default for sandboxed extension code. MCP is appropriate for external
server integrations. New lanes must implement existing host contracts rather
than creating a parallel execution pipeline.

## Adding a capability

1. Decide whether the behavior is a host-coupled built-in, WASM extension, MCP
   integration, or another existing runtime lane.
2. Define the typed request/result, scope, authority, resource, and redaction
   contract with the lowest stable owner.
3. Register a declarative descriptor/surface; do not make discovery execute it.
4. Route invocation through `CapabilityHost`, including approval/resume when the
   policy requires it.
5. Implement the effect behind host-runtime services or a `RuntimeAdapter`.
6. Return bounded, redacted output plus authoritative effect evidence and
   read-back verification, or explicitly mark a claim-only result unverified.
7. Add caller-path tests for allow, deny, approval/resume, invalid input,
   unavailable runtime, cancellation, and redaction.

Do not use a raw JSON/string dispatch convention when the shape is known. Do not
pass ambient database, filesystem, HTTP client, or secret handles into product
handlers or extension code.

## Direct access exceptions

Direct domain access is allowed inside the implementation that owns the domain,
for pure product reads through typed query/facade contracts, and for composition
startup/reconciliation. It is not an exception for a user-triggered mutation to
skip capability authorization or host mediation. If a call bypasses the normal
path, its code comment and PR description must name the owning contract and why
no authorization, approval, resource, audit, or runtime obligation is lost.

Review for parallel pipelines with:

```bash
rg -n "\.dispatch\(|\.invoke\(|\.resume\(" crates/ironclaw_product_workflow \
  crates/ironclaw_reborn_composition crates/ironclaw_webui
rg -n "RuntimeAdapter|CapabilityHost" crates
```
