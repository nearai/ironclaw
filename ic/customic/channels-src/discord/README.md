# Discord Channel for IronClaw

WASM channel for Discord integration - handle slash commands and button interactions via webhooks.

## Features

- **Slash Commands** - Process Discord slash commands
- **Button Interactions** - Handle button clicks
- **Thread Support** - Respond in threads
- **DM Support** - Handle direct messages

## Setup

1. Create a Discord Application at <https://discord.com/developers/applications>
2. Create a Bot and get the token
3. Set up Interactions URL to point to your IronClaw instance
4. Copy the Application ID and Public Key
5. Store in IronClaw secrets:

   ```bash
   ironclaw secret set discord_bot_token YOUR_BOT_TOKEN
   ```

   **Note:** The `discord_bot_token` secret is used for Discord REST API calls.
   Interaction signature verification is performed inside the Discord channel
   module and uses the channel config field `webhook_secret` (set this to your
   Discord app public key hex).

## Discord Configuration

### Register Slash Commands

```bash
curl -X POST \
  -H "Authorization: Bot YOUR_BOT_TOKEN" \
  -H "Content-Type: application/json" \
  https://discord.com/api/v10/applications/YOUR_APP_ID/commands \
  -d '{
    "name": "ask",
    "description": "Ask the AI agent",
    "options": [{
      "name": "question",
      "description": "Your question",
      "type": 3,
      "required": true
    }]
  }'
```

### Set Interactions Endpoint

In your Discord app settings, set:

- Interactions Endpoint URL: `https://your-ironclaw.com/webhook/discord`

## Usage Examples

### Slash Command

User types: `/ask question: What is the weather?`

The agent receives:

```text
User: @username
Content: /ask question: What is the weather?
```

### Button Click

When a user clicks a button in a message, the agent receives:

```text
User: @username
Content: [Button clicked] Original message content
```

## Error Handling

If an internal error occurs (e.g., metadata serialization failure), the tool attempts to send an ephemeral message to the user:

```text
❌ Internal Error: Failed to process command metadata.
```

Check the host logs for detailed error information.

## Advanced Usage
### Mention Polling

The Discord channel can also poll configured channels for `@bot` mentions.

Example channel config:

```json
{
  "require_signature_verification": true,
  "webhook_secret": "YOUR_DISCORD_PUBLIC_KEY_HEX",
  "polling_enabled": true,
  "poll_interval_ms": 30000,
  "mention_channel_ids": ["123456789012345678"],
  "owner_id": null,
  "dm_policy": "pairing",
  "allow_from": []
}
```

### Access Control

- `owner_id`: when set, only that Discord user can interact with the bot.
- `dm_policy`: `open` allows all DMs; `pairing` requires approval.
- `allow_from`: allowlist entries for DM pairing checks (`*`, user id, or username).

### Embeds

To send embeds, include an `embeds` array in the `metadata_json` field of the agent's response. The structure should match the Discord API `embed` object.

## Troubleshooting

### "Invalid Signature"

- Check that `webhook_secret` is set to your Discord app public key hex in the
  Discord channel config.
- Validation happens inside the Discord WASM channel.
- If `require_signature_verification` is `true` and `webhook_secret` is empty,
  the channel returns HTTP `500` with a configuration error.

### "401 Unauthorized"

- Check that `discord_bot_token` is set correctly in IronClaw secrets.
- Ensure the bot is added to the server.

### "Interaction Failed"

- The interaction might have timed out (Discord requires a response within 3 seconds).
- The `interactions_endpoint_url` might be unreachable.

## Building

```bash
cd channels-src/discord
cargo build --target wasm32-wasi --release
```

## License

MIT/Apache-2.0
