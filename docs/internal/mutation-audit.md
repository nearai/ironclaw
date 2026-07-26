# Mutation audits — testing the tests

Coverage proves a line *ran*. It says nothing about whether anything *checked
the result*. A test can execute a sorting function and never look at whether it
sorted.

A mutation audit sabotages one piece of production code at a time and re-runs
the suite. A sabotage the suite still passes is code with no assertion behind
it. That is the gap coverage cannot see.

Epic #6524, workstream 11.

## The finding that motivated this

`crates/ironclaw_event_projections/src/runtime_projection.rs` had a test named
`replay_projection_orders_runs_by_recent_activity_descending` — its entire job
was to verify run ordering. Deleting the sort function it was guarding did not
fail it.

The projection stores runs in a `HashMap`, and the test used two invocations.
With two entries, unsorted order matches sorted order about half the time, so
the test was a coin flip that kept landing heads. It was correct in intent and
structurally unable to fail. Reading it tells you nothing; only the sabotage
does. Fixed in PR #6674 by going to six invocations (~1-in-720 rather than
~1-in-2), verified by 20 consecutive hand-sabotaged runs.

## What the escape-history targets found

The frontier below starts with "modules where a bug already escaped to `main`".
Run against the first of those:

`crates/ironclaw_dispatcher/src/lib.rs` — 23 mutants, **9 caught / 1 missed /
13 unviable** (9 of 10 viable). The one survivor was the
`invocation.process_id.is_none()` match guard, which *is* the #6636 fix: it
decides whether a loop invocation carries parent lineage, and lineage is what
keeps a nested capability failure out of the top-level run projection. Every
test in the file left `process_id` unset, so only one side of the guard was ever
exercised. The fix for an escaped bug had no assertion on the input that
discriminates it, and deleting the guard changed nothing any test looked at.

That is the same shape as the motivating finding, and worth stating plainly
because it is the argument for the frontier order: **a module that just had an
escape is a module whose fix is likely unasserted.** The bug was found and fixed
by hand; the test that would notice it coming back was not written.

## Running one

```bash
cargo install cargo-mutants --locked

# one file (start here — a file is a coffee break, not an overnight job)
./scripts/mutation-audit.sh -p ironclaw_event_projections \
    crates/ironclaw_event_projections/src/runtime_projection.rs

# a whole package
./scripts/mutation-audit.sh -p ironclaw_dispatcher
```

Output is `mutants.out/triage-queue.md`: one entry per survivor, with the
sabotage diff and the enclosing source inlined so nobody has to go hunting.
`mutants.out/` is gitignored — it is a regenerable artifact.

Measured cost: 39 mutants over one 449-line file in ~9 minutes (a one-time
baseline build, then ~3s build + ~3s test per mutant via incremental
compilation).

### Auditing anything that depends on `ironclaw_webui`

`ironclaw_webui`'s `build.rs` shells out to a frontend build. cargo-mutants
compiles in a fresh copy of the tree, so that build runs for real on every job
and fails on any machine whose node/corepack setup can't complete it — the
audit dies at the baseline with no mutants tested. Set `SKIP_FRONTEND_BUILD=1`,
which the build script already honours:

```bash
SKIP_FRONTEND_BUILD=1 ./scripts/mutation-audit.sh -p ironclaw_reborn_composition \
    crates/ironclaw_reborn_composition/src/extension_host/channel_outbound_targets.rs
```

The frontend is not under mutation, so skipping it costs no coverage.

## Never inherit `CARGO_TARGET_DIR`

Both scripts unset it and say so. This is not fussiness — it silently corrupts
results.

cargo-mutants copies the source tree per job. If every copy is redirected at one
shared absolute target directory, parallel jobs clobber each other's compiled
artifacts and a job can run a test binary built from a *different* job's mutated
source. Verdicts then come out wrong in **both** directions: a killed mutant
reported as surviving, and — far worse — a surviving mutant reported as caught,
which launders a real hole as covered.

This was observed, not theorised. The same mutant reported MISSED with a shared
target dir and caught without one, on byte-identical source. If you invoke
`cargo mutants` directly rather than through these scripts, do not set it.

## Triage: every survivor gets exactly one verdict

| verdict | meaning | next step |
|---|---|---|
| `real-gap` | the behaviour is genuinely unasserted | write a test; `scripts/mutation-verify-fix.sh` must accept it |
| `equivalent-mutant` | no test *can* catch it — the change cannot alter observable behaviour | record the reasoning; write no test |
| `needs-product-decision` | the intended contract is unclear | route to an owner; **do not invent one** |

**Score over viable mutants, not all mutants.** Mutants that fail to compile
("unviable") carry no signal. The motivating run was 20 caught / 22 viable, with
17 unviable — reporting 20/39 would understate the suite and send someone
chasing non-problems.

### `equivalent-mutant` is real and common

Roughly a third of survivors in the first run could not be caught by any test.
Worked example from that run — `> ` changed to `>=` in
`enforce_capability_activity_output_limit`:

At `len == limit`, the only input where the two differ, the mutated branch runs
`select_nth_unstable_by`, then a `truncate(limit)` that is a **no-op** because
`len == limit`, then the same full sort. Both paths produce identical output.
Writing a test to "fix" this would assert nothing.

This is why a mutation *score* must never gate CI: it would be a flake
generator, and the pressure to make the number go up produces exactly the
decorative tests the audit exists to find.

### `needs-product-decision` is load-bearing, not an escape hatch

The second survivor in that run was `retain_invocations` — the only enforcement
of `fold_runtime_prefix`'s documented "invocations identified by `touched`" and
`O(touched)` memory contracts, on a checkpoint that accumulates across pages.

A regression test written for it **failed against unmutated code**: today a page
does return earlier pages' invocations. So the model of the seam was wrong, not
the implementation, and whether the runs list is page-scoped or thread-scoped is
a product question.

The right move was to route it, not to reshape the assertion until it went
green. Without this verdict available, whoever works the queue invents an
intended contract and writes a test asserting current behaviour — manufacturing
the decorative coverage the audit is meant to remove. Grade queue work on
correctly routing here, not on closing every entry.

## The acceptance gate

A fix for a `real-gap` survivor is accepted only when both hold:

1. the suite **passes** on unmodified code — rejects a test reshaped to match a
   mutant rather than the intended behaviour;
2. the suite **fails** with that sabotage applied — rejects a decorative test.

```bash
./scripts/mutation-verify-fix.sh -p ironclaw_event_projections \
  'crates/ironclaw_event_projections/src/runtime_projection.rs:80:5: replace sort_runs_for_projection with ()'
```

Copy the mutant string verbatim from `missed.txt` or the triage queue. Exit 0
accepted, 1 rejected, 2 usage/tooling error.

This criterion is mechanical, which is what lets the queue be worked at volume:
no reviewer has to judge whether a test is real. Both failure modes above were
produced during development of this harness, and both are now auto-rejected.

## Scaling up

Workspace-wide there are ~43,700 generatable mutants
(`cargo mutants --list --workspace | wc -l`). Compute is affordable — sharded
nightly via `--shard k/n`. The cost that matters is triage, so expand a
frontier rather than switching everything on:

1. Modules where a bug already escaped to `main` — that is where the hypothesis
   is cheapest to test, and where it first paid off.
2. The invariants workstream 11 names: authorization, approvals, credential
   scope, tenant isolation, persistence/CAS, retry classification, trigger
   scheduling, idempotency, delivery deduplication, redaction. A file list, not
   whole crates.
3. Nightly across all crates, reporting only **newly surviving** mutants against
   a committed baseline (`--iterate` skips previously-caught ones). Triage the
   delta, not the corpus — the same shape as the coverage ratchet.
4. Never a PR-blocking mutation score. See `equivalent-mutant` above.

## Self-tests

```bash
./scripts/test-mutation-audit.sh
```

Fast and hermetic — no cargo. Pins the failure modes that produced confidently
wrong answers during development: an unscoped run silently finding zero mutants,
an inherited `CARGO_TARGET_DIR`, scoring over all mutants instead of viable
ones, and a missing report reading as an empty queue. Guardrails are code; a
checker that silently does nothing is worse than none.

Section G exists because the first four sections were not enough. They drive
`mutation_triage_queue.py` directly with hand-built fixtures, so they stayed
green while every real audit failed at its last step: cargo-mutants writes to
`$MUT_OUT/mutants.out`, and the script was reading `$MUT_OUT`. The helper was
correct and the caller wired it to the wrong path — the gap
`.claude/rules/testing.md` calls *test through the caller*. Section G runs
`mutation-audit.sh` end to end behind a stub `cargo` that reproduces the
directory layout, so the wiring itself is asserted.
