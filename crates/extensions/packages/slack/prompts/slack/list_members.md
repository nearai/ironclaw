Works on channels, group DMs, and DMs. Display names are resolved for a bounded prefix of each page; members past that budget come back as a `user_ref` with no `display_name`, so page with a smaller `limit` when every name matters.

Feed a member's `user_ref` into `slack.open_dm`, `slack.get_user_info`, or a `<@U…>` mention. Raw Slack IDs are for tool calls only — never include one in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
