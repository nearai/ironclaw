## Summary

<!-- 2-5 bullet points: what changed and why -->

-

## Change Type

<!-- Check all that apply. Refactor-only PRs are for core team or maintainer-requested work. -->

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor
- [ ] Documentation
- [ ] CI/Infrastructure
- [ ] Security
- [ ] Dependencies

## Linked Issue

<!-- Closes #N, Fixes #N, Related #N, or "None". New feature PRs must link an approved issue. -->

## Validation

<!-- How did you verify this works? -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`
- [ ] `cargo build`
- [ ] Relevant tests pass: <!-- list specific tests -->
- [ ] `cargo test --features integration` if database-backed or integration behavior changed
- [ ] Manual testing: <!-- describe what you tested -->
- [ ] If a coding agent was used and supports it, `review-pr` or `pr-shepherd --fix` was run before requesting review

## Security Impact

<!-- Does this change affect: permissions, network calls, secrets, file access, tool execution, sandbox policy? If yes, describe. If no, write "None". -->

## Database Impact

<!-- Does this add/modify migrations, change schema, or affect both PostgreSQL and libSQL? If yes, describe. If no, write "None". -->

## Blast Radius

<!-- What subsystems does this touch? What could break? -->

## Rollback Plan

<!-- How to revert if this causes problems? For Track C changes, this is mandatory. -->

## Review Follow-Through

<!-- Review conversations are author-owned. Summarize any known follow-up or areas where reviewer judgment is still needed. -->

---

**Review track**: <!-- A (docs/tests/chore) | B (feature/maintainer-requested refactor) | C (security/runtime/DB/CI) -->
