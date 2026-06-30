Use `gmail.send_message` to send a new Gmail message from the selected Google account.

Pass `message.to` and `message.body` exactly as the user requested. Include `message.subject` when the user provides one or when a concise subject is clear from the requested body. Use `message.cc`, `message.bcc`, or `message.from` only when the user explicitly provides those fields. Use `message.raw` only when the user or system provides a complete pre-encoded Gmail raw payload. Do not infer recipients or send content unless the user has clearly authorized the message.

This capability performs an external write through the Gmail API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Gmail send scope.
