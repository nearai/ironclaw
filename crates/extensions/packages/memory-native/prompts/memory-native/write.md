Write, append, or patch a persistent memory document, scoped to the current
tenant/user/agent/project. Use this to save a durable user preference, fact,
decision, or correction so it is still available in future conversations —
target `memory` with `append` set is the place for those one-line facts.
Write each one as a declarative fact about the user ("User prefers concise
responses"), never as an instruction to yourself ("Always respond
concisely"): saved text re-enters your context every later turn, where an
imperative reads as a standing directive. Do not save task progress, session
outcomes, or short-lived artifacts like PR numbers and commit SHAs; if it
will be stale within a week or two, it does not belong here.
Choose a `target` (e.g. `memory`, `daily_log`, `heartbeat`, or a relative
path); set `append` to add rather than replace; or supply
`old_string`/`new_string` to patch in place. For structured user facts
(timezone, locale, location) prefer ironclaw.memory.profile_set instead.
For a full-document rewrite based on an earlier read, pass that read's
`content_hash` as `expected_content_hash`. If the result is `conflict`, the
document changed in the meantime. Re-read it and re-apply the intended change
instead of overwriting the newer content. Conditional writes are best-effort
for providers that do not support document versions; the native provider
enforces them.
