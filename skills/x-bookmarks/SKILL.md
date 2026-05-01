---
name: x-bookmarks
version: "1.0.0"
description: Triage and act on X (Twitter) bookmarks. The IronClaw gateway ingests scraped bookmarks, runs an LLM triage step, persists the queue, and exposes it via authenticated HTTP endpoints. Use this skill to query the queue, trigger a triage pass, or surface "build" candidates for follow-up.
activation:
  keywords:
    - "bookmark"
    - "bookmarks"
    - "x.com"
    - "twitter"
    - "tweet"
    - "tweets"
    - "triage"
    - "queue"
  patterns:
    - "(?i)(my|the)\\s+(x|twitter|bookmark)\\s+(queue|bookmarks|triage)"
    - "(?i)(triage|review)\\s+.*(bookmarks|tweets)"
  tags:
    - "x-bookmarks"
    - "twitter"
    - "triage"
  max_context_tokens: 1500
---

# X (Twitter) Bookmarks

The IronClaw gateway owns the full bookmarks pipeline. The flow is:

1. **Scrape (external)** — a claude-in-chrome session at `x.com/i/bookmarks`
   extracts tweets to JSON. This is out of scope for IronClaw and runs on the
   user's machine at human pace.
2. **Ingest** — the scraper POSTs the batch to `/api/x-bookmarks/ingest` on
   IronClaw. Dedupe is by `(user_id, tweet_id)`.
3. **Triage** — `/api/x-bookmarks/triage` runs the configured LLM over the
   user's untriaged queue. Each bookmark is classified into one of:
   - `build` — idea/tool/technique worth implementing as a project, skill,
     script, or PR. Triage also emits a kebab-case `project_slug`.
   - `read` — long-form content to read later (essay, thread, paper).
   - `reference` — useful saved resource (docs, library, code example).
   - `dead` — meme, joke, outdated, vague, low-signal, unactionable.
4. **Queue** — the agent reads `/api/x-bookmarks/queue?status=build` to find
   actionable items.

## HTTP endpoints

All endpoints require gateway bearer auth (`Authorization: Bearer ...`).

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/x-bookmarks/ingest` | Bulk-ingest scraped bookmarks (batch up to 500). Body: `{"bookmarks": [...]}`. |
| POST | `/api/x-bookmarks/triage` | Run the configured triage LLM over the user's untriaged queue. Body: `{"limit": 50}` (optional). |
| GET  | `/api/x-bookmarks/queue?status=build&limit=50` | Return the triaged queue, filtered. |
| GET  | `/api/x-bookmarks/stats` | Aggregate counts per status. |

## Configurable triage LLM

The triage step uses IronClaw's configured `LlmProvider`. The default model
is whatever IronClaw is set to globally (e.g. NEAR AI's default or whatever
`ANTHROPIC_MODEL`/`OPENAI_MODEL`/etc. is configured). To override the triage
model without changing IronClaw's global default, choose **one** of:

1. **Per-user setting** (recommended — runtime tunable):

   ```bash
   curl -X PUT \
     -H "Authorization: Bearer $TOKEN" \
     -H "Content-Type: application/json" \
     -d '"moonshotai/kimi-k2"' \
     http://127.0.0.1:3100/api/settings/skills.x_bookmarks.triage_model
   ```

2. **Operator-wide env var**:

   ```bash
   X_BOOKMARKS_TRIAGE_MODEL=moonshotai/kimi-k2
   ```

Resolution order is settings table > env var > unset (= use global default).
The skill ships with the override **unset** so a fresh install uses the
IronClaw-wide default.

## Schema

The `x_bookmarks` table is migrated automatically. Each row is keyed on
`(user_id, tweet_id)`; multiple users can bookmark the same tweet
independently. Triage results are persisted as `status`, `rationale`,
`project_slug`, `tags`, `triaged_at`, and `triage_model`.

## Safety

This skill never touches `x.com`. The scraper is the only component that
does, and it runs under the user's own logged-in browser session. The
ingest endpoint validates each bookmark's `tweet_id` (alphanumeric, ≤64
chars), `url` (must be `https://x.com` or `https://twitter.com`), and
length-caps free-form text fields before they reach the LLM.

The triage system prompt explicitly frames bookmark text as untrusted
**data** so prompt-injection attempts inside tweets cannot redirect the
classifier.
