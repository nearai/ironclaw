`emoji` is a Slack short name with no colons — `thumbsup`, `tada`. Wrapping colons are stripped for you; unicode characters are not accepted.

Omit `emoji` to remove every reaction the connected account added to the message. That variant reads the message's reactions first to find them, so it needs the `reactions:read` scope in addition to `reactions:write`, and it returns no `emoji` because it may have removed several.

Only the connected account's own reactions are ever removed — this cannot clear someone else's. A reaction that was not there is reported as removed rather than as an error; the requested state already holds.

The host selects this operation from the capability id. Provide only the parameters described by the input schema; do not include an action field.
