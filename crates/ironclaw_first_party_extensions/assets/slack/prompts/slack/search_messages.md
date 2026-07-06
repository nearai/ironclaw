Search across all messages you can see (your DMs, group DMs, and channels you belong to), using Slack's `search.messages`.

Query syntax (Slack search operators):
- To find messages YOU sent, use `from:me` (NOT `from:@me` — there is no user literally named "me", so `@me` returns zero results).
- Other operators: `from:@username`, `in:#channel`, `in:@username` (a DM), `after:2024-01-01`, `before:2024-01-31`, `has:link`. Combine with plain keywords.
- Set `sort` to `timestamp` to get the most recent matches first (default is relevance).

Finding "my last message": `search.messages` indexes content and is unreliable for "the single most recent message I sent" — prefer `list_conversations` to find the relevant conversation, then `get_conversation_history` (newest first) to read its latest messages. Use search when looking for messages by keyword/person/channel.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
