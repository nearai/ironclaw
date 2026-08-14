Slack has no single-message endpoint, so this reads the conversation at that exact timestamp and falls back to the thread when the message is a threaded reply. Only an exact timestamp match is ever returned — a `message_ref` that no longer resolves comes back as `messaging.unknown_message`, never as the neighbouring message.

Take `message_ref` from a prior send or read. Raw Slack IDs are for tool calls only — never include one in a reply.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
