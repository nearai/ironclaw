---
name: agent-ql
version: "1.0.0"
description: AgentQL API — AgentQL is a natural language interface that allows users to query their data us
activation:
  keywords:
    - "agent-ql"
    - "agentql"
    - "ai"
  patterns:
    - "(?i)agent.?ql"
  tags:
    - "ai"
    - "machine-learning"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AGENTQL_API_KEY]
---

# AgentQL API

Use the `http` tool. API key is automatically injected via `X-API-Key` header — **never construct auth headers manually**.

> AgentQL is a natural language interface that allows users to query their data using plain English, enabling seamless interaction with databases and APIs without writing traditional code.

## Authentication

This integration uses **API Key** authentication via the `X-API-Key` header.
Format: `X-API-Key: ...`

## Required Credentials

- `AGENTQL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
