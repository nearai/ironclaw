For a DM, `counterpart.user_ref` is the authoritative raw user ID of the counterpart: use it for follow-up tool calls or `<@U…>` mention encoding, never derived from the conversation ref itself.

The host selects this operation from the capability id. Provide only the `conversation` parameter described by the input schema; do not include an action field.
