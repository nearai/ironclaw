---
name: anthropic
version: "1.0.0"
description: Anthropic API — Anthropic is an AI safety and research company focused on building reliable
activation:
  keywords:
    - "anthropic"
    - "ai"
  patterns:
    - "(?i)anthropic"
  tags:
    - "ai"
    - "machine-learning"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ANTHROPIC_API_KEY]
---

# Anthropic API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

## Base URL

`https://api.anthropic.com/v1`

**Required headers**: `x-api-key: {ANTHROPIC_API_KEY}`, `anthropic-version: 2023-06-01`

## Actions

**Create message:**
```
http(method="POST", url="https://api.anthropic.com/v1/messages", headers=[{"name": "x-api-key","value": "{ANTHROPIC_API_KEY}"},{"name": "anthropic-version","value": "2023-06-01"}], body={"model": "claude-sonnet-4-20250514","max_tokens": 1024,"messages": [{"role": "user","content": "Hello"}]})
```

**Create message with system prompt:**
```
http(method="POST", url="https://api.anthropic.com/v1/messages", headers=[{"name": "x-api-key","value": "{ANTHROPIC_API_KEY}"},{"name": "anthropic-version","value": "2023-06-01"}], body={"model": "claude-sonnet-4-20250514","max_tokens": 1024,"system": "You are a helpful assistant.","messages": [{"role": "user","content": "Explain quantum computing"}]})
```

**List models:**
```
http(method="GET", url="https://api.anthropic.com/v1/models")
```

## Notes

- Always include `anthropic-version: 2023-06-01` header.
- Models: `claude-sonnet-4-20250514`, `claude-haiku-4-5-20251001`, `claude-opus-4-20250514`.
- Max tokens is required for all message requests.
- Streaming: set `stream: true` in body.
