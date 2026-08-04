List channels, private channels, DMs, and group DMs visible to you, not only conversations you belong to. A conversation's `is_member=true` is the authoritative membership signal. Use this to discover conversation IDs for subsequent tool calls.

DM entries carry both the raw counterpart `user` ID and `user_display_name`. Use `user` for subsequent tool calls or `<@U…>` mentions and the display name in user-facing output. Never derive a user ID from a DM conversation ID. Raw Slack IDs are only for subsequent tool calls; never include them in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
