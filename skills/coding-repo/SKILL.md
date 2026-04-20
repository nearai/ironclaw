---
name: coding-repo
version: "0.1.0"
description: Drive GitHub-backed coding work — branches, tests, safe git, PRs to staging.
activation:
  keywords:
    - "PR"
    - "pull request"
    - "branch"
    - "merge"
    - "commit"
    - "review"
    - "staging"
    - "open a pr"
    - "draft pr"
    - "fix bug"
    - "fix the"
    - "ship"
  patterns:
    - "(?i)(open|create|draft)\\s+(a\\s+)?pr"
    - "(?i)(fix|resolve|address)\\s+(the\\s+)?(bug|issue|test|failure|regression)"
    - "(?i)run\\s+(tests?|cargo|npm|pytest)"
  exclude_keywords:
    - "schedule"
    - "routine"
    - "heartbeat"
  tags:
    - "coding"
    - "github"
    - "git"
  max_context_tokens: 2000
  # Only fire when the active project has a GitHub repo configured.
  # Without `github_repo` the body below is mostly wasted — `gh pr create`
  # has nothing to target and the draft-PR flow can't run. The gateway's
  # chat-send handler sets `project_has_github_repo=true` in the message
  # metadata when applicable; the skills selector filters this skill out
  # otherwise and records a feedback note so the UI can explain why.
  context:
    require_project_field: github_repo
    include_git_context: true
requires:
  bins:
    - "git"
    - "gh"
---

# Coding — GitHub Repo Workflow

You're operating in a project that is backed by a GitHub repository. The
project's `workspace_path` is the on-disk folder where code lives; the
`shell` tool already runs with that directory as `cwd`. Do not invent
alternate paths.

## Tool-call discipline

- **Act before you narrate.** If a step needs data (file contents, git
  status, CI state, test output), call a tool first. Don't describe the
  plan in prose and then forget to run it.
- **All git + GitHub work goes through `shell`** with the project's
  `workspace_path`. There is no separate `git` or `gh` tool — the
  shell tool is the single, audited dispatch point.
- **Before you modify anything, read the ground truth**: `git status`,
  `git log --oneline -10`, and the files you're about to touch. Skipping
  this is the most common cause of merge-conflict cleanup work later.

## Branching

- Never work directly on `main` / `master` / `staging`. Always create a
  feature branch first: `git checkout -b <user>/<short-desc>`.
- Naming: `<initials-or-user>/<kebab-short-desc>` (e.g.
  `ip/fix-shell-badge`). Keep it under 50 chars.
- If the user asks you to push a throwaway / smoke-test branch, prefix
  it `smoke/` or `live-smoke-<uuid>` and delete it in the same session
  when you're done — don't leave junk branches in the repo.

## Tests before commit

Run the project's test runner before you commit. Detect the runner by
reading the repo:

- `Cargo.toml` present → `cargo test` (and `cargo clippy -- -D warnings`
  if clippy is configured).
- `package.json` present → `npm test` / `pnpm test` / `bun test`
  whichever is configured.
- `pyproject.toml` / `setup.py` → `pytest`.
- Go → `go test ./...`.

Surface failures verbatim. Don't narrate around a red test. If you have
to skip the suite because it's slow or the user asked, say so
explicitly.

## Formatting before commit

- Rust: `cargo fmt` + `cargo clippy -- -D warnings`.
- JS/TS: `prettier --check` / `eslint --max-warnings=0`.
- Python: `ruff format && ruff check`.
- Go: `gofmt -w .`.

Run the formatter, re-stage, then commit. Never bypass a formatter by
hand.

## Git safety

- **Never** `--no-verify` / `--no-gpg-sign` — pre-commit hooks exist for
  a reason.
- **Never** `--force` against `main` / `master` / `staging`. `--force`
  to a branch you personally own is acceptable after a rebase, but
  prefer `--force-with-lease`.
- **Never** `git reset --hard` / `git clean -fd` without approval —
  these silently destroy local work.
- **Never** commit secrets. If `git status` shows `.env`, credentials
  files, `*.pem`, or anything in a `secrets/` directory, stop and ask.
- Commit messages: imperative mood, first line ≤ 72 chars. Body
  explains the *why*, not the *what* (git already shows the diff).

## Pull requests

- Base branch is the project's `default_branch` (defaults to `staging`
  for this repo per `project_staging_workflow`). **Never** open PRs
  against `main` without explicit user direction.
- `gh pr create --base <default_branch> --draft --title "..." --body "$(cat <<'EOF'
  ## Summary
  - <one bullet per logical change>

  ## Test plan
  - [ ] <what you verified>
  EOF
  )"`
- Mark PRs ready (`gh pr ready`) only on explicit user ask — drafts are
  the safer default.
- When a PR already exists for the branch (`gh pr view --json state`),
  amend it rather than opening a duplicate.

## Commit trailer

Commits produced in this workflow should include this trailer so the
authorship attribution stays honest:

```
Co-Authored-By: IronClaw <noreply@ironclaw.dev>
```

## When something breaks

- If `shell` reports a non-zero exit, read the full stderr before
  retrying. "Try again" without reading the error wastes the user's
  time.
- If a pre-commit hook fails, fix the underlying cause and create a
  **new** commit. Never `--amend` against work that already left your
  local branch, and never `--no-verify` around the hook.
- If the working tree is dirty when you arrive, ask whether to stash or
  work on top — don't silently clobber the user's in-progress changes.
