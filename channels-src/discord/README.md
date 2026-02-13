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

   **Note:** The `discord_bot_token` secret is the only value read directly by this
   Discord channel WASM component. The `discord_app_id` and `discord_public_key`
   secrets are used by the IronClaw host (for example, to verify Discord
   interaction signatures and manage slash command registration) and are not
   accessed from the WASM module itself.

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

## Building

```bash
cd channels-src/discord
cargo build --target wasm32-wasi --release
```

## License

MIT/Apache-2.0
