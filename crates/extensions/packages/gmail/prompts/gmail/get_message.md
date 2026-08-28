Use `gmail.get_message` for read-only retrieval of one Gmail message when the message id is known.

Pass `message_id` exactly. Use the message ids returned by `gmail.list_messages` when the user asks to inspect one result from a search.

This capability reads from the Gmail API through host HTTP egress. It requires a configured Google credential account with Gmail read scope.

On success, `body.headers` contains only useful message headers and `body.body.text` contains decoded readable content. HTML-only mail is returned as Markdown. Prefer these semantic fields over provider transport details. `body.attachments` contains metadata, not attachment bytes.

If `body.body.kind` is `encrypted`, report that the message is encrypted and unsupported; do not claim to have read or decrypted it. If readable content is truncated, use `builtin.result_read` with JSON pointer `/body/body/text` to page the stored field that is available.
