Each message's `text` has in-text `<@U…>` mentions already resolved to human-readable `@Display Name` — use the humanized text, not raw Slack IDs, in user-facing prose. `is_self=true` marks a message the connected account itself authored.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
