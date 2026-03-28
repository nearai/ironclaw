---
name: deep-seek
version: "1.0.0"
description: DeepSeek API — DeepSeek develops advanced large language models that deliver high-performance A
activation:
  keywords:
    - "deep-seek"
    - "deepseek"
    - "ai"
  patterns:
    - "(?i)deep.?seek"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DEEP_SEEK_API_KEY]
---

# DeepSeek API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.deepseek.com`

## Actions

**Chat completion:**
```
http(method="POST", url="https://api.deepseek.com/chat/completions", body={"model": "deepseek-chat","messages": [{"role": "user","content": "Hello"}],"max_tokens": 1024})
```

**List models:**
```
http(method="GET", url="https://api.deepseek.com/models")
```

## Notes

- OpenAI-compatible API format.
- Models: `deepseek-chat`, `deepseek-coder`, `deepseek-reasoner`.
- Supports `stream: true` for streaming responses.
- Temperature range: 0.0–2.0.
