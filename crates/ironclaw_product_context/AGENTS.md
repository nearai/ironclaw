# Agent Map — ironclaw_product_context

## Start Here

- Read `CLAUDE.md` first; it is the project-wide guardrail file.
- Read `Cargo.toml` to confirm the dependency surface before adding any imports.

## What This Crate Owns

- **Single resolver for turn-origin/surface/owner classification at ingress.**
  Every inbound submission path (product workflow, web UI, Reborn local dev) must
  call one of the two resolvers here rather than constructing `ProductTurnContext`
  inline.
- `resolve_inbound` — the only place a `ScheduledTrigger` origin is minted.
  It requires `TrustLevel::Trusted && is_trigger_adapter`; any other combination
  yields `TurnOriginKind::Inbound`. Untrusted callers cannot forge a trigger origin.
- `resolve_web_ui` — always yields `TurnOriginKind::WebUi` with no adapter or
  surface; used by the WebUI gateway.
- `TrustLevel` — ingress trust enum. Callers map their local policy (e.g.
  `BindingResolutionPolicy`) to `Trusted` or `Untrusted` before calling the resolver.

## Dependency Constraints

This crate depends only on `ironclaw_turns` and `ironclaw_host_api`.

**Do not add** `ironclaw_conversations`, `ironclaw_product_adapters`,
`ironclaw_product_workflow`, or any trigger/pipeline crate as dependencies.
This crate must remain callable from every ingress layer — product workflow,
web UI gateway, Reborn composition — without introducing import cycles.

## Do Not Move In Here

- Binding resolution, thread scope construction, or conversation routing logic.
- Channel-specific adapter types or product-workflow error types.
- Raw secrets, host paths, or backend credentials.

## Validation

- Fast local check: `cargo test -p ironclaw_product_context`
- After changing the resolver contract: `cargo test -p ironclaw_product_workflow -p ironclaw_reborn_composition`

## Agent Notes

- If a new ingress path needs to stamp a `ProductTurnContext`, add it here as a
  new resolver function — do not inline `ProductTurnContext { ... }` construction
  at the call site.
- The `ScheduledTrigger` guard (`TrustLevel::Trusted && is_trigger_adapter`) is a
  security boundary; do not relax it without an explicit trust-model review.
- Keep edits inside this crate unless the resolver contract itself changes; changes
  that ripple into `ironclaw_turns::ProductTurnContext` are contract-change requests.
