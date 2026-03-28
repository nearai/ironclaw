---
name: bot-star
version: "1.0.0"
description: BotStar API — A visual bot-building platform that enables businesses to design, deploy
activation:
  keywords:
    - "bot-star"
    - "botstar"
    - "chatbot builder"
  patterns:
    - "(?i)bot.?star"
  tags:
    - "tools"
    - "chatbot-builder"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BOT_STAR_API_TOKEN]
---

# BotStar API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> A visual bot-building platform that enables businesses to design, deploy, and manage AI-powered chatbots for websites, messaging apps, and customer support workflows without coding.

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `BOT_STAR_API_TOKEN` — API Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
