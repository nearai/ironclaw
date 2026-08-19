This sends from the user's own Telegram account. Recipients see a personal message from them — there is no bot badge and no way for the recipient to tell it was automated. Confirm the exact recipient before sending: identify people by `@username` or the `user_ref` a tool returned, never by a display name or group title alone (both are settable by other people).

`conversation` must be a ref returned by `telegram.list_conversations`, `telegram.open_dm`, or `telegram.get_conversation_info`. Refs are opaque and are not valid on any other extension.

Telegram has a single reply mechanism, so `thread` and `reply_to` both resolve to it; supply either the thread anchor or the exact `message_ref` being answered.

If the result carries `sent_unverified: true`, the message was delivered but Telegram returned no id for it. Do not send it again. It cannot be edited, deleted, or reacted to afterwards.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
