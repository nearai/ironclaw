---
title: "Telegram"
description: "Interact with your agent through Telegram"
icon: telegram
---

In this guide, we will show you how to set up a Telegram channel to interact with your IronClaw agent. We assume that you already have an agent up and running.

<Note>
If you haven't set up your agent yet, follow our [Quickstart guide](../quickstart)
</Note>

---

## Set up a Telegram channel

<Steps>

<Step title="Create a new bot with BotFather">

In order to create a new Telegram bot, you need to talk to [BotFather](https://t.me/botfather), the official Telegram bot that helps you create and manage your bots.

<Steps>
    <Step title="Start a conversation with BotFather">
    Search for "BotFather" in the Telegram app and start a conversation with it. You can also use this link: [https://t.me/botfather](https://t.me/botfather)
    </Step>
    <Step title="Create a new bot">
    Send the command `/newbot` to BotFather and follow the instructions to create a new bot. You will need to choose a name and a username for your bot. The username must end with "bot". For example, "my_agent_bot".
    </Step>
    <Step title="Get your bot token">
    After creating your bot, BotFather will give you a token that looks like this: `123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ`. This token is used to authenticate your bot and allow it to access the Telegram API. Keep this token safe and do not share it with anyone. You will need it later to configure the Telegram channel in IronClaw.
    </Step>
</Steps>

</Step>
<Step title="Configure the Telegram channel in IronClaw">

    Invoke the IronClaw CLI onboard wizard using the `--channels-only` flag to configure only the channels without going through the entire onboarding process again:

    ```
    ironclaw onboard --channels-only
    ```

    <Steps>
        <Step title="Config the Tunnel">
            If you have not setup a `Tunnel` yet, the wizard will ask you to choose a tunnel provider and set it up. We recommend using [ngrok](https://dashboard.ngrok.com/) for its ease of use and reliability.

            ![ngrok setup](/images/channels/tunnel.png)
        </Step>
        <Step title="Install the Telegram channel">
        Select the Telegram channel from the list of available channels to install it.
        ![select channel](/images/channels/telegram-channel.png)
        </Step>
        <Step title="Add your bot token">
            Enter the bot token you got from BotFather in the previous step.
        </Step>
    </Steps>
</Step>

<Step title="Test your Telegram channel">
    Now that you have configured your Telegram channel, it is time to test it out. Start the `ironclaw` agent if it is not already running:

    ```
    ironclaw
    ```

    Send a message to your bot in Telegram. It will respond with a command that you will need to execute in the terminal to complete the channel setup:

    ```
    ironclaw pairing approve telegram <PAIRING_CODE>
    ```
</Step>

</Steps>

---

## Telegram Side Settings

<Accordion title="Privacy mode and group visibility">
Telegram bots default to Privacy Mode, which limits what group messages they receive.If the bot must see all group messages, either:

    - disable privacy mode via `/setprivacy`, or
    - make the bot a group admin.

When toggling privacy mode, remove and re-add the bot in each group so Telegram applies the change.
</Accordion>
<Accordion title="Group permissions">
    Admin status is controlled in Telegram group settings.Admin bots receive all group messages, which is useful for always-on group behavior.
</Accordion>
<Accordion title="Helpful BotFather toggles">
    - `/setjoingroups` to allow/deny group adds
    - `/setprivacy` for group visibility behavior

</Accordion>

---

## Configuration Options

You can configure the behavior of the Telegram channel through the
`.ironclaw/channels/telegram.capabilities.json` file, which is created
automatically after you set up the channel for the first time.

<Accordion title="Options Overview">

| Option                          | Values                         | Default   | Description                                                                     |
|---------------------------------|--------------------------------|-----------|---------------------------------------------------------------------------------|
| `dm_policy`                     | `open`, `allowlist`, `pairing` | `pairing` | Controls who can send direct messages to the bot                                |
| `allow_from`                    | user IDs                       | `[]`      | Users allowed to DM the bot when `dm_policy` is set to `allowlist`              |
| `owner_id`                      | Telegram user ID               | —         | If set, only this user can interact with the bot (DMs and group messages)       |
| `respond_to_all_group_messages` | bool                           | `false`   | Respond to all group messages                                                   |
| `bot_username`                  | username                       | —         | For group mention detection whenever `respond_to_all_group_messages` is `false` |
| `polling_enabled`               | bool                           | `false`   | Use polling instead of webhooks                                                 |
| `poll_interval_ms`              | number                         | `30000`   | Polling interval in milliseconds (only used if `polling_enabled` is `true`)     |
</Accordion>

<Note>

Remember to restart your agent after changing the configuration file for the changes to take effect

</Note>

### Direct Message Policy

The `dm_policy` option controls who can send direct messages to the bot:

- `open`: anyone can DM the bot without restrictions
- `allowlist`: only users in the `allow_from` list can DM the bot
- `pairing` **(default)**: the bot will DM back any user that contacts it with a pairing command that needs to be executed in the terminal

Related options:
- The `allow_from` option is a list of Telegram user IDs that are allowed to DM the bot when `dm_policy` is set to `allowlist`.
- The `owner_id` option restricts the bot to answer only messages from a specific Telegram user ID

<Tip> 

**User ID**

Message [@userinfobot](https://t.me/userinfobot) to get your Telegram user ID.

</Tip>

### Respond to all group messages
By default, the Telegram channel will only respond to messages that mention the bot in groups.
If you want the bot to respond to all group messages, set the `respond_to_all_group_messages`

Relevant options:
- If `respond_to_all_group_messages` is set to `false`, the bot will only respond to messages that mention it.
In this case, make sure to set the `bot_username` option with the bot's username (without the `@`)

### Polling

In case you do not want to configure a `tunnel`, you can setup the Telegram channel to poll for
new messages every certain interval of time.

To do this, set the `polling_enabled` option to `true` and configure the `poll_interval_ms` option
with the desired polling interval in milliseconds (default is 30000ms, which is 30 seconds).

### Configuration Examples

**Private Team Assistant** — mentions only, pairing for DMs:
```json
{
  "bot_username": "TeamBot",
  "respond_to_all_group_messages": false,
  "dm_policy": "pairing"
}
```

**Always-On Expert** — responds to all messages:
```json
{
  "bot_username": "DevOpsBot",
  "respond_to_all_group_messages": true,
  "allow_from": ["*"]
}
```

**Owner-Only** — personal assistant in shared groups:
```json
{
  "bot_username": "MyBot",
  "respond_to_all_group_messages": false,
  "owner_id": "12345678"
}
```

---

## Group Chat Participation

IronClaw can be configured to participate in Telegram group chats, by default the bot will only respond to commands (use `/help` to see the
list of available commands). If you want the bot to respond to mentions or all messages in a group you need to configure it.

### Adding the Bot to Groups

1. **Enable group privacy** in @BotFather:
   - Message [@BotFather](https://t.me/BotFather)
   - Send `/mybots` → Select your bot
   - Click "Bot Settings" → "Group Privacy"
   - Turn OFF "Privacy mode" (allows bot to see all messages)

2. **Add bot to a group**:
   - Open the group in Telegram
   - Add member → Search for your bot username
   - Grant admin permissions (optional but recommended)

3. **Configure `bot_username`** in IronClaw:
   ```json
   {
     "bot_username": "MyIronClawBot"
   }
   ```

### Group Trigger Modes

#### Commands & Mentions

The bot responds when a command is used, e.g. `/skills`, or when the bot is mentioned, e.g. `@MyIronClawBot what's the weather?`

Configuration:

- Set "Privacy mode" to `OFF` in @BotFather, or make the bot a group admin
- Configure the `bot_username`:

```json
{
  "bot_username": "MyIronClawBot",
  "respond_to_all_group_messages": false
}
```

Benefits:
- Respects group conversation flow
- No spam from unsolicited responses
- Users explicitly choose to engage the agent

#### Respond to All Messages

The bot processes and responds to every message in the group.

- Set "Privacy mode" to OFF in @BotFather, or make the bot a group admin
- Configure both `bot_username` and `respond_to_all_group_messages`:

Configuration:
```json
{
  "bot_username": "MyIronClawBot",
  "respond_to_all_group_messages": true
}
```

Use cases:
- Small team rooms where the agent is always helpful
- Automated moderation or summarization
- Specific-topic groups where the agent provides expertise

---

## Message Privacy

<AccordionGroup>
  <Accordion title="What the bot can see" icon="eye">
    - All messages in groups where privacy mode is disabled
    - Usernames and display names
    - Message timestamps
    - Reply chains (threading context)
  </Accordion>

  <Accordion title="What gets sent to the LLM" icon="message">
    - The message text (with @mention stripped)
    - Sender identifier (username or first name)
    - Recent conversation history in that thread
  </Accordion>

</AccordionGroup>