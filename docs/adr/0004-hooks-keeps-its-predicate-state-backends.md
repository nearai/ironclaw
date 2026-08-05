# ADR 0004: `ironclaw_hooks` keeps its libSQL/PostgreSQL predicate-state backends

**Status:** Accepted 2026-08-04 (delegated authority — target-architecture WS4
`hooks` ADR-or-converge row; PROPOSAL §12.12 D-M)
**Issue / rows:** CHECKLIST WS4 "`hooks`: ADR-or-converge decision on its
libSQL/Postgres predicate backends"; **#6945** (the coverage gap the row's note
attaches); PROPOSAL §6.7.4, §11.2.6, §12 item 10
**Measured at:** `89080c5160`

## Context

`ironclaw_hooks` is the second crate outside the `ScopedFilesystem` floor
PROPOSAL §11.2.6 sets for durable persistence (the first is `ironclaw_triggers`
— ADR 0003). Its exception is a hook **predicate-state** store: the sliding-window
counters behind rate and value caps, keyed `(hook_id, tenant_id, capability)`.

`PredicateStateBackend` (`src/predicate_state.rs:370`) has three
implementations:

| Implementation | Path | Lines |
|---|---|---|
| `InMemoryPredicateStateBackend` | `src/predicate_state.rs:497` (impl `:586`) | — |
| `LibSqlPredicateStateBackend` | `src/libsql_backend/backend.rs:127` (impl `:363`) | 906 (module) |
| `PostgresPredicateStateBackend` | `src/postgres_backend/backend.rs:124` (impl `:612`) | 897 (module) |

These are not two parallel designs — they are two drivers behind one trait,
folded in from the former `ironclaw_hooks_{libsql,postgres}` crates
(`Cargo.toml:19-22`), which is why the durable code is **1,803 lines** inside
this crate rather than two crates outside it.

### The measurement that decides the framing

The obvious argument — *"both backends are live deployment shapes, composition
picks one per profile"* — is **false for hooks**, and stating it would have put
an untrue claim in an ADR. Composition hard-codes the in-memory backend:

```rust
// crates/ironclaw_reborn_composition/src/observability/hooks/factory.rs:322-328
// In-memory predicate-state backend for v1. Swappable: a durable
// Postgres/libSQL backend (#3933) drops in here without touching the rest
// of the wiring.
let backend: Arc<dyn PredicateStateBackend> = Arc::new(InMemoryPredicateStateBackend::new());
let evaluator = Arc::new(PredicateEvaluator::with_state_backend(Arc::clone(&backend)));
evaluator.warn_in_memory_backend_active_in_production();
```

That is the **only** `with_state_backend` call site outside the owning crate.
A workspace search for `LibSqlPredicateStateBackend` / `PostgresPredicateStateBackend`
outside `crates/ironclaw_hooks/` returns **zero** hits. There is no profile
switch, no config key, and no env var selecting a durable hooks backend — and
`warn_in_memory_backend_active_in_production()` exists precisely because that
is the state.

So the honest question is not "which shipped shape do we drop" (ADR 0003's
question) but **"do two unwired, fully-implemented durable backends earn their
1,803 lines?"**

## Decision

**`ironclaw_hooks` keeps both durable predicate-state backends. They stay
in-crate, behind the one `PredicateStateBackend` trait, on the §11.2.6
shrink-only driver allowlist — as a *staged* implementation awaiting its
composition switch, not as a live deployment shape.**

Three measurements carry it:

1. **They close a correctness gap the in-memory backend structurally cannot.**
   `InMemoryPredicateStateBackend`'s replay dedup is process-local
   (`src/predicate_state.rs:357`), so it cannot defend against multi-host
   replay at all. `tests/multi_host_adversarial.rs` (783 lines) exists to
   exercise exactly the cross-host properties only the durable backends
   provide. Rate and value caps are a security control; the moment IronClaw
   runs more than one host against one tenant, an in-memory counter is
   bypassable by landing on another host.
2. **They are proven interchangeable, so they are not drifting while they
   wait.** `tests/parity_matrix.rs` feeds one deterministic scripted sequence
   to *every* backend and cross-asserts identical logs, plus an independent
   hand-computed oracle so a bug shared by two backends still fails. This is
   the difference between "unwired code" and "rotting code".
3. **The swap is one line** (`factory.rs:325`). Deleting the backends converts
   a one-line change into re-deriving 1,803 lines plus both migration sets when
   multi-host lands. Deletion is the expensive option here, not the cheap one.

Rejected alternatives. **Delete both and re-derive later** — reason 3, and it
would delete the only implementations of a security-relevant property (1).
**Delete one, keep the other** — the two exist because the workspace ships both
substrates; keeping only one guarantees the other is written under time
pressure later, without the parity suite that currently keeps them honest.
**Re-extract per-backend crates** — reverses the fold that produced the single
conformance suite, and adds crates to a tree PROPOSAL §2 is deleting.
**Move behind `ironclaw_filesystem`'s mount catalog** — the predicate store is
counter state with read-modify-write and windowed eviction semantics, not a file
tree; the catalog's contract does not express it.

### Consequence stated plainly

This ADR keeps code that production does not execute. That is a real cost and
the reason the decision is written down rather than assumed. It is bounded by
being *staged*, not speculative: the consumer (`#3933`, multi-host counters) is
named, the switch point is one line, and the parity suite is what stops the
staging from decaying. If the multi-host requirement is ever formally dropped,
this ADR should be revisited and the backends deleted — see the revisit
condition.

## Parity between the backends

Parity is enforced by a shared conformance suite, in the shape
`.claude/rules/database.md` prescribes — and this crate is the workspace's
reference implementation of that pattern:

- `predicate_state::contract` (`src/predicate_state.rs:957`) is a single
  trait-level suite of **12 cases**, gated `#[cfg(any(test, feature = "test-support"))]`
  (`:956`) *so an out-of-crate backend can depend on `ironclaw_hooks` with
  `test-support` and run the same suite against its impl*. The case list is
  generated from one canonical inventory macro
  (`predicate_backend_contract_cases!`, `:1485`), so there is no second
  hand-maintained list to drift.
- Both drivers run it: `tests/predicate_state_libsql_contract.rs` and
  `tests/predicate_state_postgres_contract.rs`.
- `tests/parity_matrix.rs` cross-asserts all three backends against each other
  *and* against an independent oracle; `tests/multi_host_adversarial.rs`
  (behind `integration`) covers cross-host replay.
- `parity_matrix.rs` already honours `IRONCLAW_REQUIRE_POSTGRES=1` to turn a
  missing PostgreSQL into a hard failure *"so a skip cannot masquerade as a
  green full-matrix run"*. ADR 0003 adopts the same switch for triggers.

No extension was required here; the suite is the house pattern.

## #6945 — the coverage gap this row carried, now closed

The CHECKLIST row attached a warning: `crates/ironclaw_hooks/CLAUDE.md` claimed
cross-run hook isolation was regression-tested, naming
`crates/ironclaw_runner/tests/hooks_integration.rs` and two tests **that never
existed**. #6944 corrected the false claim; #6945 tracks the gap it was hiding.
A guardrail that does not exist reads, from the guidance, exactly like one that
does — so the ADR ships with the test.

**The semantic.** `RebornLoopDriverHostFactory` offers hook seams with two
deliberately different lifetimes. `with_hook_dispatcher_builder_factory`
(`crates/ironclaw_runner/src/loop_driver_host.rs:1289`) invokes its closure once
per `build_text_only_host*` call — i.e. once per run — so dispatcher-owned state
(slot poisoning, registry mutations, the run-scoped milestone sink) is scoped to
one run. The legacy `with_hook_dispatcher` adapter (`:1390`) deliberately does
the opposite, cloning one `Arc<HookDispatcher>` into every build. Production
wires the isolating seam (composition's factory → `runtime.rs:769-771`), so the
property holds today — but nothing failed if someone swapped it.

**The test.** `poisoned_hook_slot_does_not_leak_into_the_next_run` in
`tests/integration/hooks.rs`, at the tier and through the caller #6945 names.
A hook commits a gate-sink protocol violation, so run 1 fails closed *and*
poisons its slot; a poisoned slot is skipped for the rest of that dispatcher's
life. Two turns on one harness are two host builds, so:

- per-run dispatcher (production): run 2 gets a clean slot — the hook fires a
  second time and the fail-closed deny is re-applied. **2 fires, 0 egress.**
- shared dispatcher (legacy adapter): run 2 skips the poisoned hook, the gate
  goes quiet, and the capability reaches the wire. **1 fire, 1 egress.**

Both assertions flip, which is what makes the test red-able rather than
decorative — verified by temporarily pointing `runtime.rs` at the legacy adapter
and watching it fail on the fire count.

**Deliberately not asserted: predicate counter state.** It is keyed
`(hook_id, tenant_id, capability)` and shared across runs *by design* — the
`PredicateEvaluator` is built once per tenant by composition and `Arc`-cloned
into the per-run closure. Asserting isolation for it would pin a rate-cap
bypass, and #6945 says so explicitly. This is also why the two halves of this
document belong together: the counters this ADR keeps a durable backend for are
exactly the state the isolation test must leave alone.

**What remains pinned only at the dispatcher tier.** #6945's second property —
that the legacy adapter *shares* state on purpose — is not separately tested at
the integration tier, because the harness exposes only the isolating seam and
adding the legacy one would mean widening a production struct for a deprecated
path. It is not unpinned: `with_hook_dispatcher` is a one-line delegation to
`with_hook_dispatcher_factory(move || Arc::clone(&dispatcher))`, and
`poisoned_during_dispatch_skips_subsequent_invocations`
(`src/dispatch/mod.rs:3596`) pins that one dispatcher instance keeps its poison.
The cross-*build* half was the gap, and that is what the new test covers.

## Revisit condition

Reopen when **any** of the following becomes true:

1. **A durable backend is wired.** When `factory.rs:325` stops constructing
   `InMemoryPredicateStateBackend` unconditionally, this ADR's framing changes
   from "staged" to ADR 0003's "live deployment shapes", and the
   `warn_in_memory_backend_active_in_production` guard should go with it.
2. **Multi-host is formally dropped from the roadmap.** Reason 1 for keeping
   them evaporates, and the right move becomes deletion — 1,803 lines and two
   driver dependencies, recoverable from history.
3. **The backends stop being provably interchangeable** — a divergence the
   parity matrix cannot express, or a case that has to be skipped for one
   driver. Staged code that is no longer proven honest is just dead code.
4. **A third backend is proposed.** The exported `predicate_state::contract`
   makes that cheap; use it rather than adding per-implementation tests.

## Consequences

- `ironclaw_hooks` stays on `DRIVER_LINKED_CRATES`
  (`crates/ironclaw_architecture/tests/reborn_persistence_driver_boundary.rs:37`),
  which is asserted as bidirectional set equality and can only ratchet down.
  This ADR is the justification that entry's doc comment asks for; it does not
  widen the list.
- The crate carries unconditional `libsql`, `deadpool-postgres` and
  `tokio-postgres` dependencies (`Cargo.toml:40-51`) for code production does
  not run. Anyone shrinking the workspace's driver cone should read revisit
  condition 2 before assuming this is an oversight.
- `crates/ironclaw_hooks/CLAUDE.md`'s cross-run isolation section now names a
  test that exists. The correction #6944 made was to stop claiming coverage;
  this closes the loop by supplying it.
