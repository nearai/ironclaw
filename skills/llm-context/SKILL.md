---
name: llm-context
version: "1.0.0"
description: Fetch pre-extracted web content from Brave Search for grounding LLM answers via HTTP tool with automatic credential injection
activation:
  keywords:
    - "llm context"
    - "grounding"
    - "rag"
    - "fact check"
    - "verify"
    - "cite sources"
  patterns:
    - "(?i)(ground|verify|fact.check|cite).*(source|claim|statement|answer)"
    - "(?i)(llm context|rag context|web context)"
  tags:
    - "search"
    - "grounding"
  max_context_tokens: 2000
credentials:
  - name: brave_api_key
    provider: brave
    location:
      type: header
      name: X-Subscription-Token
    hosts:
      - "api.search.brave.com"
    setup_instructions: "Get a free API key at brave.com/search/api/ (Free tier: 2,000 queries/month). Same key as Web Search."
http:
  allowed_hosts:
    - "api.search.brave.com"
---

# LLM Context Skill

You have access to the Brave LLM Context API via the `http` tool. This returns **actual page content** (text chunks, tables, code) relevant to the query — not just search result links. Use this for fact-checking, grounding answers in source material, or RAG-style retrieval.

Credentials are automatically injected — **never construct `X-Subscription-Token` headers manually**.

## API

**Fetch context for a query:**
```
http(method="POST", url="https://api.search.brave.com/res/v1/llm/context", body={"q": "query text", "count": 20, "maximum_number_of_tokens": 8192})
```

### Body Parameters (JSON)

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `q` | string | required | Search query (1–400 chars) |
| `count` | int | 20 | Max search results to consider (1–50) |
| `search_lang` | string | — | 2-letter lowercase language code |
| `country` | string | — | 2-letter uppercase country code |
| `maximum_number_of_tokens` | int | 8192 | Approx max tokens in response (1024–32768) |
| `maximum_number_of_urls` | int | 20 | Max URLs to include (1–50) |
| `maximum_number_of_snippets` | int | 50 | Max total snippets (1–100) |
| `maximum_number_of_tokens_per_url` | int | 4096 | Max tokens per URL (512–8192) |
| `maximum_number_of_snippets_per_url` | int | 50 | Max snippets per URL (1–100) |
| `context_threshold_mode` | string | — | `strict`, `balanced`, `lenient`, or `disabled` |

### Response

The `http` tool returns `body` as a parsed dict. The context content is in the response body — structured text ready for direct use in answers.

## Common Mistakes

- Do NOT add an `X-Subscription-Token` header — it is injected automatically.
- This is a **POST** request (unlike web-search which is GET).
- Use `maximum_number_of_tokens` to control response size. 8192 is a good default; increase to 32768 for deep research.
- This uses the same `brave_api_key` as the web-search skill.
