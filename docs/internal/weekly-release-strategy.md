# Weekly Wednesday Release Strategy

## Objective

Ship one predictable production release every Wednesday from an immutable,
QA-approved release candidate while development continues on `main`.

This is a proposed operating policy. It assumes Monday-to-Monday sprints, a
high-volume `main` branch, and a two-day release-candidate stabilization window.
The release process must preserve the repository's required reviews, automated
checks, deployment approvals, and audit evidence.

## Weekly cadence

| Day | Sprint and development | Release activity |
| --- | --- | --- |
| Monday | Close the previous sprint and start the next sprint. | At the cutoff, select the latest fully green commit, create `release/YYYY-MM-DD`, build `rc.1`, and deploy it to the RC environment. |
| Tuesday | Continue the new sprint on `main`. | QA runs regression and exploratory testing. Owners resolve release blockers and publish new RCs as needed. |
| Wednesday | Continue normal development. | Run final smoke tests, record QA sign-off, approve production deployment, promote the tested artifact, canary, and monitor. |
| Thursday–Friday | Continue the sprint and merge work intended for the next release. | Monitor the Wednesday release and address production issues through the emergency process. |

The Monday cutoff is fixed and visible to the whole team. Work merged after the
cutoff belongs to the following Wednesday. Incomplete work may remain on `main`
only when it is safe and disabled behind a feature flag. Each Wednesday release
promotes at one fixed time (for example 17:00 UTC) so the team, QA lead, release
owner, and deployment approver can synchronize on a predictable window.

## Candidate and artifact rules

1. Cut the candidate only from a commit with all required `main` checks green.
   If the tip is red, use the latest known-green commit when it contains the
   intended release scope; otherwise delay the release.
2. Treat the candidate as the commit SHA, artifact digest, configuration
   version, and migration version—not merely a branch name.
3. Tag candidates as `ironclaw-vX.Y.Z-rc.N` and deploy the immutable candidate
   artifact to a stable, production-like RC environment. The `ironclaw-`
   namespace matches the tag-only cargo-dist publisher in
   `.github/workflows/ironclaw-release.yml`; a tag without that namespace never
   invokes the publisher. The release owner creates that tag with the
   `Cut Ironclaw Release` workflow, supplying the exact approved commit SHA and
   matching Cargo version. **Land the `docs/changelog.mdx` entry for the
   target stable version on `main` before the Monday cut** so the candidate
   inherits it — the cut workflow refuses a stable (non-rc) tag whose
   candidate commit has no `description="vX.Y.Z"` changelog entry
   (`scripts/ci/cut_ironclaw_release.py`). The entry must live on `main`,
   not only the release branch: the branch freezes and is never merged
   back, so an entry written only there would vanish from next week's
   candidate and the live changelog would silently drop this release. If
   the cut already happened without it, add the entry on the release branch
   *and* cherry-pick it to `main`.
4. Freeze the release branch after the cut. Do not add features, merge `main`
   into it, or rebase it. The approximately 30 PRs per day that continue landing
   on `main` are for the next release.
5. Production receives the artifact built from the exact commit QA approved.
   The tag-only publisher rebuilds the final `ironclaw-vX.Y.Z` tag from that
   same commit, so confirm the recorded artifact digest matches the approved
   candidate before promotion. If a future promotion step can reuse the approved
   artifact without rebuilding, prefer it and record which path was used.
6. A stable tag automatically repoints the `docs-live` branch at the released
   commit (`publish-docs-live` in `.github/workflows/ironclaw-release.yml`),
   which redeploys the public docs site. After promotion, verify the Mintlify
   deployment reflects the release (see "Docs publication" below). Prerelease
   tags never move `docs-live`.

## Test and promotion gates

| Stage | Required evidence |
| --- | --- |
| Pull request | Compile, lint, unit tests, and targeted integration tests selected for the change. |
| Merge queue | Short deterministic integration and critical-path smoke checks on the merged result. |
| Push to `main` | Full deterministic E2E and compatibility checks; failures make `main` red and block release cuts. |
| Release candidate | Release artifact smoke, QA regression and exploratory testing, `release-public-full`, and the upgrade canary when applicable. |
| Production | Independent deployment approval, canary smoke tests, health monitoring, and rollback readiness. |

The authoritative test tiers remain in
[`testing-playbook.md`](testing-playbook.md), and live release lanes remain in
[`live-canary.md`](live-canary.md). A red `main` receives an owner immediately;
release readiness must never be inferred from the calendar alone.

## Release-blocker fixes

Only release-blocking fixes cross into the frozen candidate:

1. Branch from `release/YYYY-MM-DD` and open a focused PR back to that branch.
2. Require normal review and relevant automated checks.
3. After merge, produce a new immutable candidate (`rc.2`, `rc.3`, and so on)
   and rerun affected tests plus the final smoke suite. Record the new
   candidate's exact configuration version, migration version, commit SHA,
   artifact digest, and test evidence, and require fresh QA sign-off for that
   specific candidate before it may be promoted.
4. Automatically open a forward-port PR containing the fix for `main`.
5. Block production promotion while the release branch contains a fix that is
   neither represented on `main` nor explicitly waived with a documented
   reason.

Repeated or material RC fixes trigger a new go/no-go decision. Prefer delaying
or recutting the candidate over accumulating a release branch that QA can no
longer reason about confidently.

## Ownership and decisions

Assign a rotating release owner and a deputy in another timezone for every
Wednesday release. Protect their capacity from Monday through Wednesday.

- **Release owner:** cuts the RC, maintains the release record, coordinates
  testing and blocker triage, publishes new candidates, runs the go/no-go
  meeting, coordinates canary and rollback, and owns timezone handoffs.
- **Deputy:** continues coordination while the primary owner is offline.
- **QA lead:** owns the test plan and records candidate sign-off.
- **Component owners:** diagnose and implement fixes in their own areas; the
  release owner is not the default fixer for the entire repository.
- **Deployment approver:** provides independent production authorization when
  required and must not approve their own change or deployment.

The QA lead records candidate sign-off, the release owner runs the go/no-go
meeting and makes the final call to promote, and the deployment approver
provides the independent production authorization. Release-branch names use the
form `release/YYYY-MM-DD` so each weekly window has a unique, chronological
identifier regardless of the final version number; the versioned tag
(`ironclaw-vX.Y.Z`) carries the semantic version.

The release owner has explicit authority to delay the release. Wednesday is a
target, not permission to ship an unverified candidate.

## Release record and emergency path

The release record retains each candidate's exact configuration version,
migration version, commit SHA, and artifact digest, plus the included PRs,
automated results, QA sign-off and evidence, known issues, forward-port status,
approvals, deployment timestamps, canary outcome, and rollback instructions.
This provides the traceability needed to demonstrate that the production
artifact was tested and independently authorized.

Emergency releases are reserved for material production incidents. They keep the
same approval chain as a regular release: the two required approvals and a
documented rollback plan from `CONTRIBUTING.md` Track C, plus independent
deployment approval, an incident record, relevant automated tests, explicit
rollback readiness, immediate production validation, and retrospective review.
Only the release owner may authorize a waiver, and the release record must
document each approval, any waiver authority and its rationale, and rollback
readiness evidence. Missing a preferred Wednesday scope does not qualify as an
emergency.

## Docs publication

The public Mintlify docs site deploys from the **`docs-live` branch**, not
`main`, so the site always describes the latest stable release instead of
unreleased `main` behavior (issue #7317). The moving parts:

- **Automatic repoint.** The `publish-docs-live` job in
  `.github/workflows/ironclaw-release.yml` force-updates
  `refs/heads/docs-live` to the released commit on every **stable** tag —
  a pointer update, not a merge; successive stable tags need not be
  ancestor-related, and the branch's content is always exactly the tagged
  tree. The job bootstraps the branch if it does not exist yet, and moves
  it only when its own tag is the **newest** stable `ironclaw-v*` tag — so
  re-running an older release's workflow (routine after a flaky artifact
  upload) cannot silently revert the live site.
- **One-time, out-of-repo configuration.** In the Mintlify dashboard
  (Settings → Git → deployment branch), point the docs deployment at
  `docs-live`. This is the single manual configuration this pipeline cannot
  verify from CI; the post-promotion check below is the compensating
  control.
- **Post-promotion check (Wednesday).** After the stable tag lands, confirm
  the deployed site reflects the release — the changelog page is the quick
  probe, since its newest entry is the version just shipped.
- **Branch protection (recommended, with a caveat).** If you restrict who
  can push to `docs-live`, the rule **must allow force pushes** for the
  Actions actor (a ruleset with the force-push restriction left off, or a
  bypass for `github-actions`): the repoint is a forced ref update by
  design, and protection that blocks force pushes 422s the automation it
  is meant to guard. Anything pushed to the branch manually is overwritten
  at the next stable release by design.
- **Emergency manual repoint.** If the automation is unavailable and the
  site must move now:
  `git push origin +ironclaw-vX.Y.Z^{commit}:refs/heads/docs-live`.
- **Docs hotfix (wrong page live mid-week).** Fix the live site without
  waiting for the next stable release by publishing the *tagged tree plus
  the fix* — never `main`, which would republish unreleased behavior:

  ```bash
  git checkout -b docs-hotfix ironclaw-vX.Y.Z   # current stable tag
  # fix the page(s), commit
  git push origin +docs-hotfix:refs/heads/docs-live
  ```

  Land the same fix on `main` too, or the next stable release republishes
  the wrong page.
- **Changelog gate.** `scripts/ci/cut_ironclaw_release.py` refuses to create
  a stable tag when the candidate's `docs/changelog.mdx` lacks the
  release's `<Update description="vX.Y.Z">` entry (the tag's exact
  attribute — an rc-labeled entry, prose mention, or lookalike attribute
  does not satisfy it); rc tags are exempt. Land the entry
  on `main` before the Monday cut (Candidate and artifact rules, step 3).
- **Older releases.** Every tag preserves its docs tree
  (`https://github.com/nearai/ironclaw/tree/ironclaw-vX.Y.Z/docs`); the
  public changelog page links this instead of the site carrying versioned
  page sets.
