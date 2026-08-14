Turns a `user_ref` into the conversation ref for a one-to-one chat with that person. It does not contact them, create anything on Telegram's side, or produce a notification — the chat becomes real when a message is sent to it. Use a `user_ref` from `telegram.resolve_user`, `telegram.list_members`, `telegram.whoami`, or a message author; never derive one from a conversation ref.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
