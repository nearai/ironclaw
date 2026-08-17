# Doc-Truth Verification Pipeline

**Date:** 2026-08-07
**Status:** Implemented (phases 1–4); follow-ups specced below
**Answers:** issue #7317 "Proposal: Doc-Truth Verification Pipeline"
**PRs:** #7375 (drift fixes) → #7376 (docs reference gate) → #7378 (doc-fact
contract tests), #7379 (docs-live + changelog), this document

## 1. Problem

Public Mintlify docs (`docs/`) drifted from shipped behavior, and nothing
caught it. Confirmed live cases at the time of writing:

- `docs/extensions/building-a-tool.md` taught the retired
  `reborn.extension_manifest.v2` authoring shape (`[[host_api]]` /
  `[capability_provider.tools]`), which the v3 parser hard-rejects, and never
  mentioned `origin_gate_matrix` — the field whose absence broke
  1.0-era extensions on newer binaries.
- `docs/api/responses.mdx` claimed `temperature` was rejected (the router
  accepts 0.0–2.0 and forwards it), claimed `model` must be `"default"` (any
  well-formed ≤256-byte name passes), claimed `max_output_tokens` is rejected
  (accepted-and-ignored by DTO policy), and omitted the required `model`
  field from every request example.
- `docs/channels/building-a-channel.mdx` instructed contributors to edit two
  files that no longer exist.

Root cause, in two parts:

1. **No deterministic coupling** between doc claims and code — the repo has
   41 architecture ratchet tests pinning facts in `crates/`, and zero aimed
   at `docs/`.
2. **Deploy-target mismatch** — the Mintlify GitHub App deployed `docs/` on
   every push to `main`, while binaries ship from `ironclaw-v*` tags, so
   even perfectly accurate docs describe unreleased behavior between
   releases.

## 2. Decisions of record

- **Single doc tree, no Mintlify versions.** Versioning would multiply every
  page across versions × the two locales (`en` + `zh`) on a weekly release
  train. Instead the site tracks the **latest stable release**, and older
  releases are served by each git tag's preserved `docs/` tree
  (`https://github.com/nearai/ironclaw/tree/ironclaw-vX.Y.Z/docs`).
- **`docs-live` deployment branch.** Release automation force-points
  `refs/heads/docs-live` at each stable release commit; the Mintlify
  dashboard deploys from that branch (one-time out-of-repo change). Details
  and runbook: `docs/internal/weekly-release-strategy.md` § Docs
  publication.
- **Gates are deterministic only.** Pass/fail comes from string/table/route
  assertions, never an LLM judgment (refinement 1 of the issue). LLM
  assistance is reserved for the *non-blocking* fix-PR generator
  (follow-up, §5).
- **Catch drift at PR time.** The static gates run in Code Style on every
  PR (refinement 2); the release-time checks are a backstop, not the
  primary defense.
- **Human-curated changelog** (`docs/changelog.mdx`), one `<Update>` entry
  per stable release, landed on `main` **before** the Monday cut so the
  candidate branch inherits it, and enforced by the cut script. Writing the
  entry only on the frozen release branch would satisfy that week's gate and
  then vanish: the branch is never merged back, so the next candidate — cut
  from `main` — would ship a changelog missing the previous release.

## 3. Architecture — three layers

### 3.1 PR-time static gates (Code Style)

| Gate | What it pins | Where |
| --- | --- | --- |
| `scripts/ci/check-guidance.py` (docs surface) | Every backticked repo path in published pages, the `zh/` mirror, and `docs/internal/reborn/contracts/` resolves against `git ls-files`. Mintlify link targets are a different namespace and deliberately unchecked; dated archives (`docs/internal/`, non-contract `docs/internal/reborn/`) are excluded as classes. MDX comment form of the suppress marker: `{/* check-guidance: path-ok */}`. | `fast-checks` job, `has_guidance` trigger (probes pinned in `ws12_workflow_contracts.py`) |
| `crates/app/ironclaw_cli/tests/docs_cli_reference.rs` | `docs/using/cli.mdx` ↔ the real binary's `--help`, both directions, subcommand granularity, alias-aware, row-count floor. | `cargo test -p ironclaw` |
| `crates/extensions/ironclaw_extension_registry/tests/docs_manifest_schema_version.rs` | Every `reborn.extension_manifest.<version>` mention in a published page (fences included) names the current `MANIFEST_SCHEMA_VERSION_V3`; `building-a-tool.md` names it and documents `origin_gate_matrix`. | `cargo test -p ironclaw_extension_registry` |
| `crates/product/ironclaw_openai_compat/tests/docs_responses_contract.rs` | The `doc-fact:responses-request-policy` marker block in `docs/api/responses.mdx`, driven behaviorally through the real router — the marker's values parameterize the assertions. | `cargo test -p ironclaw_openai_compat` |
| `scripts/ci/docs_publication_boundary.py` (pre-existing) | Publication fence: every page published or fenced; `.mintignore` frozen. | `docs-publication-boundary` job |

**The doc-fact marker convention.** A doc region delimited by
`{/* doc-fact:<name> ... */}` in `.mdx` (HTML comment in `.md`) holds
`key = value` lines that the owning crate's contract test parses and verifies
against code — invisible in the rendered page, adjacent to the prose it pins
so an editor changing the prose sees the contract. Tests live in the crate
that owns the truth (clap tree → `ironclaw_cli`; schema constant → the
registry; request policy → `ironclaw_openai_compat`), never in a generic
doc-checking engine that would re-encode the truth as strings and drift
itself.

**Gate reachability.** The affected-area planner
(`scripts/ci/reborn_pr_test_plan.py`) historically classified `docs/` as
having no Rust test surface, which the doc-fact tests falsify: a docs-only
PR would have selected zero crate tests and merged green, deferring the
failure to the next full run on someone else's change. The planner now
routes docs changes to the crates whose tests read them — any published-tree
docs change selects `ironclaw_extension_registry` (its sweep walks the whole
published tree), `docs/using/cli.mdx` additionally selects `ironclaw`, and
`docs/api/responses.mdx` additionally selects `ironclaw_openai_compat`.
Fenced trees (`docs/internal/`, `docs/internal/reborn/`, drafts) keep the prose
classification: no cargo test reads them, and check-guidance covers their
path claims independently.

### 3.2 Release-time (the cut and publish chain)

- **Changelog gate** — `scripts/ci/cut_ironclaw_release.py::
  ensure_stable_changelog_entry`: a stable (non-rc) tag is refused when the
  candidate commit's `docs/changelog.mdx` lacks the release's
  `description="vX.Y.Z"` entry — an exact attribute match, so an rc-labeled
  entry (`description="vX.Y.Z-rc.1"`) cannot satisfy the stable gate by
  substring. Rc cuts exempt (hotfix flow unimpeded).
- **`publish-docs-live`** — job in `.github/workflows/ironclaw-release.yml`
  after `host`, prerelease-guarded, force-updates `refs/heads/docs-live` to
  the released commit via the refs API (bootstraps the branch on first
  run). Forced by design: successive stable tags need not be
  ancestor-related; the branch is a pointer, not a history. Two guards keep
  the pointer honest: the prerelease check, and a newest-stable-tag check —
  the job moves the branch only when the workflow's own tag is the highest
  stable `ironclaw-v*` tag, so re-running an older release's workflow (a
  routine move after a flaky artifact upload) cannot silently revert the
  live site. Pinned in `ws12_workflow_contracts.py` `REQUIRED_MARKERS` so
  cargo-dist regeneration cannot silently drop either the job or the guard.

### 3.3 Human checklist

`docs/internal/weekly-release-strategy.md`: the changelog entry lands on
`main` before the Monday cut (see §2 — the candidate inherits it and every
later candidate keeps it); Wednesday promotion verifies the deployed site
reflects the release (the changelog page is the probe); the "Docs
publication" section holds the Mintlify dashboard configuration, the
branch-protection shape for `docs-live` (restrict who can push, but allow
force pushes for the Actions actor — protection that blocks force pushes
breaks the automation it guards), the emergency manual repoint command, and
the **docs-hotfix recipe**: when a live page is wrong mid-week, publish a
commit whose tree is the released tag plus the docs fix and repoint
`docs-live` at it — never at `main`, which would republish unreleased
behavior. The dashboard repoint is the one out-of-repo configuration this
pipeline cannot verify from CI; the post-promotion check is the
compensating control.

## 4. What is enforced where

| Doc surface | Drift class | Gate |
| --- | --- | --- |
| Any published page, `zh/`, `docs/internal/reborn/contracts/` | dead repo path in backticks | check-guidance docs surface |
| `docs/using/cli.mdx` | missing/retired subcommand rows | `docs_cli_reference.rs` |
| `docs/extensions/building-a-tool.md` + published tree | non-current schema version named; missing v3/gate-matrix teaching | `docs_manifest_schema_version.rs` |
| `docs/api/responses.mdx` | request-policy claims (rejections, ranges, caps, unknown-field tolerance) | `docs_responses_contract.rs` |
| `docs/changelog.mdx` | stable release missing its entry | cut-script changelog gate |
| whole site | describing unreleased behavior | `docs-live` deployment branch |
| nav/fence integrity | unpublished-but-reachable pages | `docs_publication_boundary.py` |

## 5. Deferred follow-ups (specced, not built)

1. **Release-time live probe gate.** Extend
   `scripts/ci/smoke-release-binary.py` `REQUIRED_EVIDENCE` with doc-derived
   probes against the *exact packaged binary* (every documented subcommand
   answers `--help`; a served instance's `/v1/responses` rejects
   `tool_choice` with 400). Catches build-feature divergence that PR-tier
   tests on the dev profile cannot. Alternative home: an e2e blackbox lane
   scenario beside `tests/e2e/scenarios/test_reborn_blackbox_smoke.py`.
2. **LLM auto-fix docs PRs** — modeled on `openwiki-update.yml` (cron +
   dispatch, `GH_RELEASES_MANAGER` app token, opens a PR, never auto-merge):
   run the deterministic gates in `--json` mode, feed failures to a model
   that drafts the docs-only fix PR. The deterministic gates remain the only
   blocking authority; the generator only reduces fix friction (refinements
   1 and 3 of the issue). MDX lint of generated output before the PR opens
   (refinement 4) belongs in that workflow.
3. **CLI flag-level doc coverage** — deliberately out of the subcommand
   gate today; flags churn fast enough that a naive gate would train people
   to stop reading failures. Needs a curated flag-fact marker per command
   group if demand appears.
4. **Mintlify internal-link integrity** — `mint broken-links` is documented
   for local use but never runs in CI; wiring it (or a Python equivalent
   over `docs.json` routes) into the docs-publication-boundary job closes
   the link-rot class the reference gate deliberately skips.
5. **zh freshness policy** — the `zh/` mirror gets path-reference checking
   but no translation-parity signal; a per-page `translated-at` frontmatter
   plus a non-blocking staleness report is the cheap version.
6. **Contract-doc pinned-test claims** — `docs/internal/reborn/contracts/` names
   pinning tests whose functions have moved or been renamed (found while
   fixing its dangling paths); an owner pass re-verifying those claims is
   recorded here rather than half-fixed mechanically.

## 6. Risks and mitigations

- **Docs-only contributors hitting gate failures**: extraction is
  inline-code-only with a curated not-a-path filter; failures name the file,
  line, and token; the `path-ok` marker (HTML or MDX comment form) is the
  documented escape hatch.
- **Historical archives**: excluded as classes, not seeded as debt — dated
  plans and ADRs describe the tree as it stood; forcing them current would
  rewrite history, and 705 of 709 pre-exclusion dangles sat there.
- **Docs-only PRs skipping the cargo gates**: closed by the planner routing
  in §3.1 — without it the doc-fact tests only ran on full-scope events, so
  the PRs most likely to break them merged green and the failure landed on
  an unrelated change.
- **Changelog history loss**: closed by writing the entry on `main` before
  the cut (§2); the gate alone cannot catch it because each cut only checks
  its own version.
- **Workflow re-run reverting the site**: closed by the newest-stable-tag
  guard in `publish-docs-live` (§3.2).
- **Branch protection breaking the forced update**: the runbook's
  protection recommendation spells out the force-push allowance; protection
  configured without it would 422 the repoint.
- **cargo-dist regeneration dropping the docs-live job**: `REQUIRED_MARKERS`
  fails Code Style.
- **Dashboard repoint is invisible to CI**: weekly-checklist observable
  (changelog probe) is the compensating control.
- **A wrong page live mid-week**: the runbook's docs-hotfix recipe (§3.3)
  fixes the live site without waiting for the next stable release and
  without republishing `main`.
- **First `docs-live` publication predating the drift fixes**: the branch
  publishes the *tagged* tree, so the fixes go live at the first stable
  release cut after #7375 merged.
