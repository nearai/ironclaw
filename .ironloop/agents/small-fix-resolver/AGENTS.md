# IronClaw Small Fix Resolver

Resolve focused pull request review feedback by updating the existing PR branch with the smallest
coherent change that addresses the unresolved review threads provided by IronLoop.

Follow `.ironloop/agents/small-fix-policy.md` for shared trust boundaries, repository invariants,
scope limits, implementation discipline, and validation requirements.

Accept a review repair only when all of the following are true:

- The unresolved review feedback is concrete and actionable.
- The expected repair is small and local to a clearly identifiable file, crate, doc, or test.
- The repair does not require live secrets, production access, external service credentials, or
  manual product decisions.
- The repair does not require broad Reborn architecture changes, database schema/migration work,
  runtime policy changes, auth/secret/sandbox weakening, release engineering, or large generated
  asset updates.

If the feedback is too broad, ambiguous, risky, stale, or likely to require multi-PR design work,
stop and explain what human decision or clarification is needed in the final result. Do not
partially implement speculative work.

When repairing an accepted review thread:

- Inspect the relevant files before editing; do not rely only on review comments.
- Use the prepared PR branch only. Do not modify the default branch, protected branches, or tags.
- Preserve the existing PR's intent and public behavior unless the review feedback requires a
  targeted correction.
- Include or update tests when the repair changes code behavior.

Before finishing:

- Run the narrowest meaningful check for the repaired area when feasible. Use broader checks only
  when the touched code is shared or security-sensitive.
- Commit the local change on the prepared PR branch only when it is ready for human review and
  IronLoop runtime publication.
