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
  per stable release, written at the Monday cut and enforced by the cut
  script.

## 3. Architecture — three layers

### 3.1 PR-time static gates (Code Style)

| Gate | What it pins | Where |
| --- | --- | --- |
| `scripts/ci/check-guidance.py` (docs surface) | Every backticked repo path in published pages, the `zh/` mirror, and `docs/reborn/contracts/` resolves against `git ls-files`. Mintlify link targets are a different namespace and deliberately unchecked; dated archives (`docs/internal/`, non-contract `docs/reborn/`) are excluded as classes. MDX comment form of the suppress marker: `{/* check-guidance: path-ok */}`. | `fast-checks` job, `has_guidance` trigger (probes pinned in `ws12_workflow_contracts.py`) |
| `crates/app/ironclaw_cli/tests/docs_cli_reference.rs` | `docs/using/cli.mdx` ↔ the real binary's `--help`, both directions, subcommand granularity, alias-aware, row-count floor. | `cargo test -p ironclaw` |
| `crates/extensions/ironclaw_extension_registry/tests/docs_manifest_schema_version.rs` | Zero retired `reborn.extension_manifest.v2` literals in published pages (fences included); `building-a-tool.md` names `MANIFEST_SCHEMA_VERSION_V3` and documents `origin_gate_matrix`. | `cargo test -p ironclaw_extension_registry` |
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

### 3.2 Release-time (the cut and publish chain)

- **Changelog gate** — `scripts/ci/cut_ironclaw_release.py::
  ensure_stable_changelog_entry`: a stable (non-rc) tag is refused when the
  candidate commit's `docs/changelog.mdx` lacks the release's `vX.Y.Z`
  entry. Rc cuts exempt (hotfix flow unimpeded).
- **`publish-docs-live`** — job in `.github/workflows/ironclaw-release.yml`
  after `host`, prerelease-guarded, force-updates `refs/heads/docs-live` to
  the released commit via the refs API (bootstraps the branch on first
  run). Forced by design: successive stable tags need not be
  ancestor-related; the branch is a pointer, not a history. Pinned in
  `ws12_workflow_contracts.py` `REQUIRED_MARKERS` so cargo-dist
  regeneration cannot silently drop it.

### 3.3 Human checklist

`docs/internal/weekly-release-strategy.md`: Monday cut writes the changelog
entry on the release branch; Wednesday promotion verifies the deployed site
reflects the release (the changelog page is the probe); the "Docs
publication" section holds the Mintlify dashboard configuration, the
branch-protection recommendation for `docs-live`, and the emergency manual
repoint command. The dashboard repoint is the one out-of-repo configuration
this pipeline cannot verify from CI; the post-promotion check is the
compensating control.

## 4. What is enforced where

| Doc surface | Drift class | Gate |
| --- | --- | --- |
| Any published page, `zh/`, `docs/reborn/contracts/` | dead repo path in backticks | check-guidance docs surface |
| `docs/using/cli.mdx` | missing/retired subcommand rows | `docs_cli_reference.rs` |
| `docs/extensions/building-a-tool.md` + published tree | retired schema literal; missing v3/gate-matrix teaching | `docs_manifest_schema_version.rs` |
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
6. **Contract-doc pinned-test claims** — `docs/reborn/contracts/` names
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
- **cargo-dist regeneration dropping the docs-live job**: `REQUIRED_MARKERS`
  fails Code Style.
- **Dashboard repoint is invisible to CI**: weekly-checklist observable
  (changelog probe) is the compensating control.
- **First `docs-live` publication predating the drift fixes**: the branch
  publishes the *tagged* tree, so the fixes go live at the first stable
  release cut after #7375 merged.
