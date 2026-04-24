---
name: coding-repo
version: "0.2.0"
description: Drive GitHub-backed coding work — per-thread worktrees, branches, tests, safe git, PRs via github API.
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
    - "fix issue"
    - "issue"
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
  # Without `github_repo` the body below is mostly wasted — the github
  # API calls have nothing to target and the worktree/PR flow can't run.
  # The gateway's chat-send handler sets `project_has_github_repo=true`
  # in the message metadata when applicable; the skills selector filters
  # this skill out otherwise and records a feedback note so the UI can
  # explain why.
  context:
    require_project_field: github_repo
    include_git_context: true
requires:
  bins:
    - "git"
---

# Coding — GitHub Repo Workflow

You're operating in a project that is backed by a GitHub repository
(`project.github_repo` = `<owner>/<repo>`). The project's `workspace_path`
is mounted at `/project/` inside your sandbox. Do not invent alternate
paths.

**Each conversation (thread) works on its own git branch in its own
git worktree**, recorded in `thread.metadata.dev`. The host transparently
rewrites `/project/...` tool calls to target your thread's worktree, so
you can read and write files with paths like `/project/src/foo.rs` and
they land in the right checkout. You only need to see a worktree path
when you're setting it up (once per thread).

## The thread_state section

Every turn, the system message ends with a `thread_state: {...}` line
showing the current `thread.metadata`. Read it. Patch it via the
`thread_metadata_set` tool using replace-at-top-level-key semantics —
so to update one `dev` subfield, re-send the whole `dev` object.

Keep these fields current under the `dev` namespace as you progress:

```json
{"dev": {
  "repo": "nearai/ironclaw-e2e-test",
  "base": "main",
  "branch": "ip/fix-42-idor",
  "worktree": "worktrees/<thread_id_short>",
  "issue_num": 42,
  "pr_url": "https://github.com/nearai/ironclaw-e2e-test/pull/17",
  "pr_num": 17
}}
```

Update after each milestone: branch created → set `branch` + `worktree`;
issue fetched → set `issue_num`; PR opened → set `pr_url` + `pr_num`.
The gateway shows these as pills on the conversation header.

## Step 1: Set up your worktree (once per thread)

On your first tool call in a new coding thread:

1. Read `thread_state`. If `dev.worktree` is already set, skip this step.
2. Clone the repo into `/project/` if empty:
   ```
   shell(command="[ -d .git ] || git clone https://github.com/<owner>/<repo>.git .", workdir="/project/")
   ```
3. Pick a thread-local worktree dir under `worktrees/`. Use the first 8
   chars of the thread id (visible in the UI / available from any
   `shell` call as `$IRONCLAW_THREAD_ID` when the harness sets it, or
   just coin a short slug if not set):
   ```
   shell(command="git worktree add worktrees/<slug> -b <branch_name> origin/<base>", workdir="/project/")
   ```
4. Call `thread_metadata_set` with the `dev` object above.

From this point on, every `/project/...` file op and every `shell`
command without an explicit workdir runs inside your worktree.

## Tool-call discipline

- **Act before you narrate.** If a step needs data (file contents, git
  status, CI state, test output), call a tool first. Don't describe the
  plan in prose and then forget to run it.
- **Read the ground truth first**: `git status`, `git log --oneline -10`,
  and the files you're about to touch.
- **Use the github extension for API-level work**, use `shell` for git
  and build/test — see "GitHub API vs shell" below.

## Branching

- Never work on `main` / `master` / `staging` directly. The worktree
  setup above enforces this: your worktree is on a fresh feature branch.
- Branch naming: `<initials-or-user>/<kebab-short-desc>` (e.g.
  `ip/fix-shell-badge`). For issue fixes, `<user>/fix-<num>-<slug>`.
- Throwaway / smoke-test branches: prefix `smoke/` and delete when done.

## Tests before commit

Detect the runner by reading the repo:

- `Cargo.toml` → `cargo test` (+ `cargo clippy -- -D warnings` if configured).
- `package.json` → `npm test` / `pnpm test` / `bun test`.
- `pyproject.toml` / `setup.py` → `pytest`.
- `go.mod` → `go test ./...`.

Surface failures verbatim. Don't narrate around a red test.

## Formatting before commit

- Rust: `cargo fmt` + `cargo clippy -- -D warnings`.
- JS/TS: `prettier --check` + `eslint --max-warnings=0`.
- Python: `ruff format && ruff check`.
- Go: `gofmt -w .`.

Run the formatter, re-stage, then commit. Never bypass a formatter by hand.

## Git safety

- **Never** `--no-verify` / `--no-gpg-sign`.
- **Never** `--force` against `main` / `master` / `staging`. Prefer
  `--force-with-lease` on your own branch after a rebase.
- **Never** `git reset --hard` / `git clean -fd` without approval.
- **Never** commit secrets (`.env`, `*.pem`, `secrets/`). Stop and ask.
- Commit messages: imperative mood, first line ≤ 72 chars. Body
  explains the *why*.

## GitHub API vs shell

Two complementary entry points — use each for what it's best at:

| Use `shell` for | Use the github http skill for |
|---|---|
| `git clone`, `git worktree`, `git checkout`, `git add`, `git commit` | issue fetch, comment, close |
| `git push` | PR create, PR review, PR merge |
| `cargo test`, formatters, build scripts | repo metadata, branch protection, labels |

The github skill (already active when you see this) documents the
endpoints — e.g.
`http(method="POST", url="https://api.github.com/repos/<owner>/<repo>/pulls", body={...})`.
Credentials are injected automatically from the `github_token` secret;
**never construct Authorization headers manually**.

## Pull requests

- Base: `project.default_branch` (defaults to `staging` if unset; `main`
  otherwise per project config). Never open PRs against `main` without
  explicit user direction if `staging` exists.
- Open as **draft** by default:
  ```
  http(method="POST", url="https://api.github.com/repos/<owner>/<repo>/pulls",
       body={"title": "...", "body": "...", "head": "<branch>",
             "base": "<default_branch>", "draft": true})
  ```
- On success, capture `html_url` and `number` from the response and
  patch `thread.metadata.dev.pr_url` + `pr_num` via `thread_metadata_set`.
- If a PR already exists for the branch
  (`GET /repos/.../pulls?head=<owner>:<branch>`), update it instead of
  opening a duplicate.
- Mark ready (`PATCH /pulls/<n>` with `{"draft": false}`) only on
  explicit user ask.

## Commit trailer

Commits produced in this workflow include:

```
Co-Authored-By: IronClaw <noreply@ironclaw.dev>
```

## When something breaks

- Non-zero `shell` exit — read the full stderr before retrying.
- Pre-commit hook fails — fix the underlying cause and create a **new**
  commit. Never `--amend` against already-pushed work. Never
  `--no-verify` around the hook.
- Dirty working tree you didn't create — ask whether to stash or work
  on top.
