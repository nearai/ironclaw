`is_member=true` is the authoritative membership signal — Slack lists conversations visible to you, not only ones you belong to.

For a DM, `counterpart.user_ref` is the raw counterpart user ID: use it for subsequent tool calls or `<@U…>` mention encoding, never derived from the DM's own `conversation` ref.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
