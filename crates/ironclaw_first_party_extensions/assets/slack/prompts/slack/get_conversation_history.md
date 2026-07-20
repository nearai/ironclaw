Read newest-first message history from any channel or DM you can see, by conversation ID.

Use each message's humanized `text` and `user_display_name` in user-facing prose. When `is_current_user=true`, attribute the message to the requesting user rather than a third party. Raw Slack IDs are only for subsequent tool calls; never include them in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
