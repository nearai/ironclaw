# ironclaw_event_streams — working rules

Canonical crate guidance (the crate's `CLAUDE.md` is a pointer here).
Orientation and public surface: [`README.md`](./README.md). Family boundary
and the one-way pipeline rule: [`../AGENTS.md`](../AGENTS.md).

## Start here

- Read `README.md` for what the crate is; read `Cargo.toml` for actual
  dependencies and feature shape.
- Contract docs that outrank intuition: `docs/internal/reborn/contracts/events.md`,
  `docs/internal/reborn/contracts/events-projections.md`; neighbors:
  `crates/events/ironclaw_event_projections/AGENTS.md`,
  `crates/domains/ironclaw_outbound/AGENTS.md`.

## Working rules

Keep this crate:

- **above `ironclaw_event_projections` and `ironclaw_outbound`** — it consumes
  projection DTOs and reads outbound push candidates; it implements neither;
- **transport-neutral:** no Axum, WebSocket, SSE, Telegram, Slack,
  OpenAI/Responses, or channel framing — concrete transports adapt stream
  items elsewhere (`ironclaw_assistant` and the product tier);
- **projection-safe:** consume projection DTOs, never durable event rows or
  store adapters directly;
- **access-first:** the actor/scope/view/target authorization check runs
  before any snapshot, replay, or live subscription work; tenant/user scope is
  authority-bearing for admission and projection stream reads;
- **bounded:** long-lived subscriptions pass admission policy, hold an RAII
  permit (an abandoned subscription always releases its slot), and use
  bounded buffers with explicit lag/rebase signals;
- **no-exposure:** stream-boundary validation fails closed for raw prompts,
  tool I/O, secrets, host paths, provider errors, invocation fingerprints,
  approval reasons, lease material, and backend diagnostics
  (`ProjectionRedactionValidator`);
- **egress-separated:** external push candidates are selected through outbound
  policy and are **not** implied by projection subscription access — watch and
  push are two independent decisions, and the outbound dependency stays
  read-only.

Never move in here: durable event-store adapters or direct event-row reads;
product workflow turn submission, conversation binding, runtime dispatch, or
host execution.

## Dependency boundary

Internal deps (normal): `ironclaw_event_projections`, `ironclaw_host_api`
(including turn vocabulary), `ironclaw_outbound` (one read-only
push-candidate lookup). The `ironclaw_event_streams` `BoundaryRule` in
`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
forbids `ironclaw_event_log`, `ironclaw_event_store`, `ironclaw_filesystem`,
and every transport/product/kernel crate — evidence is reachable only through
projections (`ironclaw_event_log` appears as a dev-dependency for fixtures
only).

## Validation

- Fast local check: `cargo test -p ironclaw_event_streams --locked`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests reborn --locked`
- Run `cargo clippy -p ironclaw_event_streams --all-targets -- -D warnings`
  before requesting review.
- **Test through `EventStreamManager`** when a helper gates access, admission,
  redaction, source subscription, or outbound lookup — not the helper alone
  (`.claude/rules/testing.md`).
