# ironclaw_host_runtime guardrails

- Own host-side composition shared across Reborn runtime lanes.
- Keep runtime-specific request shapes in the runtime crates; adapters should translate into host API contracts and delegate here.
- Compose low-level services such as `ironclaw_network` and `ironclaw_secrets`; do not duplicate URL parsing, DNS checks, private-IP filtering, HTTP clients, secret stores, or redaction logic in runtime crates.
- Host HTTP egress lives under `src/egress/`: keep request validation/sanitization, credential-source resolution, staged network-policy lookup, staged secret injection, transport dispatch, response sanitization, and response-body storage as separate pipeline steps instead of rebuilding a monolithic service method.
- Obligation code lives under `src/obligations/` and stays in its owner: `handler` decides and executes obligations, `staged_handoffs` owns the secret/network material staged for a later consumer, `process_store` owns post-start cleanup and reservation reconciliation. `mod.rs` holds only `BuiltinObligationServices` — the assembly seam. A change that needs all three at once is a signal the split is being undone, not a reason to add code to `mod.rs`. Access needed **only** by a sibling owner is `pub(super)` — `RuntimeSecretInjectionStore::{has_for_capability, prune_expired}` and `NetworkObligationPolicyStore::contains` — so widening one of those is a deliberate edit. `pub(crate)` on the staged-handoff stores is *not* a violation of that rule: `insert`/`take`/`clone_material`/`get`/`discard_for_capability` are also called from `src/egress/**`, which is host-runtime composition outside `obligations/` and is the reason those stores exist. The test is what the caller set actually is, not which keyword appears: if a method's only callers are inside `obligations/`, it is `pub(super)`.
- Production host HTTP egress must be constructed with staged `NetworkObligationPolicyStore` and `RuntimeSecretInjectionStore` handoffs. Request-carried policy and direct `SecretStoreLease` sources are legacy/test compatibility paths only.
- Preserve the accounting invariant: `network_egress_bytes` is outbound request bytes only, with response bytes tracked separately.
- Keep raw secret material inside the narrow lease/injection path. Reject runtime-supplied manual credentials, scan raw and percent-decoded URL forms, redact leased values from runtime-visible errors and responses, strip sensitive response headers, and block credential-shaped runtime requests/responses before they reach external services or runtime callers.
- Do not own product workflow, authorization/approval policy, persistence migrations, or event emission unless a later Reborn contract explicitly moves that composition here.

## Agent-loop touch points

- Production wiring validates the `TurnRunWakeNotifier` handle consumed by
  `ironclaw_runner` (`ProductionWiringComponent::TurnRunWakeNotifier`); it does
  not construct or own the scheduler/executor.
- `surface.rs` owns host-runtime capability-surface shaping and versions.
- `production.rs` and `services.rs` compose runtime services and readiness
  evidence used by Reborn loop wiring.
- Production wiring must reject local-only runtime policy shapes, not just require
  that some `EffectiveRuntimePolicy` value is present.
- First-party runtime tools belong under `first_party_tools/`; do not append new
  built-ins to broad runtime files.
- What belongs there is the **host half**: the `CapabilityManifest`, the
  registry wiring, and a thin `FirstPartyCapabilityHandler` that translates this
  crate's dispatch types into the executor's own request/error pair. The
  **executor half** — parsing, network fetching through `RuntimeHttpEgress`,
  extraction, domain calls — belongs in `ironclaw_extension_support`, which may
  not name this crate (its `BoundaryRule` forbids it). WS3 is moving the
  existing families across that seam one at a time; the skill-install family
  (`extension_support::skills::{url_install, resolve_install_input}`) is the
  worked example.

## Adding code

- Add a new runtime service module when the service has its own authority,
  readiness, or resource accounting boundary.
- Add a first-party tool file per capability, except for tightly-coupled
  v1-compatible coding-tool families that share one legacy surface contract.
- Keep readiness checks near the runtime service they validate; driver/product
  readiness belongs in `ironclaw_runner`.

## Common mistakes

- Do not call `AgentLoopDriver` or compose loop families here.
- Do not own product adapter routing or workflow idempotency.
- Do not bypass host API contracts with runtime-specific shortcuts.
