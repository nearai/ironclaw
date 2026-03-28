---
name: open-router
version: "1.0.0"
description: OpenRouter API — OpenRouter is a unified API and marketplace that lets developers access, compare
activation:
  keywords:
    - "open-router"
    - "openrouter"
    - "ai"
  patterns:
    - "(?i)open.?router"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [OPEN_ROUTER_API_KEY, OPEN_ROUTER_PROVISIONING_KEY]
---

# OpenRouter API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://openrouter.ai/api/v1`

## Actions

**Chat completion:**
```
http(method="POST", url="https://openrouter.ai/api/v1/chat/completions", body={"model": "anthropic/claude-sonnet-4-20250514","messages": [{"role": "user","content": "Hello"}]})
```

**List models:**
```
http(method="GET", url="https://openrouter.ai/api/v1/models")
```

**Get generation:**
```
http(method="GET", url="https://openrouter.ai/api/v1/generation?id={generation_id}")
```

## Notes

- OpenAI-compatible API format.
- Model IDs: `provider/model-name` (e.g., `anthropic/claude-sonnet-4-20250514`, `openai/gpt-4o`).
- Supports `stream: true` for streaming.
- `X-Title` header sets the app name shown to model providers.
