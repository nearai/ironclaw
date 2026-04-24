---
name: fix-issue
version: "0.2.0"
description: Resolve a GitHub issue end-to-end — fetch, branch, implement, commit, push, open PR.
activation:
  keywords:
    - "fix issue"
    - "fix this issue"
    - "resolve issue"
    - "close issue"
    - "/fix-issue"
  patterns:
    - "(?i)fix\\s+issue\\s+#?\\d+"
    - "(?i)/fix-issue\\s+\\S+"
  tags:
    - "coding"
    - "github"
    - "issue"
  max_context_tokens: 2200
requires:
  bins:
    - "git"
---

# Fix GitHub Issue

Resolve a GitHub issue end-to-end. **Never narrate a plan with
`echo`.** Every step is a real tool call. Skip any step whose
precondition is already satisfied (read the `thread_state` line to
check).

Use the `github` skill's `http` patterns for GitHub API calls
(`github_token` credential is injected automatically on requests to
`api.github.com` — never construct Authorization headers yourself).

## Step 1 — Parse the issue reference

Extract `<owner>`, `<repo>`, and `<number>` from the user's message.
Shapes:

- URL: `https://github.com/<owner>/<repo>/issues/<number>`
- Bare: `#42` or `42` (use the active project's `github_repo` for owner/repo)

If any part is missing, stop and ask the user.

## Step 2 — Fetch the issue (+ REQUIRED metadata call)

```
http(method="GET", url="https://api.github.com/repos/<owner>/<repo>/issues/<number>")
```

If the response's `state` is `closed` or the body contains
`pull_request`, stop and tell the user. Otherwise extract `title` and
`body` for use in the PR.

**MANDATORY next call — do this BEFORE any other tool call:**
```
thread_metadata_set(patch={"dev": {
  "repo": "<owner>/<repo>",
  "issue_num": <number>,
  "issue_title": "<title from response>"
}})
```

This is not optional. The UI pills that show the user what's happening
come from `thread.metadata.dev`; skipping this call means the chrome
never updates and the feature looks dead. `thread_metadata_set` takes
one argument, `patch`, which is a JSON object merged into
`thread.metadata`. Example JSON call shape:
`{"patch": {"dev": {"repo": "x/y", "issue_num": 1, "issue_title": "..."}}}`

## Step 3 — Clone + worktree (MANDATORY before any edit)

**All shell calls in this step and every subsequent step MUST use
`workdir="/project/"` (or a sub-path of it). Never `cd /tmp`, never
clone into `/tmp/...`, never edit files outside `/project/`.** If you
catch yourself about to run `cd /<anything-outside-project>`, stop and
re-read this step.

Run these shell calls IN ORDER. Do not skip. Do not narrate. Do not
move on to Step 4 until `git worktree list` shows your worktree.

1. If `/project/` is empty, clone into it:
   `shell(command="[ -d .git ] || git clone https://github.com/<owner>/<repo>.git .")`
2. Determine base branch:
   `shell(command="git symbolic-ref refs/remotes/origin/HEAD --short 2>/dev/null | sed 's|origin/||' || echo staging")`
3. Fetch: `shell(command="git fetch origin")`
4. Compute `<branch>` = `fix/<number>-<3-5 word slug>` from the issue
   title (lowercase, hyphens, ≤ 50 chars).
5. Compute `<slug>` = first 8 chars of the thread id if available,
   otherwise equal to `<branch>` with `/` replaced by `-`.
6. Create the worktree — this is non-negotiable; do NOT edit files
   on the default branch:
   `shell(command="git worktree add worktrees/<slug> -b <branch> origin/<base>")`
7. VERIFY it was created:
   `shell(command="git worktree list")` — the output MUST contain
   `worktrees/<slug>`. If it does not, re-run step 6. If it still
   fails, report the error to the user and stop.

Record progress:
```
thread_metadata_set(patch={"dev": {
  "repo": "<owner>/<repo>",
  "issue_num": <number>,
  "issue_title": "...",
  "base": "<base>",
  "branch": "<branch>",
  "worktree": "worktrees/<slug>"
}})
```

From here every `/project/...` file op and every bare `shell()` call
lands in your worktree (the host rewrites paths transparently). You
should NEVER need to reference an absolute host path like `/tmp/...`
or `/home/...` — all work stays under `/project/worktrees/<slug>/`.

## Step 4 — Understand + research

Read the issue `body` carefully. Identify:
- **Root cause** (bugs) or **design** (features).
- **Files to change**.
- **Tests to add** — every new code path needs a happy-path test
  plus at least one error path.

Use `glob`/`grep` to find relevant code. Read the whole file for any
function you're about to modify.

## Step 5 — Implement + test

- Follow the repo's conventions (check `CLAUDE.md`, `AGENTS.md`,
  `CONTRIBUTING.md` if present).
- Edit via `apply_patch` or `file_write`.
- Run the repo's quality gate (detect from build files):
  - Rust: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test --lib`
  - JS: `npm test` (or `pnpm test`, whichever `package.json` declares)
  - Python: `ruff format && ruff check && pytest`
  - Go: `gofmt -w . && go vet ./... && go test ./...`
- Fix every failure before continuing. Don't `--no-verify` or disable
  a check to make it pass.

## Step 6 — Commit + push

1. `shell(command="git status")` — verify only expected files.
2. `shell(command="git add <files>")`
3. `shell(command="git -c user.name='IronClaw' -c user.email='noreply@ironclaw.dev' commit -m '<title>\n\nCloses #<number>\n\nCo-Authored-By: IronClaw <noreply@ironclaw.dev>'")`
4. `shell(command="git push -u origin <branch>")`

## Step 7 — Open the PR

```
http(method="POST",
     url="https://api.github.com/repos/<owner>/<repo>/pulls",
     body={
       "title": "<title> (#<number>)",
       "body": "Closes #<number>\n\n## Summary\n- ...\n\n## Test plan\n- [ ] ...",
       "head": "<branch>",
       "base": "<base>",
       "draft": true
     })
```

Read `html_url` and `number` from the response; update metadata:
```
thread_metadata_set(patch={"dev": {
  "repo": "...", "issue_num": <number>, "issue_title": "...",
  "base": "...", "branch": "...", "worktree": "...",
  "pr_url": "<html_url>",
  "pr_num": <number from PR response>
}})
```

## Step 8 — Final summary

Send **one** final message to the user naming: issue `#<number>`, PR
URL, files changed, tests added. That's the only text you should emit
outside of tool calls.
