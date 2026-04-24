# Live-tier tests

Live-tier scenarios exercise the coding-agent UX end-to-end against a
**real** GitHub repository — they clone the repo into a project
workspace, drive `POST /api/chat/send` with `mode=shell`, let the
coding skill run a full branch-and-draft-PR flow, and tear down after
themselves.

They are expensive (network I/O, LLM calls, GitHub API rate limits)
and intentionally quarantined:

- Gated behind `IRONCLAW_LIVE_TESTS=1` so they do not run in the
  default `cargo test` pass.
- Marked `#[ignore]` so `cargo test --features integration -- --ignored`
  is the opt-in invocation.
- Require `GH_TOKEN` with `repo` scope on the target repo (or a fork).

## Required environment

| Variable | Purpose |
|----------|---------|
| `IRONCLAW_LIVE_TESTS=1` | Enables the module. Without it, every scenario is a no-op. |
| `IRONCLAW_LIVE_REPO` | `owner/repo` slug. Defaults to `nearai/ironclaw` when unset. |
| `GH_TOKEN` | PAT with `repo` (write) scope on the target repo. |
| `OPENAI_API_KEY` (or configured LLM backend env) | Required for the skill-activation scenarios. |

## Scenarios

The scenarios mirror Stage D of `.claude/plans/*.md`:

1. **Clone + create.** Clone the live repo into the project's
   `workspace_path` after `POST /api/engine/projects` with
   `github_repo` set.
2. **Chrome populates.** Verify `ThreadInfo.project.branch`,
   `dirty=false`, `pr=None` on a fresh clone.
3. **Shell turn round-trip.** `POST /api/chat/send` with
   `content:"!git status"`, `mode:"shell"`. Assert SSE emits
   `shell_command` + `shell_output` with `exit_code=0` and that the
   turn persists in history.
4. **Blocklist enforced.** `!rm -rf /` must be blocked or
   approval-gated — never auto-executed.
5. **Project-less reject.** On a thread with no resolvable project,
   shell mode must return `409 Conflict`.
6. **Coding skill activates.** With the project active, a prompt
   referencing the branch triggers the `coding-repo` skill and the
   rendered git-context appears in the system prompt.
7. **Branch + draft-PR flow.** A prompt asking to open a draft PR
   against `staging` results in `git checkout -b …`, `git push -u
   origin …`, and `gh pr create --draft --base staging`.
8. **Per-thread override.** Two projects, one thread pinned to the
   second; `!pwd` returns the overridden workspace.

## Teardown contract

Live scenarios **must** close any PR they open and delete any branch
they pushed, even if the scenario itself fails. The teardown must run
in `afterAll` / `Drop` semantics — do not guard it behind
`assert_eq!` / `?` that would short-circuit on failure and leave the
repo dirty.

Branch names: always use `live-smoke-<uuid>` so collisions with real
work are impossible.

## Running

```bash
# Rust scenarios (1–5, 8 where applicable)
IRONCLAW_LIVE_TESTS=1 GH_TOKEN=... \
  cargo test --features integration -- --ignored live_

# Python / Playwright scenarios (6, 7 — browser-visible)
IRONCLAW_LIVE_TESTS=1 GH_TOKEN=... \
  python -m pytest tests/e2e/scenarios/live/ -v --nocapture
```

Per the `feedback_inspect_test_output` memory, scan the run output for
`[ACTION FAILED]`, `SyntaxError`, and tool errors — a green exit code
alone is not sufficient.

## CI

A separate, opt-in nightly workflow runs this tier with a dedicated
secret scope so PR-gating CI stays deterministic and fast. Do not add
live scenarios to the PR workflow.
