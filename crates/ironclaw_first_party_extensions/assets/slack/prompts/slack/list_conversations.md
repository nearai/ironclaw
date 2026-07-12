List channels, private channels, DMs, and group DMs visible to you, not only conversations you belong to. A conversation's `is_member=true` is the authoritative membership signal. Use this to discover conversation IDs for subsequent tool calls.

DM entries carry the counterpart's `user_display_name`; use that display name in user-facing output. Raw Slack IDs are only for subsequent tool calls; never include them in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
