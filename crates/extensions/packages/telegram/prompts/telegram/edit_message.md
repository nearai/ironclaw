Edits a message the linked account itself sent, using the exact `message_ref` from an earlier send or read. Telegram closes the edit window (about 48 hours in most chats) and refuses edits on other people's messages; both surface as `messaging.edit_not_allowed`.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
