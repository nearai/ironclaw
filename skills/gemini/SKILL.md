---
name: gemini
version: "1.0.0"
description: Gemini API — Gemini is a family of AI models created by Google DeepMind
activation:
  keywords:
    - "gemini"
    - "ai"
  patterns:
    - "(?i)gemini"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GEMINI_API_KEY]
---

# Gemini API

Use the `http` tool. API key is automatically injected as `key` query parameter.

## Base URL

`https://generativelanguage.googleapis.com/v1beta`

## Actions

**Generate content:**
```
http(method="POST", url="https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={GEMINI_API_KEY}", body={"contents": [{"parts": [{"text": "Explain quantum computing"}]}]})
```

**List models:**
```
http(method="GET", url="https://generativelanguage.googleapis.com/v1beta/models?key={GEMINI_API_KEY}")
```

**Count tokens:**
```
http(method="POST", url="https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens?key={GEMINI_API_KEY}", body={"contents": [{"parts": [{"text": "Hello world"}]}]})
```

## Notes

- API key passed as `key` query parameter.
- Models: `gemini-2.0-flash`, `gemini-2.0-pro`, `gemini-1.5-pro`, `gemini-1.5-flash`.
- Content structure: `{contents: [{parts: [{text: "..."}]}]}`.
- Supports `generationConfig` for temperature, topP, maxOutputTokens.
