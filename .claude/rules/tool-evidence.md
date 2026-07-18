---
paths:
  - "crates/ironclaw_capabilities/**"
  - "crates/ironclaw_host_runtime/**"
  - "crates/ironclaw_dispatcher/**"
  - "crates/ironclaw_product_workflow/**"
  - "crates/ironclaw_first_party_extensions/**"
  - "crates/ironclaw_webui/**"
  - "crates/ironclaw_reborn_composition/**"
  - "crates/ironclaw_agent_loop/**"
  - "crates/ironclaw_runner/**"
---
# Capability evidence and side-effect verification

A successful capability result is a claim about an effect. Side-effecting
capabilities must return evidence produced by the authoritative boundary, not an
optimistic local message.

## Result admission

The loop may claim completion only from the structured capability outcome and
its admitted evidence. Model prose such as “done,” an empty output, or a locally
constructed success string is not evidence. Correctable failures stay
model-visible so the loop can retry or explain them; host/infra failures remain
terminal according to `agent-loop-capabilities.md`.

Examples:

- Provider writes return the provider-issued identifier or revision.
- Filesystem writes return the committed version/size and may re-read metadata.
- Installation and activation read back the durable lifecycle state.
- OAuth completion performs a minimal authenticated read before success.
- Trigger or automation changes read back the stored definition and schedule.

Empty output from a capability that promises data is an error unless the
contract explicitly defines an empty success. Fast completion is not itself
evidence.

## Read-back rules

Use the strongest inexpensive verification the boundary offers:

- provider mutation: provider-issued ID/revision plus a minimal read when the
  provider is eventually consistent enough to support it;
- filesystem mutation: committed version/metadata and re-open when durability
  is the claim;
- lifecycle mutation: durable installation/activation state and registered
  surface;
- auth completion: authenticated identity/account read;
- trigger/automation mutation: stored definition, schedule, and identity;
- outbound send: provider delivery/message identifier, not merely queued.

When read-back is impossible, define an explicit weaker evidence type (for
example, accepted/queued) rather than reporting completed/delivered.

UI success follows backend evidence. Do not show a local optimistic checkmark
for install, connect, save, or execute while the durable or provider state is
unknown.

The frontend renders backend evidence or a pending state. It must reconcile
after reconnect instead of preserving an optimistic mutation indefinitely.

## Failure tests

Cover missing evidence, malformed provider responses, read-back mismatch,
accepted-but-not-completed states, duplicate/idempotent retry, and success where
the UI or model-visible result accidentally drops the evidence.

Tests must drive the production caller and always assert the evidence value and
sanitized model/user result. Assert durable state and emitted events when those
effects are part of the capability contract; do not invent either requirement
for a capability that provides neither. A test that only checks the leaf helper
does not protect the effect claim.
