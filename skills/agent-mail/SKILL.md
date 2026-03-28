---
name: agent-mail
version: "1.0.0"
description: AgentMail API — Agent Mail enables developers to give AI agents unique
activation:
  keywords:
    - "agent-mail"
    - "agentmail"
    - "ai"
  patterns:
    - "(?i)agent.?mail"
  tags:
    - "ai"
    - "machine-learning"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AGENT_MAIL_API_KEY]
---

# AgentMail API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Agent Mail enables developers to give AI agents unique, programmable email inboxes that can send, receive, and act on emails at scale—featuring API-first integration, custom domains, and built-in deli

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `AGENT_MAIL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
