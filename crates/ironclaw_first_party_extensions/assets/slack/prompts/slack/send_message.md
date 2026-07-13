Send a message as you to a channel or DM. The message appears to come from your account.

Use it when messaging someone or posting somewhere is the task the user asked for. The run's final reply (including routine/trigger results) is delivered automatically to the configured outbound delivery target; do not use this capability to deliver your reply or a routine/trigger result — it would arrive twice.

To notify someone, encode the mention as `<@U…>` with their real user ID; a plain `@name` does not notify. Never guess a user ID or derive one from a channel or DM conversation ID. If only a name or DM conversation ID is known, call `slack.list_conversations`, match the DM entry, and use its `user` field as the authoritative mention target.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
