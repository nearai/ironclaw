Send a message as you to a channel or DM. The message appears to come from your account.

Never call this capability — or instruct a trigger to call it — for that run's own final reply when outbound delivery or `delivery_target_id` is configured. The host delivers that result automatically. Do not use this capability to deliver your reply or a routine/trigger result; it would arrive twice. Use it only when messaging someone else or posting somewhere is itself the requested task.

To notify someone, encode the mention as `<@U…>` with their real user ID; a plain `@name` does not notify. Never guess a user ID or derive one from a channel or DM conversation ID. When a DM conversation ID is known, call `slack.get_conversation_info` with that exact ID and use the returned conversation's `user` field as the authoritative mention target. When only a name is known, call `slack.list_conversations` to discover and match the DM.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
