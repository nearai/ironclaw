`text` is Slack mrkdwn. To notify someone, encode the mention as `<@U…>` with their real user ID; a plain `@name` does not notify. Never guess a user ID or derive one from a conversation ID. When a DM conversation ID is known, call `slack.get_conversation_info` with that exact ID and use the returned `counterpart.user_ref` as the authoritative mention target. When only a name is known, call `slack.list_conversations` to discover and match the DM.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
