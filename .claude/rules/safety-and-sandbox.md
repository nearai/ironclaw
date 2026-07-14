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
  - "crates/ironclaw_reborn_webui_ingress/**"
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
