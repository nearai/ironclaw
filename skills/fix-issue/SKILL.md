---
name: fix-issue
version: "0.3.0"
description: Resolve a GitHub issue end-to-end — fetch, branch, implement, commit, push, open PR. CodeAct-first.
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
  max_context_tokens: 2500
requires:
  bins:
    - "git"
---

# Fix GitHub Issue (CodeAct)

Resolve a GitHub issue end-to-end using Python CodeAct. Every step below
is a ```repl``` block. Chain tools inside one block — don't split one
tool call per turn.

**Critical rules:**
- **Never use `apply_patch` for edits.** Read with `await read_file(path=...)`,
  edit the returned string with `.replace()`, write with
  `await write_file(path=..., content=...)`. Python string substitution
  is unambiguous and avoids the "file has not been read yet" and
  wrong-shape-params failure modes of `apply_patch`.
- **Always `workdir="/project/"`** for shell calls. Never `cd /tmp/...`.
- **The `github_token` credential is auto-injected** for
  `https://api.github.com/...` — don't construct Authorization headers.
- **All edits happen inside the thread's worktree** at
  `/project/worktrees/<slug>/`. The host rewrites `/project/` paths into
  your worktree transparently once metadata is set.
- **CodeAct has a 30s execution cap.** Long-running shell commands
  (`cargo check`, `cargo test`, `npm test`, `pytest`) MUST be issued as
  **plain Tier-0 tool calls** — your turn emits a `shell` tool_call
  directly, NOT a ```repl``` block. Fast ops (file read/write/replace,
  git status, git add) are fine inside ```repl``` because they return
  in milliseconds. When in doubt, run slow shells as their own turn.
- **Never narrate with `echo`.** `echo "about to parse issue URL"` is
  a wasted LLM call. Go straight to the real tool.

## Step 1 — Parse + fetch + record metadata (one block)

```repl
# Parse the user's message — URL shape or bare number.
# Owner/repo/number extraction left for the LLM; then:

issue = await http(
    method="GET",
    url=f"https://api.github.com/repos/{owner}/{repo}/issues/{number}",
)
# Bail out early on closed issues or PR links.
assert "pull_request" not in issue and issue.get("state") != "closed", \
    f"issue #{number} is not open: {issue.get('state')}"

title = issue["title"]
body = issue.get("body", "")

# MANDATORY: populate the UI pills. This is not optional.
await thread_metadata_set(patch={"dev": {
    "repo": f"{owner}/{repo}",
    "issue_num": number,
    "issue_title": title,
}})
```

## Step 2 — Clone + worktree (one block)

```repl
# Derive branch + slug.
slug_words = "-".join(title.lower().split()[:4])[:40]
branch = f"fix/{number}-{slug_words}"
# `<slug>` for the worktree directory. Pick something filesystem-safe.
wt_slug = branch.replace("/", "-")

# Clone + fetch + worktree + verify, all from /project/ root.
r1 = await shell(command="[ -d .git ] || git clone https://github.com/" + owner + "/" + repo + ".git .", workdir="/project/")
base = (await shell(command="git symbolic-ref refs/remotes/origin/HEAD --short 2>/dev/null | sed 's|origin/||' || echo staging", workdir="/project/"))["output"].strip() or "staging"
r3 = await shell(command="git fetch origin", workdir="/project/")
r4 = await shell(command=f"git worktree add worktrees/{wt_slug} -b {branch} origin/{base}", workdir="/project/")
r5 = await shell(command="git worktree list", workdir="/project/")
assert wt_slug in r5["output"], f"worktree did not appear in list: {r5['output']}"

worktree = f"worktrees/{wt_slug}"

await thread_metadata_set(patch={"dev": {
    "repo": f"{owner}/{repo}",
    "issue_num": number,
    "issue_title": title,
    "base": base,
    "branch": branch,
    "worktree": worktree,
}})
```

From here every `/project/...` file op routes into your worktree.

## Step 3 — Understand the issue

```repl
# Read repo convention files (AGENTS, CLAUDE, CONTRIBUTING) if present.
# Glob / grep for the code sites mentioned in the issue body.
# Use read_file for any file you intend to edit.
# Reference `body` from step 1 — don't re-fetch.
```

## Step 4 — Implement + test (CodeAct edits, not apply_patch)

```repl
# Pattern for every edit:
fp = "/project/src/foo.rs"
data = await read_file(path=fp)
content = data["content"]

# Use str.replace(old, new, 1) for single-site edits. The `, 1` guards
# against accidental multi-replacement. For multi-site edits, call
# .replace() multiple times with distinct contexts.
content = content.replace(
    "old exact snippet",
    "new snippet",
    1,
)
await write_file(path=fp, content=content)

# For new files, skip read_file and just call write_file with the body.
```

Run the repo's quality gate as **plain Tier-0 `shell` calls**, one per
turn — the 30s CodeAct cap would time these out. Detect from build
files: Rust → `cargo fmt && cargo clippy --all-targets -- -D warnings
&& cargo test`; JS → `npm test`; Python → `ruff check && pytest`;
Go → `go vet ./... && go test ./...`. Capture each command's output
and iterate on failures. Do NOT `--no-verify` or disable checks to
force a pass.

## Step 5 — Commit + push + open PR (one block)

```repl
# Stage only the files you edited. Avoid `git add .`.
await shell(command="git status", workdir="/project/")
await shell(command="git add <explicit files>", workdir="/project/")

commit_msg = f"{title}\\n\\nCloses #{number}\\n\\nCo-Authored-By: IronClaw <noreply@ironclaw.dev>"
await shell(
    command=f"git -c user.name='IronClaw' -c user.email='noreply@ironclaw.dev' commit -m \"{commit_msg}\"",
    workdir="/project/",
)
await shell(command=f"git push -u origin {branch}", workdir="/project/")

pr = await http(
    method="POST",
    url=f"https://api.github.com/repos/{owner}/{repo}/pulls",
    body={
        "title": f"{title} (#{number})",
        "body": f"Closes #{number}\\n\\n## Summary\\n- ...\\n\\n## Test plan\\n- [ ] ...",
        "head": branch,
        "base": base,
        "draft": True,
    },
)

await thread_metadata_set(patch={"dev": {
    "repo": f"{owner}/{repo}",
    "issue_num": number,
    "issue_title": title,
    "base": base,
    "branch": branch,
    "worktree": worktree,
    "pr_url": pr["html_url"],
    "pr_num": pr["number"],
}})

FINAL(f"Opened draft PR {pr['html_url']} for issue #{number}.")
```
