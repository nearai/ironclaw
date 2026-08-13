Deletion is permanent — Slack has no undo and no trash. Deleting a thread parent removes its replies with it.

Only messages the connected account wrote can be deleted; anyone else's comes back as `messaging.permission_denied`. Take `message_ref` from a prior `slack.send_message` result or a read — never assemble one by hand.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
