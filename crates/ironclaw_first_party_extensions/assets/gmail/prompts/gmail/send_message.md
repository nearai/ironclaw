Use `gmail.send_message` to send a new Gmail message from the selected Google account.

Pass the recipient fields, subject, and body exactly as the user requested. Do not infer recipients or send content unless the user has clearly authorized the message.

This capability performs an external write through the Gmail API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Gmail send scope.
