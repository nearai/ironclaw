Send a message as the user to a channel or DM. The message appears to come
from the user's own Slack account — this is delegated authority, so use it
only for side effects the user asked for inside the current job (for example,
"DM Sergey this joke").

Never use this tool to deliver your final answer or report results back to
the user. Final replies and notifications are delivered by the host on the
user's outbound channels (the Slack bot, WebChat, push) after the turn
completes — just finish the turn with your answer. If you find yourself
sending the user a Slack message that says what you were going to say anyway,
stop and answer normally instead.

To notify someone, encode the mention as `<@U…>` with their real user ID; a
plain `@name` does not notify. Never guess a user ID or derive one from a
channel or DM conversation ID. When a DM conversation ID is known, call
`slack.get_conversation_info` with that exact ID and use the returned
conversation's `user` field as the authoritative mention target. When only a
name is known, call `slack.list_conversations` to discover and match the DM.

The host selects this operation from the capability id. Provide only the
parameters described by the input schema; do not include an action field.
