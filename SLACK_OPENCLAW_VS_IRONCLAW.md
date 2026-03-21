# Slack OpenClaw vs IronClaw

This note compares a broader Slack app manifest used with OpenClaw against the current Slack channel behavior in IronClaw.

## Summary

The current IronClaw Slack channel is narrower. It currently:

- handles `app_mention`
- handles Slack `message.*` events for DMs and threaded replies
- sends replies with `chat.postMessage`
- optionally downloads shared files from `url_private`

It does not currently implement:

- Slash Commands
- Slack interactivity payloads
- reactions
- pins
- channel rename handling
- member join/leave handling
- file uploads

## OAuth Scope Comparison

### Needed

- `app_mentions:read`
- `chat:write`
- `im:history`

### Needed only if the matching feature is enabled

- `channels:history` for `message.channels`
- `groups:history` for `message.groups`
- `mpim:history` for `message.mpim`
- `files:read` if Slack file attachments should be downloaded and processed

### Not needed for the current IronClaw Slack channel

- `channels:read`
- `groups:read`
- `commands`
- `emoji:read`
- `files:write`
- `mpim:read`
- `pins:read`
- `pins:write`
- `reactions:read`
- `reactions:write`
- `users:read`
- `im:write`
- `im:write.topic`
- `mpim:write`
- `mpim:write.topic`
- `chat:write.public`
- `channels:write.topic`
- `channels:write.invites`

## Event Subscription Comparison

### Needed

- `app_mention`
- `message.im`

### Optional

- `message.channels`
- `message.groups`
- `message.mpim`

### Not needed for the current IronClaw Slack channel

- `channel_rename`
- `member_joined_channel`
- `member_left_channel`
- `pin_added`
- `pin_removed`
- `reaction_added`
- `reaction_removed`

## Socket Mode

Event subscriptions are still required in Socket Mode. Socket Mode only changes delivery transport.

- **Webhook Mode**: Slack sends subscribed events to the HTTPS Request URL
- **Socket Mode**: Slack sends the same subscribed events over the Socket Mode WebSocket

## Reactions

The current IronClaw Slack channel does not inspect or act on reactions.

That means these are not needed today:

- `reactions:read`
- `reactions:write`
- `reaction_added`
- `reaction_removed`

If a future Slack workflow uses emoji reactions as approvals or triggers, those scopes and events would become relevant.

## Slash Commands

The current IronClaw Slack channel does not implement Slack Slash Commands.

That means these are not needed today:

- `commands` scope
- slash command request URLs
- slash command manifest configuration

## Interactivity

The current IronClaw Slack channel does not process Slack interactivity payloads.

That means these are not needed today:

- `settings.interactivity.is_enabled = true`
- button, modal, shortcut, or block action configuration
