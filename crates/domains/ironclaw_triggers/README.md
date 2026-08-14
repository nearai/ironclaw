# ironclaw_triggers

The scheduled-trigger domain: trigger records, cron/timezone validation,
deterministic fire identity, the poller's per-tick evaluation step, and the
sealed trusted-submission minting that identifies a fire as coming from this
crate's own poller. One of only two trust-minting crates in the family
(`ironclaw_outbound` is the other), and the family's one ADR-held hand-written
SQL exception.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_triggers` · **Manifest:** `crates/domains/ironclaw_triggers/Cargo.toml`
- **Use this when:** creating/validating trigger records or schedules,
  evaluating due fires, or anything on the host-trusted fire submission path.
- **Don't use this when:** you need poller *lifecycle* (start/stop/wiring) →
  `ironclaw_composition`; first-party trigger capabilities (create/list/remove
  tools) → the extensions tier; turn-coordinator wiring → composition's
  submitter adapter.

## Public surface

- Record grammar: `TriggerRecord`, `TriggerSchedule` (cron + timezone
  validation; sub-minute schedules rejected), `TriggerFireIdentity`
  (deterministic from `(tenant_id, trigger_id, fire_slot)`), `TriggerState`,
  `TriggerRunRecord`, `TriggerError`.
- `TriggerRepository` + implementations: `LibSqlTriggerRepository`,
  `PostgresTriggerRepository` (both ship; ADR 0003), and
  `InMemoryTriggerRepository` for tests.
- The poller tick (`worker::`): evaluation over repository / materializer
  (`TriggerPromptMaterializer`) / submitter / state-lookup ports this crate
  defines.
- Trusted-submission minting (`trusted_submit::`): the sealed
  `TrustedTriggerSubmitRequest` — `new` runs the trusted-trigger prompt safety
  scan (`src/prompt_safety.rs`) before sealing, so "this prompt passed the
  scan" is an invariant of the type, not a step a submitter performs.
- `TriggerSourceProvider` (+ `ScheduleTriggerSourceProvider`) — the source
  extension point.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_host_api` (incl.
  its turn vocabulary), `ironclaw_libsql_runtime` (admission for its SQL lane
  — the crate owns its SQL and transactions but never the pool),
  `ironclaw_safety` (the scan at the mint — a same-layer leaf edge, chartered
  2026-08-04). **No normal `ironclaw_filesystem` dependency** — persistence
  here is the ADR-held SQL pair plus in-memory; the `BoundaryRule` forbids
  `ironclaw_filesystem` outright (dev-dep only, for tests).
- **Consumed by (6):** `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_conversations`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_host_runtime`.

## Invariants

- **Hand-written SQL is chartered, not a precedent:**
  [ADR 0003](../../../docs/internal/adr/0003-triggers-keeps-hand-written-sql.md)
  (claim/lease semantics inexpressible on the filesystem fabric; both backends
  ship by profile). `reborn_persistence_driver_boundary.rs` pins this crate as
  the tagged exception; a third SQL crate needs its own ADR.
- **The mint validates what it seals:**
  `tick_rejects_injection_prompt_before_any_trusted_submitter_is_reached`
  (`src/worker/tests.rs`) drives the real `tick_once` with a non-scanning
  materializer and proves the submitter is never reached; its companion pins
  that medium-severity findings still submit. Severity policy stays in
  `ironclaw_safety::validate_trusted_trigger_prompt`.
- A scan-rejected prompt is a **permanent** failure (`InvalidMaterialization`)
  — the poller advances the slot rather than re-firing.
- No reaching upward: `BoundaryRule { crate_name: "ironclaw_triggers" }` in
  `reborn_dependency_boundaries.rs` forbids kernel/loop/product/app crates;
  ownership of the trusted path is additionally pinned by
  `untrusted_ingress_paths_cannot_submit_host_trusted_inbound`.
- No database URL/env parsing or handle construction — repositories accept
  already-open handles (`Arc<libsql::Database>`, pool clients) from
  composition.

## Tests

```bash
cargo test -p ironclaw_triggers
cargo test -p ironclaw_triggers --test repository_contract   # backend parity; IRONCLAW_REQUIRE_POSTGRES=1 hardens skips
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.3;
  `docs/internal/reborn/contracts/triggers.md`.
