# ironclaw_outbound

Metadata-only outbound policy and state: notification opt-in, sealed access
grants, subscription cursors, delivery resolution, and at-most-once
delivery-attempt reservation (compare-and-swap `Prepared → Sending`,
crash-recoverable). Never a transport itself — this crate has no HTTP client
and sends nothing. Alongside `ironclaw_triggers` it is one of the family's two
trust-minting authorities: its sealed grant/binding types are constructible
only through its own policy service.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_outbound` · **Manifest:** `crates/domains/ironclaw_outbound/Cargo.toml`
- **Use this when:** deciding *whether and where* something may be pushed,
  recording/reserving a delivery attempt, communication preferences, or
  subscription cursor checkpoints.
- **Don't use this when:** actually sending — product adapters consume
  candidates and report status; mutating projections →
  `ironclaw_event_projections` owns them (outbound only reads); binding reply
  routes → `ironclaw_conversations`.

## Public surface

- `OutboundPolicyService` (in `service`) — the only constructor of the sealed
  trust types; `OutboundStateStore` / `OutboundStateStorePort`.
- Sealed types + their untrusted counterparts:
  `ThreadProjectionAccessGrant` ← `ThreadProjectionAccessClaim`,
  `ValidatedReplyTargetBinding` ← `ReplyTargetBindingClaim` (claim/seal split;
  implementors return claims, never grants).
- `OutboundResolutionEngine` — the read-only candidate selector resolving
  typed delivery requests into `CommunicationDeliveryCandidate` / `NoDelivery`.
- Communication preferences, delivery targets, reply-attachment intents,
  run-final-reply handoff records, delivered gate routes, cleanup.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_attachments` (reuses
  `DEFAULT_ATTACHMENT_BUDGETS` for reply-attachment intents — the inventoried
  same-layer edge), `ironclaw_event_projections`, `ironclaw_filesystem`,
  `ironclaw_host_api` (incl. turn vocabulary).
- **Consumed by (7):** `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_event_streams`, `ironclaw_extension_host`,
  `ironclaw_host_runtime`, `ironclaw_loop_host`, `ironclaw_turn_runner`.

## Invariants

- **No transport:** the `BoundaryRule { crate_name: "ironclaw_outbound" }` in
  `reborn_dependency_boundaries.rs` forbids network/runtime crates; there is
  no HTTP dependency to misuse.
- **Sealed minting:** grants/bindings have `pub(crate)` constructors reachable
  only through `OutboundPolicyService`; trust-bearing types and the envelopes
  carrying them do not derive `Deserialize` (see [`AGENTS.md`](./AGENTS.md)).
- **At-most-once:** delivery attempts reserve via CAS `Prepared → Sending`;
  scope/candidate identity mismatches are rejected before validator I/O or
  store writes — pinned by the two contract suites below.
- Every push target is a candidate until `ReplyTargetBindingValidator`
  revalidates the route; revoked authorization records a sanitized failure and
  returns no sendable target.
- Persists metadata/refs/cursors only — no prompts, bodies, tool payloads,
  secrets, host paths, or backend error details.

## Tests

```bash
cargo test -p ironclaw_outbound
cargo test -p ironclaw_outbound --test outbound_policy_service_contract
cargo test -p ironclaw_outbound --test outbound_state_store_contract
IRONCLAW_SKIP_POSTGRES_TESTS=1 cargo test -p ironclaw_outbound --all-features  # parity without live Postgres
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.15;
  `docs/reborn/contracts/events-projections.md`.
