---
name: openai
version: "1.0.0"
description: OpenAI API — completions, embeddings, images, audio, moderation
activation:
  keywords:
    - "openai"
    - "openai api"
    - "dall-e"
    - "whisper"
    - "gpt"
  exclude_keywords:
    - "anthropic"
    - "gemini"
  patterns:
    - "(?i)openai.*(api|completion|embedding|image|audio)"
    - "(?i)(generate|create).*(image|embedding).*openai"
    - "(?i)dall.?e"
  tags:
    - "ai"
    - "llm"
    - "image-generation"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [OPENAI_API_KEY]
---

# OpenAI API

Use the `http` tool. Credentials are automatically injected for `api.openai.com`.

## Base URL

`https://api.openai.com/v1`

## Actions

**Chat completion:**
```
http(method="POST", url="https://api.openai.com/v1/chat/completions", body={"model": "gpt-4o", "messages": [{"role": "system", "content": "You are a helpful assistant."}, {"role": "user", "content": "Explain quantum computing in one paragraph."}], "temperature": 0.7, "max_tokens": 500})
```

**Create embedding:**
```
http(method="POST", url="https://api.openai.com/v1/embeddings", body={"model": "text-embedding-3-small", "input": "The quick brown fox"})
```

**Generate image (DALL-E):**
```
http(method="POST", url="https://api.openai.com/v1/images/generations", body={"model": "dall-e-3", "prompt": "A serene mountain lake at sunset", "size": "1024x1024", "n": 1, "quality": "standard"})
```

**Transcribe audio (Whisper):**
Requires multipart upload — use file upload approach.

**Text moderation:**
```
http(method="POST", url="https://api.openai.com/v1/moderations", body={"input": "Text to check"})
```

**List models:**
```
http(method="GET", url="https://api.openai.com/v1/models")
```

## Notes

- Chat models: `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1-mini`, `o1-preview`.
- Image sizes: `1024x1024`, `1792x1024`, `1024x1792` (DALL-E 3).
- Embedding dimensions: 1536 (text-embedding-3-small), 3072 (text-embedding-3-large).
- Responses include `usage` with `prompt_tokens`, `completion_tokens`, `total_tokens`.
- Rate limits vary by model and tier. Check `x-ratelimit-remaining-*` headers.
- For structured output: use `response_format: {"type": "json_object"}`.
