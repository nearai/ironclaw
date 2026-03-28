---
name: x-ai
version: "1.0.0"
description: X AI API — xAI is an artificial intelligence company focused on developing advanced AI syst
activation:
  keywords:
    - "x-ai"
    - "x ai"
    - "ai"
  patterns:
    - "(?i)x.?ai"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
    - "communication"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [X_AI_API_KEY]
---

# X AI API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.x.ai/v1`

## Actions

**Chat completion:**
```
http(method="POST", url="https://api.x.ai/v1/chat/completions", body={"model": "grok-3","messages": [{"role": "user","content": "Hello"}],"max_tokens": 1024})
```

**List models:**
```
http(method="GET", url="https://api.x.ai/v1/models")
```

## Notes

- OpenAI-compatible API format.
- Models: `grok-3`, `grok-3-mini`, `grok-2`.
- Supports `stream: true` for streaming.
- Temperature range: 0.0–2.0.
