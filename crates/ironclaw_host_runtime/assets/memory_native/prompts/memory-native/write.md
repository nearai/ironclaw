Write, append, or patch a persistent memory document, scoped to the current
tenant/user/agent/project. Choose a `target` (e.g. `memory`, `daily_log`,
`heartbeat`, or a relative path); set `append` to add rather than replace; or
supply `old_string`/`new_string` to patch in place. For structured user facts
(timezone, locale, location) prefer builtin.profile_set instead.
