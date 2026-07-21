Search across all messages you can see (your DMs, group DMs, and channels you belong to), using Slack's indexed `search.messages`.

Query syntax (Slack search operators):
- To find messages YOU sent, use `from:me` (NOT `from:@me` — there is no user literally named "me", so `@me` returns zero results).
- Other operators: `from:@username`, `in:#channel`, `in:@username` (a DM), `after:2024-01-01`, `before:2024-01-31`, `has:link`. Combine with plain keywords.
- Set `sort` to `timestamp` to get the most recent matches first (default is relevance).

Do not use `search.messages` to answer the single newest message when the conversation is known. Prefer `list_conversations` to find the relevant conversation, then `get_conversation_history` (newest first) to read its latest messages. Use indexed search when looking for messages by keyword, person, or channel.

Matches carry `user_display_name`, and mentions in their text are already resolved to human-readable @display-names. Use those humanized fields in user-facing output. Raw Slack IDs are only for subsequent tool calls; never include them in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
