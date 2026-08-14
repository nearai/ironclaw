Deletes a single message by its exact `message_ref`. Where the account is allowed to, Telegram removes it for everyone; otherwise it disappears only for the linked account. A message that no longer exists is reported as `messaging.unknown_message` — the result never claims a delete that did not happen.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
