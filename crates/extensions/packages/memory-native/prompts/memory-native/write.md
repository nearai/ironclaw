Write, append, or patch a persistent memory document, scoped to the current
tenant/user/agent/project. Use this to save a durable user preference, fact,
decision, or correction so it is still available in future conversations —
target `memory` with `append` set is the place for those one-line facts.
Choose a `target` (e.g. `memory`, `daily_log`, `heartbeat`, or a relative
path); set `append` to add rather than replace; or supply
`old_string`/`new_string` to patch in place. For structured user facts
(timezone, locale, location) prefer ironclaw.memory.profile_set instead.
