Use `gmail.get_message` for read-only retrieval of one Gmail message when the message id is known.

Pass `message_id` exactly. Use the message ids returned by `gmail.list_messages` when the user asks to inspect one result from a search.

This capability reads from the Gmail API through host HTTP egress. It requires a configured Google credential account with Gmail read scope.
