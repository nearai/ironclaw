# IronClaw Substrate Landing Policy

This document classifies the kinds of change that land while the IronClaw stack
is built out behind a default-off profile, and states the evidence each kind
must carry. It exists so a reviewer can look at a IronClaw PR — or a single file
in one — and know which compatibility bar applies before approving.

It is a review aid, not a merge gate. The mechanical gates live elsewhere:
`IronClawCompositionProfile` keeps production wiring default-off (see
`docs/ironclaw/production-cutover-readiness-closeout.md`), and the final
user-visible cutover is governed by the reviewer checklist in #3039. This
policy tells you which of those bars a change is reaching for.

Related trackers: #2987 (landing epic), #3230 (default-off substrate landing),
#3026 (production-composition readiness, closed out), #3039 (final cutover
reviewer checklist), #3032 (no-exposure safeguards).

## Why classify

A large IronClaw PR can mix code that cannot affect any current user with code
that changes a path v1 users hit today. Those carry very different risk and
very different test expectations. Naming the category up front keeps the
compatibility conversation on the small set of files that actually need it,
instead of re-litigating the whole diff.

If a single PR spans more than one category, say so in the description and
label the files — reviewers apply the strictest bar to each file, not the
loosest bar to the PR.

## The four categories

| Category | One-line test | Can it change what a current user sees? |
| --- | --- | --- |
| Default-off substrate | New IronClaw crate/module/type reachable only under a non-default profile | No |
| Existing-path hardening | Edits a path v1 (or an already-shipped IronClaw surface) executes today | Only as an explicit, tested delta |
| Cutover-enabling change | Wiring/config/readiness that lets a profile turn IronClaw on, still not default | No, until a separate flip |
| User-visible cutover | Flips a default so real traffic reaches IronClaw | Yes — the point of the change |

### 1. Default-off substrate

New IronClaw architecture — crates, modules, traits, adapters, stores, contract
types, and their tests — that no default startup path reaches. Most IronClaw PRs
are this category.

Expected evidence:

- The change is reachable only under a non-default `IronClawCompositionProfile`
  (`disabled` is the default; `build_ironclaw_runtime` rejects live traffic
  outside `production` with validated readiness).
- Crate-boundary tests still pass (`crates/ironclaw_architecture/tests/ironclaw_dependency_boundaries.rs`);
  the new code does not add a dependency edge from a v1/default path into
  IronClaw-only crates.
- **State plainly in the PR why this cannot become user-visible by default** —
  i.e. which profile/flag still gates it and that the default is unchanged.
  A substrate PR that cannot name its gate is really a cutover-enabling or
  user-visible change and must meet that bar instead.
- Contract/behavior covered by crate-level tests; no compatibility note needed
  because there is no existing behavior to preserve.

### 2. Existing-path hardening

Edits to a path that runs today — either legacy v1 (`src/`) or an
already-shipped IronClaw surface — for a bug fix, refactor, or safety
improvement. This is the highest-risk category precisely because it is not
gated behind an off-by-default profile.

Expected evidence:

- **An explicit compatibility note in the PR**: what behavior is preserved,
  and what (if anything) intentionally changes. "Behavior-preserving" is a
  claim that must be backed by tests, not asserted.
- A regression test that pins the behavior at the caller, not just a helper —
  see `.claude/rules/testing.md` ("Test Through the Caller"). A pure-refactor
  PR shows the existing suite still green; a behavior delta adds a test that
  fails before and passes after.
- If the edit touches a security/compatibility boundary (auth, secrets,
  egress, redaction, run-state), the note says so and links the relevant
  contract under `docs/ironclaw/contracts/`.

The test-harness extraction that motivated this doc is itself an
existing-path-hardening change of the mildest kind: test-only, behavior-
preserving, evidenced by the unchanged `host_runtime_services_contract.rs`
assertions passing after setup moved behind `tests/support/host_runtime_harness.rs`.

### 3. Cutover-enabling change

Composition, configuration, or readiness plumbing that makes it *possible* for
a profile to turn IronClaw on — without changing any default. Examples: adding a
production factory, a readiness diagnostic, a profile parse path, or a
config-file boot profile.

Expected evidence:

- The default remains `disabled`; the change adds capability, not exposure.
  Cite the readiness/fail-closed test (e.g.
  `runtime_rejects_disabled_profile_before_local_substrate_lookup`,
  `runtime_rejects_migration_dry_run_before_live_traffic`).
- Production wiring fails closed on missing/local-only/unverified components
  (`ProductionWiringReport` mapping), so a half-wired graph cannot serve.
- No hidden dual-writer: the change does not silently write both v1 and IronClaw
  state. Migration/backfill bridges belong to #3029, not to a readiness PR.
- The PR notes that a *separate*, later change is required to actually flip a
  default — this PR is not that flip.

### 4. User-visible cutover

Flips a default so real traffic reaches IronClaw (or makes a IronClaw surface the
served path for some cohort). This is the only category that intentionally
changes what a user experiences.

Expected evidence:

- Goes through the **#3039 final integration reviewer checklist** — this
  policy does not authorize a default flip on its own.
- Rollback is defined and default-off remains reachable: switch the profile
  back to `disabled` or stop the IronClaw binary keeps v1 serving
  (`docs/ironclaw/production-cutover-readiness-closeout.md`, "Rollback And
  Default-Off Contract").
- No-exposure safeguards (#3032) are satisfied for every surface the cutover
  now exposes: no raw secrets, credential-shaped values, or host paths in
  user-, model-, event-, or audit-visible output.
- Backend parity (libSQL + PostgreSQL) evidence for any newly-served durable
  state.

## Using this in review

1. For each non-trivial file in the diff, pick the category.
2. Confirm the evidence for that category is present (note, tests, gate,
   checklist link).
3. If a file claims "default-off substrate" but a default path can reach it,
   it is miscategorized — send it back to the existing-path-hardening or
   cutover bar.
4. Anything reaching category 4 needs #3039 sign-off, full stop.
