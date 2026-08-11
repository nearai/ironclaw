`user_ref` must be a real Slack user ID (`U…` or `W…`) from a people operation — `slack.resolve_user`, `slack.get_user_info`, `slack.list_members`, or a `counterpart.user_ref` from `slack.get_conversation_info`. Never derive one from a conversation ID, and never guess.

Slack returns the existing DM when one is already open, so calling this twice is safe and never creates a duplicate conversation. This is how "DM Sergey" becomes a usable `conversation` ref when no DM appears in `slack.list_conversations` yet.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
