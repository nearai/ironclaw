Use `gmail.create_draft` to create a Gmail draft without sending it.

Pass the recipient fields, subject, and body for the draft. Prefer a draft over `gmail.send_message` when the user asks to prepare, compose, or stage an email for later review.

This capability performs an external write through the Gmail API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Gmail modify scope.
