# acme.send_note

Send a note into an Acme Messenger conversation as the connected user.

- Use only when the job explicitly requires posting into Acme Messenger as a
  side effect.
- Never use this tool to deliver the final answer; the host delivers final
  replies on the conversation's outbound channel.
- `conversation_id` comes from context or an earlier listing; do not invent
  ids.
