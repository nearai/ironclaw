# Resource governor recovery hardening

## Context

The filesystem-backed resource governor updates an in-memory authority before
its delta journal acknowledges durable storage. A terminal journal error must
therefore fail the current operation, discard that optimistic generation, and
reload from the durable snapshot plus journal. The original recovery patch
adds typed database-contention errors, bounded journal retries, authority
invalidation, and a distinct host-visible accounting failure category.

This hardening pass closes the remaining concurrency and lifecycle gaps without
changing quota ownership. The governor remains one process-global authority
with per-account state; this does not create per-user governor instances or an
unlimited bypass.

## Invariants

1. A successful mutation is acknowledged only after its own atomic journal
   batch commits. A later failure in the same authority generation cannot
   retroactively turn that committed mutation into an error.
2. A failed journal generation is never exposed as ready for a newly loaded
   authority. Recovery publishes an explicit `Recovering` lifecycle state,
   releases the lock, installs a replacement journal sender, then publishes the
   authority slot as vacant. Concurrent enqueue and reload paths use that same
   lifecycle state.
3. The error returned to the operation that discovered the failure is always
   the primary journal/storage error. A secondary restart failure is sanitized
   and logged and leaves the poisoned authority installed, so later work fails
   closed instead of using a dead worker.
4. Default budget seeding is part of admission. A failed snapshot read or
   `set_limit` fails `pre_model_work` with `budget_accounting_failed`; the model
   provider is not called without the intended limit.
5. Post-provider accounting retains both the reservation id and the required
   terminal action. Successful calls retain `Reconcile(actual)`; failed or
   cancelled calls retain `Release`. Storage failures retry that exact action
   after governor recovery and never substitute a release for known spend.
6. Retried storage operations are limited to typed contention outcomes from an
   atomic journal write. A singleton flush uses `append`; a multi-delta flush
   uses all-or-nothing `append_batch`. SQLite/libSQL BUSY/LOCKED and Postgres
   serialization/deadlock/lock-unavailable errors enter this path; unrelated
   backend failures do not.

## Recovery lifecycle

The authority mutex owns an explicit lifecycle state:

- `Vacant` allows the next operation to load durable state;
- `Ready(authority)` is the active generation;
- invalidation changes `Ready` to `Recovering` before releasing the mutex;
- enqueue and reload fail closed while state is `Recovering`;
- recovery creates and installs the replacement sender without holding the
  authority mutex, then changes `Recovering` to `Vacant`;
- if replacement fails, recovery restores `Ready(poisoned_authority)` and all
  subsequent operations fail closed until the process is restarted.

No condition variable is required: operations that arrive during the short
restart transition receive the typed fail-closed storage outcome and can be
retried by the normal run machinery. A journal-owned generation object would be
a larger refactor without strengthening this boundary.

## Retry and acknowledgement semantics

The journal performs a small number of jittered retries only for
`FilesystemError::BackendBusy`. The SQL backend already bounds an individual
lock wait (`busy_timeout` for local libSQL); the journal also bounds how long it
will continue starting additional retries. It does not cancel an in-flight SQL
future at an arbitrary wall-clock timeout because cancellation at the commit
boundary could create an ambiguous outcome.

`RootFilesystem::append` is the atomic safety contract for a singleton delta;
`RootFilesystem::append_batch` is the all-or-nothing safety contract for two or
more deltas. Success means the requested write committed, while a retryable
contention error means none committed. A remote backend that cannot distinguish
a pre-commit contention failure from a lost post-commit response must not
classify that ambiguous transport outcome as `BackendBusy`.

Once a request receives its sequence acknowledgement, it returns success and
emits its success event. It does not re-check whether another request poisoned
the authority after that acknowledgement.

## Model-call accounting recovery

The accountant keeps an in-flight record per run:

```text
Reserved { id, estimate, pending: Release }
    | provider succeeded
    v
Reserved { id, estimate, pending: Reconcile(actual) }
```

The post-call hook retries a typed storage failure against the recovered
governor. It removes the record only after the required action succeeds. If the
post hook still fails, the model port leaves its RAII guard armed; the guard
performs one final synchronous retry of the retained action. A later pre-call
for the same run also attempts retained cleanup before rejecting overlap.

This is deliberately conservative. Known provider usage is never converted to
a zero-spend release. If storage remains unavailable, the run fails closed and
the reservation remains held rather than undercounting spend.

## Durability and compatibility

No resource snapshot or journal schema changes are required. Existing records
replay unchanged, and rollback is a code revert. Local libSQL uses WAL with
`synchronous=NORMAL`: process crashes preserve committed transactions, while an
OS/power loss may lose the most recent commits without corrupting the database.
That existing throughput/durability tradeoff is unchanged by this patch.

## Verification

Regression coverage must prove:

- restart failure preserves the primary error and does not expose a reloadable
  authority with a dead journal;
- a durably acknowledged reservation remains successful when a later request
  invalidates the same authority generation;
- default seeding failure prevents reservation/provider admission and retries
  on the next call;
- reconciliation retries preserve provider-reported usage, while cancellation
  retries release, including a transient storage failure on the release path;
- the internal accounting failure log preserves the bound storage error for
  operators while the model-visible error remains sanitized;
- a real concurrent operation cannot reload or enqueue while journal recovery
  holds the lifecycle in `Recovering` with the authority mutex released;
- SQLite/libSQL and Postgres contention codes map to `BackendBusy`, and only
  that typed outcome enters the bounded atomic journal retry path;
- singleton flushes use `append`, multi-delta flushes use `append_batch`, and
  both paths preserve the same retry eligibility and acknowledgement rules;
- the real libSQL contention contract still fails the affected request,
  reloads the same governor, and writes exactly one successful delta.
