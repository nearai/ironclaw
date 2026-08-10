Fetches one message by the exact `message_ref` an earlier send or read returned. A Telegram message id is only meaningful inside its own conversation, so both halves of the ref must come from the same result.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
