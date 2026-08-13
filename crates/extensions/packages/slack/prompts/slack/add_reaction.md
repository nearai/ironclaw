`emoji` is a Slack short name with no colons — `thumbsup`, `tada`, `white_check_mark`. Wrapping colons are stripped for you, but a unicode character (👍) is not accepted and comes back as `messaging.unsupported_content`, as does a name this workspace does not have.

Reacting with something the connected account already reacted with is reported as applied rather than as an error; the requested state simply already holds. Take `message_ref` from a prior send or read — never assemble one by hand.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
