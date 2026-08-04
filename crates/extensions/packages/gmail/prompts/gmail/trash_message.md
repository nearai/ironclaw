Use `gmail.trash_message` to move an existing Gmail message to trash when the user has clearly identified the message.

Pass `message_id` exactly. Use ids from `gmail.list_messages` or `gmail.get_message`; do not trash a message based only on an ambiguous description.

This capability performs an external write through the Gmail API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Gmail modify scope.
