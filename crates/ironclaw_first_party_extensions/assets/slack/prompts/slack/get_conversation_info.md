Retrieve one exact Slack conversation by its conversation ID. Use this operation whenever the conversation ID is already known; do not scan `slack.list_conversations` for a known ID.

For a DM, the returned `conversation.user` is the authoritative raw user ID of the counterpart. Use it for follow-up tool calls or `<@U…>` mention encoding. Use `conversation.user_display_name` in user-facing text. Never derive a user ID from the DM conversation ID, and never expose raw Slack IDs in the final reply.

The host selects this operation from the capability id. Provide only the `channel` parameter described by the input schema; do not include an action field.
