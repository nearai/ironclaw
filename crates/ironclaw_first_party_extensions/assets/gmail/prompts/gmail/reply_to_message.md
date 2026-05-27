Use `gmail.reply_to_message` to send a reply in an existing Gmail thread when the original message id and reply body are known.

Pass `message_id` exactly and provide the reply body. Use `gmail.get_message` first when the user asks to inspect the thread before replying.

This capability performs an external write through the Gmail API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Gmail send scope.
