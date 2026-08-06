Slack search operators: `from:me` for your own messages (NOT `from:@me` — there is no user literally named "me", so `@me` returns zero results); `from:@username`, `in:#channel`, `in:@username` (a DM), `after:2024-01-01`, `before:2024-01-31`, `has:link`. Combine with plain keywords.

Matches' authors and in-text mentions are already resolved to human-readable `@Display Name` — use the humanized fields, not raw Slack IDs, in user-facing output.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
