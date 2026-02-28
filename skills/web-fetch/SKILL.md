---
name: web-fetch
version: 0.1.0
description: Fetch and read web content, documentation, and articles as clean Markdown.
activation:
  keywords:
    - fetch url
    - browse website
    - read article
    - look up documentation
    - check website
    - get page
    - visit url
    - download page
    - web research
    - read docs
    - open link
    - read page
  patterns:
    - "https?://"
    - "fetch.*https?://"
    - "read.*from.*web"
    - "look up.*online"
    - "check.*website"
    - "open.*link"
---

# Web Fetch

Use `web_fetch` to retrieve web pages, articles, and documentation as clean Markdown.
Use `http` for API calls (POST/PUT/DELETE, custom headers, authenticated endpoints).

## When to use `web_fetch`

- User shares a URL and asks you to read or summarize it
- You need to look up current documentation or reference material
- Research tasks: checking facts, reading articles, verifying information
- Fetching release notes, changelogs, or README files from public URLs

## Guidelines

1. **Try `memory_search` first** — If asked about a topic you may have researched before, search memory before fetching again to avoid redundant requests.

2. **HTML is auto-converted** — Responses are cleaned via Readability and converted to Markdown. Navigation, footers, and ads are stripped automatically; you receive the main article content.

3. **Summarize large content** — If `word_count` is high, extract the sections relevant to the user's question rather than returning all content verbatim.

4. **On errors**:
   - Non-200 status → report the status code and suggest the user verify the URL
   - Timeout (30 s) → the server may be slow or the URL may be invalid
   - "only https URLs are allowed" → the URL uses HTTP; ask the user for the HTTPS version

5. **No credentials in URLs** — For sites requiring authentication, ask the user to configure credentials via `tool_auth` for zero-exposure injection, then use the `http` tool with credential injection.

6. **Rate limit** — 30 requests/min, 500/hr. Avoid fetching in tight loops; if you need multiple pages, batch lookups thoughtfully.

7. **Non-HTML content** — For JSON APIs, PDFs, or plain text, use `http` instead (or note that `web_fetch` will return the raw content for non-HTML responses).
