---
paths:
  - "crates/ironclaw_network/**"
  - "crates/ironclaw_secrets/**"
  - "crates/ironclaw_safety/**"
  - "crates/ironclaw_host_runtime/**"
  - "crates/ironclaw_processes/**"
  - "crates/ironclaw_process_sandbox/**"
  - "crates/ironclaw_wasm/**"
  - "crates/ironclaw_mcp/**"
  - "crates/ironclaw_product_adapters/**"
  - "crates/ironclaw_webui/**"
  - "crates/ironclaw_prompt_envelope/**"
---
# Reborn safety and sandbox rules

## Mediation is the boundary

Untrusted inputs never receive ambient filesystem, network, process, secret, or
credential authority. Execution crosses typed host APIs, authorization,
obligations, resource reservation, and the owning runtime adapter.

External HTTP goes through `ironclaw_network`. Secrets remain encrypted and
host-side; runtime code receives references or mediated effects, not raw values.
Do not manually inject credentials or construct a second outbound HTTP path.

## Zero-exposure credentials

Persist encrypted secret material only in the secrets subsystem. Capabilities,
runtime lanes, containers, events, logs, and model context carry credential
references or redacted metadata. The host resolves and injects credentials at
the narrowest egress boundary and removes them from returned errors, URLs,
headers, bodies, and saved process output.

Process launch uses an allowlisted/scrubbed environment. Never inherit the full
host environment into a runtime lane or extension process.

## Validate before transformation

Every ingress validates and bounds the original payload before storage, prompt
construction, URL resolution, credential injection, or dispatch. Preserve the
trust class through prompt envelopes and result handling. Sanitize model-visible
output and user-visible errors without discarding server-side causes.

For URL fetches, validate and authorize the original URL, then repeat
validation, authorization, and leak scanning after resolution and on every
redirect destination. Never inject credentials until the resolved destination
passes those checks.

Attachments use the single landing routine. Memory writes use their owning
write-safety contract. Product adapters may normalize transport data but may not
upgrade its trust.

Capability parameters, outputs, provider errors, and process results are
sensitive by default. Apply the owning Reborn redaction obligation before
logging, durable event append, projection, SSE/WebSocket delivery, model-visible
results, or user-visible errors. Never add an unredacted observability path in
parallel with the mediated host result. Re-verify the active redaction boundary
with `rg -n "redact_output|redaction|sanitize" crates/ironclaw_host_runtime crates/ironclaw_events crates/ironclaw_event_streams`.

Ingress review checklist:

- authenticate and bind actor/tenant scope;
- cap body/file/count/depth before allocation or parsing fan-out;
- validate the original data before normalization hides dangerous content;
- land attachments through the shared attachment boundary;
- classify trust before prompt construction;
- reject client attempts to mint trusted inbound requests;
- persist only the validated representation and retain a sanitized error cause.

## Bounded resources

User-controlled files, bodies, strings, collections, fan-out, queues, caches,
process output, and concurrent tasks require explicit limits. Stream large
content. Use bounded queues/semaphores and documented eviction. Cache keys must
include every input that affects authorization, visibility, or stored value.

For caches, ask whether actor, tenant, scope, credential account, trust class,
runtime profile, or policy input changes the value. If so, it belongs in the key
or the cache must sit below that decision. Add a cross-user/scope regression
test for any authorization-sensitive cache.

## Sandbox invariants

Sandbox plans use typed policies and minimal mounts. Containers and external
services remain untrusted. Install and credentialed execution are separate
phases. Runtime lanes cannot bypass host-mediated filesystem, network, secret,
event, process, or resource services.

Data returned from worker processes is untrusted. The host validates the
capability identity and domain, binds it to the authorized invocation, enforces
server-side nesting/depth and resource limits, and re-applies sensitivity and
redaction obligations. Worker-provided metadata cannot grant authority or
declare itself safe.

Changes require tests for denial, limit exhaustion, redaction, scope isolation,
cancellation, and cleanup through the production caller.

Sandbox-policy changes also test mount containment, network allow/deny,
environment scrubbing, output limits, process-tree cancellation, install/run
phase separation, and malicious worker metadata.

## Process and shell execution: real OS isolation, per tenant

Motivated by issue #6170 (a shipped cross-tenant file-disclosure via `shell`).
Case study and target design: `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` §6.

- **The virtual filesystem does not contain a subprocess.** `ScopedFilesystem` /
  `MountView` bound the *filesystem capability* (`filesystem.read`), a virtual-path
  abstraction. A spawned OS process (`builtin.shell`, script lanes) sees the **real
  kernel filesystem** and ignores those mounts. Never treat the scoped/virtual
  filesystem as containment for a subprocess.
- **The only real containment for an OS process is the sandbox it runs in.** Any
  deployment that authenticates more than one user MUST route process spawns through
  the sandboxed port (`TenantSandboxProcessPort`, backed by `ironclaw_process_sandbox`)
  whose mount is derived from the turn scope — never through the unsandboxed
  `LocalHostProcessPort`. `LocalHostProcessPort` / `ProcessBackendKind::LocalHost` is
  for genuinely single-user-local deployments only.
- **Deployment mode must reflect the fact of multi-user serving.** A served instance
  (SSO on, >1 admitted `UserId`, non-loopback bind) must not resolve to
  `LocalSingleUser` / host-shell semantics. The #6170 root cause was a composition
  profile (`HostedSingleTenant`) declaring `LocalSingleUser`, which the sound resolver
  faithfully mapped to an unsandboxed host shell. Do not add or preserve a
  profile→mode mapping that gives a served profile a host process backend.
- **Fail closed.** No verified tenant sandbox ⇒ the process/shell capability is hidden
  by the visibility filter and rejected by the planner (`ProcessBackendKind::None`),
  never silently downgraded to a host process. Missing Docker/sandbox degrades to "no
  shell," never "host shell."

Any change touching process ports, the planner's process/filesystem backend rules,
or the composition-profile → `(DeploymentMode, RuntimeProfile)` mapping requires a
**two-user cross-tenant escape test** driven through the caller: user B runs a shell
command and the test asserts it cannot read user A's files. Re-verify the current
wiring with `rg -n "LocalHostProcessPort|TenantSandboxProcessPort|ProcessBackendKind::LocalHost|DeploymentMode::LocalSingleUser" crates/ironclaw_host_runtime crates/ironclaw_reborn_composition crates/ironclaw_runtime_policy`.
