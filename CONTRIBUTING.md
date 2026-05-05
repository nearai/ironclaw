# Contributing

## Getting Started

```bash
git clone https://github.com/nearai/ironclaw.git
cd ironclaw
./scripts/dev-setup.sh
```

This installs the Rust toolchain, WASM targets, git hooks, and runs initial checks.

## How to Contribute

- Bug fixes, docs improvements, and focused cleanup tied to a concrete problem are welcome.
- Search existing issues and PRs before opening a new one to avoid duplicates.
- Keep changes scoped. One bug, one feature, or one documentation improvement per PR.

### Creating Issues

Open an issue when you are reporting a bug, proposing a feature, or documenting a gap in behavior.

For bug reports, include:

- What you expected to happen
- What actually happened
- Clear reproduction steps
- Relevant logs, screenshots, or error output
- Environment details when they matter (OS, database backend, feature flags, commit/branch)

For feature requests:

- Open an issue first before writing code
- Explain the problem being solved, not just the implementation idea
- Wait for maintainer feedback before investing in a large PR

We require an issue for new features so maintainers can prioritize the work and confirm it fits the roadmap before anyone spends time implementing it.

### Fixing Bugs

- Small, targeted bug-fix PRs are welcome
- If there is already an issue, link it in your PR
- If the bug is non-trivial, security-sensitive, or changes behavior across subsystems, open or confirm an issue first so the approach can be aligned before implementation

### Refactor-Only PRs

Refactor-only PRs are not accepted from contributors outside the core team. If a refactor is necessary to land a bug fix or approved feature, keep it minimal and clearly tied to that change.

## Development Workflow

```bash
cargo fmt                                                    # format
cargo clippy --all --benches --tests --examples --all-features  # lint (zero warnings)
cargo test                                                   # unit tests
cargo test --features integration                            # + PostgreSQL tests
```

These commands are for day-to-day iteration while you are developing locally. The pre-submission checks below are intentionally stricter and use CI-style flags so you can catch formatting drift and clippy warnings before requesting review.

## Before You Open a PR

Run the local validation checks required before requesting a review. These are stricter than the commands for iterative development:

```bash
cargo fmt --all -- --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo build
cargo test
```

Also run this when your change touches database-backed or integration behavior:

```bash
cargo test --features integration
```

Before asking for review:

- Build and exercise the changed path locally, not just the narrowest unit test
- Keep the PR focused and avoid mixing unrelated concerns
- Fill out the PR template with a clear summary, validation notes, and impact assessment
- If your change affects tracked behavior, update `FEATURE_PARITY.md` in the same branch
- If onboarding or setup behavior changes, update the relevant setup docs in the same branch
- If you are using a coding agent and it supports them, run `review-pr` or `pr-shepherd --fix` before opening or updating the PR
- `codex review --base origin/main` is also encouraged before requesting review

## Review Follow-Through

Review conversations are author-owned.

- Address each review comment with a code change or a clear explanation
- Resolve conversations you have handled; leave them open only when reviewer judgment is still needed
- Do not leave review cleanup for maintainers when the follow-through belongs to the author

If a PR is stale for more than 48 hours after review feedback is posted, maintainers may take over the follow-up work and land the changes needed to accomplish the original PR or issue intent.

## Code Style

- Zero clippy warnings policy
- No `.unwrap()` or `.expect()` in production code (tests are fine)
- Use `thiserror` for error types, map errors with context
- Prefer `crate::` for cross-module imports
- Comments for non-obvious logic only

See `CLAUDE.md` for full style guidelines.

## Feature Parity Requirement

When your change affects a tracked capability, update `FEATURE_PARITY.md` in the same branch.

### Required before opening a PR

1. Review the relevant parity rows in `FEATURE_PARITY.md`.
2. Update status/notes if behavior changed.
3. Include the `FEATURE_PARITY.md` diff in your commit when applicable.

## Review Tracks

All PRs follow a risk-based review process:

| Track | Scope | Requirements |
|-------|-------|-------------|
| **A** | Docs, tests, chore, dependency bumps | 1 approval + CI green |
| **B** | Features, maintainer-requested refactors, new tools/channels | 1 approval + CI green + test evidence |
| **C** | Security (`src/safety/`, `src/secrets/`), runtime (`src/agent/`, `src/worker/`), database schema, CI workflows | 2 approvals + rollback plan documented |

Select the appropriate track in the PR template based on what your changes touch.

## Database Changes

IronClaw uses dual-backend persistence (PostgreSQL + libSQL). All new persistence features must support both backends. See `src/db/CLAUDE.md`.

## Adding Dependencies

Run `cargo deny check` before adding new dependencies to verify license compatibility and check for known advisories.

## Document your Changes

- The folder `/docs` contains user-facing documentation for technical savvy users, developers and operators. It is built with Mintlify and rendered on the website.
- For features, update the relevant capability doc in `docs/capabilities/`
- For channels, update the relevant channel doc in `docs/channels/`
- For extensions / tools, update the relevant doc in `docs/extensions/`
- Core features live in `docs/capabilities`

In case you want to document the library itself (i.e. reference documentation) for other core contributors, use the `docs/internal/` folder

If you use your Claude Code to "plan" and want to leave a record of it, use the `docs/plans` folder.

### Skills
Read the `.claude/skills/mintlify-docs` for guidelines on how to generate documentation with mintlify.

### Test the Docs
To make sure the documentation still works, do:

```bash
cd docs
mint dev
```

To make sure you did not break any internal links, do:

```bash
cd docs
mint broken-links
```

---

## Adding Skills

Skills extend IronClaw with domain-specific workflows. See `skills/SKILL_TEMPLATE.md` for the format.

### Quick Start

1. Copy `skills/SKILL_TEMPLATE.md` to `skills/<name>/SKILL.md`
2. Fill in the template with your skill's workflow
3. Validate: `cargo test -p ironclaw_skills`
4. Add behavioral test: see `tests/e2e/scenarios/test_skills.py`
5. Open PR

### Skill Validation Checklist

Before opening a PR:

- [ ] Skill parses without errors (`cargo test -p ironclaw_skills`)
- [ ] `activation` block is present and non-empty
- [ ] All tools referenced in skill are available in IronClaw
- [ ] Behavioral test added to `tests/e2e/scenarios/test_skills.py`
- [ ] Description accurately reflects skill purpose

### Tool Name Reference

| Claude Code | IronClaw |
|-------------|----------|
| `Bash` | `shell` |
| `Read` | `read_file` |
| `Write` | `write_file` |
| `Edit` | `apply_patch` |
| `Grep` | `grep` |
| `Glob` | `glob` |
| `WebFetch` | `http` |
