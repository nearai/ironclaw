Slack offers no server-side people search to a user token, so this scans one page of the workspace directory per call and matches `query` against display name, real name, and handle. Deactivated accounts and bots are skipped.

Because it scans rather than searches, an absent match does not mean the person does not exist — follow `next_cursor` to keep scanning before concluding anything. Raise `limit` when a common query truncates.

Feed a match's `user_ref` into `slack.open_dm`, `slack.get_user_info`, or a `<@U…>` mention. Raw Slack IDs are for tool calls only — never include one in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
