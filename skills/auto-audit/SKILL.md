---
name: auto-audit
version: 0.1.0
description: Deterministic ship-readiness audit for autonomous coding work. Combines the latest ironclaw verify state, local git diff/status, and GitHub PR checks into a clear ship/no-ship verdict.
activation:
  keywords:
    - auto-audit
    - audit my work
    - ship readiness
    - ready to ship
    - PR audit
    - review readiness
    - can I merge
    - autonomous PR review
  patterns:
    - "(?i)(audit|review).*(work|changes|PR|pull request)"
    - "(?i)(ready|safe).*(ship|merge)"
    - "(?i)(ship|merge).*(audit|check|readiness)"
  tags:
    - developer
    - verification
    - review
  max_context_tokens: 1600
requires:
  bins:
    - git
    - gh
---

# Auto Audit

Use this skill when autonomous work needs a final ship/no-ship gate after tests have run. The core command is:

```bash
ironclaw audit --target <repo> --compact
```

`ironclaw audit` reads `.autoverify.state.json`, checks that the recorded `git_head` matches the current HEAD, checks the current git status and diff against a base revision, confirms the PR head matches local HEAD, and inspects GitHub PR checks when `gh` is available. Its verdicts are:

- `ship`: verification passed, the worktree is clean, diff checks pass, and PR checks are green
- `needs_review`: no blocker, but a warning remains, such as pending checks or unavailable PR metadata
- `blocked`: verification failed or is missing, the worktree is dirty, diff checks failed, the PR is draft/closed, or checks failed

## Operating Loop

1. Run the meaningful verification tier first:

```bash
ironclaw verify --target . --upto replay --compact
```

2. Commit the verified tree, then audit it:

```bash
ironclaw audit --target . --compact
```

3. Use strict mode before declaring the PR ready:

```bash
ironclaw audit --target . --strict
```

4. If GitHub metadata is intentionally unavailable, run a local-only audit and state the limitation:

```bash
ironclaw audit --target . --no-checks --compact
```

Do not treat `needs_review` as green. Either wait for pending checks to finish, fix the warning, or clearly record why a human should make the final call.
