`text` is Slack mrkdwn and replaces the entire message body — Slack has no partial edit, so include everything the message should still say, not just the change.

Only messages the connected account wrote can be edited, and a workspace may close the edit window some time after posting; either refusal comes back as `messaging.edit_not_allowed`. Take `message_ref` from a prior `slack.send_message` result or a read — never assemble one by hand.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
