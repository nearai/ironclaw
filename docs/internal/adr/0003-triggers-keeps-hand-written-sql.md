# ADR 0003: `ironclaw_triggers` keeps its hand-written libSQL/PostgreSQL SQL

**Status:** Accepted 2026-08-04 (delegated authority — target-architecture WS6
`triggers` SQL ADR-or-converge row; PROPOSAL §12.12 D-L)
**Issue / rows:** CHECKLIST WS6 "Domain-internal cleanups" clause (f);
PROPOSAL §6.4.3, §11.2.6, §12 item 10
**Measured at:** `89080c5160`
**Path note (2026-08-05):** crate paths and line numbers in this ADR are as of
the measured commit, which predates the WS6/WS7 family restructure. Today
`crates/ironclaw_X` lives at `crates/<family>/ironclaw_X` (e.g.
`crates/ironclaw_triggers` → `crates/domains/ironclaw_triggers`,
`crates/ironclaw_architecture` → `crates/app/ironclaw_architecture_tests`).
The decision itself is unchanged.

## Context

PROPOSAL §11.2.6 sets the persistence idiom for the `domains/` family:
`ScopedFilesystem` is the floor, every domain crate is backend-neutral, and
*"a crate that instead needs a hand-written SQL backend is a deliberate, narrow
design choice that must be justified by an ADR"*
(`docs/internal/reborn/target-architecture/families/domains.md:49`). Two crates are
outside that floor today: `ironclaw_triggers` and `ironclaw_hooks` (ADR 0004).
The restructure row gave each the same binary choice — converge onto the
`RootFilesystem` mount catalog, or write the ADR.

`ironclaw_triggers` carries **3,372 lines** of hand-written SQL across two
drivers: `src/libsql.rs` (1,869) and `src/postgres.rs` (1,503). (PROPOSAL
§6.4.3 records "3,347"; the figure has drifted +25 and is corrected here.)
Both implement the same 21-method `TriggerRepository` trait
(`src/lib.rs:1036`), alongside an in-memory reference (`src/in_memory.rs`).
Counted by executed-statement site, that is **46 distinct SQL statements on
libSQL and 40 on PostgreSQL**, of which **26 / 25** are on the runtime path.

The question this ADR answers is not "is hand-written SQL nice" — it is
whether the crate's *claim/queue semantics* survive the move to the fabric.

## What the SQL actually does

The load-bearing statements are not CRUD. They are the concurrency control for
a distributed work queue, and the contract they implement is explicit:
`docs/internal/reborn/contracts/triggers.md:202-204` requires the worker to enforce
`max_concurrent_fires_per_trigger = 1` *"through an atomic repository
claim/lease operation that covers read, eligibility check, active-fire check,
and claim write."*

**1. The claim, as a single-statement compare-and-swap** (`src/libsql.rs:754`,
inside `BEGIN IMMEDIATE` opened at `:752`):

```sql
UPDATE trigger_records
   SET active_fire_slot = ?4, active_run_ref = NULL
 WHERE tenant_id = ?1 AND trigger_id = ?2 AND state = ?3
   AND next_run_at = ?4 AND ?4 <= ?5
   AND active_fire_slot IS NULL AND active_run_ref IS NULL
 RETURNING <21 columns>
```

Five predicates and the winner's write are one indivisible statement. Split
into read-then-write, two pollers both observe `active_fire_slot IS NULL`, both
decide they are eligible, and both claim. A lost race here is not a retry — it
is **two agent turns and two threads for one scheduled fire**, each minting its
own trusted-inbound request through this crate's sealed-mint path.
`BEGIN IMMEDIATE` (rather than the default deferred) is what makes the claim
either win or fail immediately instead of upgrading a read transaction to a
write transaction and rolling back at COMMIT.

**2. The same invariant by the opposite mechanism on PostgreSQL**
(`src/postgres.rs:515` → `:1071`, then `:536`): `SELECT … FOR UPDATE` takes a
pessimistic row lock, eligibility is evaluated in Rust on the locked row, and
the `UPDATE … RETURNING` is unconditional because the lock already excludes
concurrent writers. `FOR UPDATE` is precisely the primitive a document/CAS API
cannot express: it lets a reader **serialize other readers of the same row**,
rather than merely detect after the fact that its version was superseded.
(There is no `SKIP LOCKED` anywhere in the crate — verified zero occurrences.)

**3. Lease release proves ownership in the predicate** (`src/libsql.rs:904`,
PostgreSQL peer `src/postgres.rs:702`):
`… WHERE active_fire_slot = ?4 AND active_run_ref IS NULL AND next_run_at <= ?4`.
The clause releases the lease **only if this caller still owns it and no run
has been attached**. As read-then-write, a slow failure handler can clear a
lease a *newer* fire already took — the classic lease ABA, silently
double-firing the next slot.

**4. Derived state is computed inside the transaction that writes it**
(`src/libsql.rs:1126`, `src/postgres.rs:886`, both carrying the comment *"Fetch
the record inside the transaction to compute next state atomically"*). The next
fire time comes from the cron/timezone schedule in Rust; the read of `schedule`
and the write of the derived `next_run_at` must be one unit, or a concurrent
`upsert_trigger` (a user editing the cron expression) is silently overwritten
by a scheduler advancing a slot computed from the old expression.

**5. Multi-row invariants span two tables in one transaction.** Every claim,
failure, and settlement pairs its `trigger_records` write with an
`upsert_run_history` / `complete_run_history` on `trigger_run_history`
(`src/libsql.rs:1713`, `:1758`), keyed `PRIMARY KEY (tenant_id, trigger_id,
fire_slot)` and merged with **per-column** `ON CONFLICT` precedence — the
insert path takes `excluded.run_id` but keeps a known `thread_id`, the
completion path does the reverse. Two concurrent settlement writers (a
submitter recording acceptance, a failure handler recording an error) must
converge on one row per fire slot; a read-then-write merge loses whichever
column the second writer did not know about. History pruning
(`src/libsql.rs:1793`) runs in that same transaction, so a crash cannot leave
history unbounded and two concurrent settlements cannot compute different
"keep sets" and delete each other's rows.

## Decision

**`ironclaw_triggers` keeps its hand-written libSQL and PostgreSQL repositories.
The crate is a permanent, documented exception to the `ScopedFilesystem` floor,
and stays on the §11.2.6 shrink-only driver allowlist.**

The convergence the row offered is not available at an acceptable cost, for
three separate reasons, any one of which is sufficient:

1. **The fabric's contract does not express these operations.** `RootFilesystem`
   offers virtual paths, mounts, and per-document compare-and-swap. That covers
   optimistic single-document replacement. It does not express a
   multi-predicate CAS whose predicate spans columns the writer is also
   setting; it does not express `FOR UPDATE` (serializing *other readers*); and
   it does not express a transaction spanning two record families with
   per-column merge precedence. Rebuilding claim/lease on per-document CAS
   would mean re-deriving the queue's concurrency control on a weaker
   primitive — the exact class of change most likely to reintroduce a
   double-fire that only appears under production concurrency.
2. **Both backends are live deployment shapes, so "converge on one" is a
   product decision this restructure has no mandate to make.** Unlike the hooks
   backends (ADR 0004), the trigger repositories *are* wired: composition
   selects `LibSqlTriggerRepository` or `PostgresTriggerRepository` by profile
   at `crates/ironclaw_composition/src/backend_store_assembly.rs:89`
   and `:99`, and again in the production path at
   `src/factory/production_backend_assembly.rs:1330` and `:1373`. Deleting
   either drops a shipped deployment shape.
3. **Convergence would require deleting an architecture boundary rule, not just
   rewriting persistence.** `ironclaw_triggers` is *mechanically forbidden* a
   dependency on `ironclaw_filesystem` — it is in the `forbidden` list of the
   crate's `BoundaryRule` at
   `crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs:3968`.
   Adopting `ScopedFilesystem` means legalizing a new substrate→substrate edge
   *on top of* the persistence rewrite.

Rejected alternatives: **converge on libSQL** or **on PostgreSQL** — each drops
a deployment shape (see 2). **Re-extract per-backend crates** — reverses the
fold that produced today's single conformance suite, and adds crates to a tree
PROPOSAL §2 is deleting. **Route through the mount catalog** — reason 1.

### The exception's boundaries

The crate does **not** get ambient database authority. It owns its SQL and its
transactions and takes connection admission from the substrate that owns the
pool: production libSQL composition hands the *same* `Arc<LibSqlRuntime>` to
both the root filesystem and the trigger repository
(`production_backend_assembly.rs:1328`/`:1330`), so trigger writes queue on the
one write-admission lane §11.2.6 and #6863 require. `crates/ironclaw_triggers/AGENTS.md`
already forbids the crate database URL/path/env parsing and handle
construction; that stays.

The exception is registered where it is enforced: `ironclaw_triggers` is on
`DRIVER_LINKED_CRATES` in
`crates/ironclaw_architecture_tests/tests/reborn_persistence_driver_boundary.rs:33`.
That list is asserted as a **bidirectional set equality** — a crate gaining the
driver fails, and a crate that sheds it also fails until the entry is removed —
so the allowlist can only ratchet down. This ADR is the argued justification the
const's doc comment asks for; it does not widen the list.

## Parity between the backends

`.claude/rules/database.md` requires that *"when a domain explicitly supports
multiple durable backends, keep behavioral parity for ordering, uniqueness,
timestamps, indexes, transactions, and error classification. Put adversarial
parity cases in a shared conformance suite instead of copying tests per
implementation."*

A shared suite exists and is **not** thin —
`crates/ironclaw_triggers/tests/repository_contract.rs`, 4,710 lines, **51
tests** built from **31 shared `assert_*` helpers** that each backend drives:

- Two aggregate drivers run the same 11 helpers in the same order against each
  durable backend (`libsql_repository_contract_parity:1224`,
  `postgres_repository_contract_parity:1356`), and
  `assert_durable_fire_claim_contract:3238` bundles six more.
- **Every one of the 21 `TriggerRepository` methods is exercised by a shared
  helper** — no method is covered for one backend only.
- **The claim/queue atomicity paths are covered specifically, and only against
  the durable backends**: `assert_durable_claim_is_atomic:2989` races two
  `claim_due_fire` calls with `tokio::join!` and asserts exactly one `Claimed`
  and exactly one `AlreadyActive`; `assert_mark_fire_accepted_is_idempotent_under_concurrency:3068`
  and `assert_mark_fire_replayed_is_idempotent_under_concurrency:3136` do the
  same for settlement.
- Backend-asymmetric cases are correctly *not* shared — notably
  `libsql_filesystem_and_trigger_writes_share_one_runtime_lane:1152`, which
  pins the shared write-admission invariant above. PROPOSAL §12 item 6 sanctions
  this asymmetry: parity means "same observable contract", not "same connection
  machinery".

**One real gap was found and half-closed with this ADR — read the second half.**
Every PostgreSQL leg began
`let Some((_container, pool)) = postgres_pool_or_skip().await else { return; }`,
and each of the five skip paths returned `None` after an `eprintln!`. On a
runner without Docker the entire PostgreSQL half of the parity matrix therefore
**skipped silently and reported green** — so the parity claim this ADR rests on
was only true where Docker happened to exist. The suite now honours
`IRONCLAW_REQUIRE_POSTGRES=1`, which turns every skip into a hard failure naming
the reason, following the switch `crates/ironclaw_hooks/tests/parity_matrix.rs`
already carries for the same hazard.

⚠ **The switch exists; no CI lane sets it for this crate yet.** The only lane
that exports `IRONCLAW_REQUIRE_POSTGRES=1` is `hooks-parity` in
`.github/workflows/platform-and-compat.yml:168`, and it runs `-p ironclaw_hooks`
targets only. It is also not a drop-in: that lane serves PostgreSQL from a
workflow *service container* via `DATABASE_URL`, whereas this suite starts its
own through `testcontainers`. So today the mechanism is opt-in and the honest
statement of coverage is *"parity is proven wherever the suite runs with Docker,
and can no longer silently claim otherwise when asked to be strict."* Making it
unconditional means giving triggers a lane that guarantees a Docker daemon —
worth doing, deliberately out of scope for a decision PR, and the first thing to
build if this ADR's parity argument is ever load-bearing for a release.

**Known shape deviation, recorded rather than fixed.** The suite is 31 private
helpers inside one integration-test binary, not a `pub mod contract` behind the
`test-support` feature the way `ironclaw_hooks::predicate_state::contract` is.
It is therefore not runnable by an out-of-crate backend. That costs nothing
today — both durable backends live in this crate — but a third backend, or a
future fabric-routed one, would have to refactor the suite before it could opt
in. Whoever adds one should convert the helpers to an exported contract module
first, and should not copy per-implementation tests instead.

## Revisit condition

Reopen this decision when **any** of the following becomes true:

1. **`RootFilesystem` grows a multi-document transaction with predicate-scoped
   conditional writes** (something that can express "claim this row iff these
   five columns still hold, and write the history row in the same unit"). The
   fabric gaining that capability removes reason 1 outright, and this ADR
   should be re-argued rather than assumed.
2. **The queue's concurrency requirement drops** — if `max_concurrent_fires_per_trigger`
   stops being 1, or claiming stops being the mechanism (e.g. fires move to a
   real broker with its own at-most-once delivery), the atomicity argument no
   longer applies.
3. **One of the two backends is retired as a product decision.** That collapses
   this to a single-driver crate and makes convergence cheap enough to
   re-evaluate on its own merits.
4. **A third durable backend is proposed.** Do not add one under this ADR: the
   exception is for the two shapes that ship today. A third means either
   converging first, or exporting the conformance suite as described above.

Note the open follow-up in `docs/internal/reborn/contracts/triggers.md:472-474` — trigger
count quotas *"must be enforced through an atomic repository/database policy
when they are added"* — which would add SQL under this decision rather than
challenge it.

## Consequences

- Two of the workspace's ~64 crates hold a database driver by charter rather
  than by accident, and both now have an ADR a reviewer can cite (this one and
  ADR 0004). The §11.2.6 allowlist stops being a list of unexplained entries.
- The parity suite is load-bearing, not incidental: it is the *only* thing
  keeping two hand-written drivers behaving identically, and with
  `IRONCLAW_REQUIRE_POSTGRES=1` a CI lane can no longer report parity it did not
  prove. CI lanes that intend to cover PostgreSQL must set it.
- Any future change to claim/lease SQL must land in both drivers and in a shared
  `assert_*` helper. A change made in one driver only is the failure mode this
  decision accepts responsibility for.
- The crate keeps taking connection admission from the substrate runtime. A
  refactor that gave it its own pool would reintroduce the competing-writer
  defect #6863 fixed, and is out of bounds under this ADR as much as under
  §11.2.6.
