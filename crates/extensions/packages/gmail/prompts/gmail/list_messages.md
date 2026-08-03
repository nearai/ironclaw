Use `gmail.list_messages` for read-only discovery of Gmail messages in the selected Google account.

Pass `query` for Gmail search syntax, `label_ids` to restrict by labels, and `page_token` or `max_results` only when paginating or limiting results.

This capability reads from the Gmail API through host HTTP egress. It requires a configured Google credential account with Gmail read scope.
