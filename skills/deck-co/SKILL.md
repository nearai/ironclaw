---
name: deck-co
version: "1.0.0"
description: Deck.co API — Deck provides developer infrastructure for accessing credentialed user data from
activation:
  keywords:
    - "deck-co"
    - "deck.co"
    - "ai"
  patterns:
    - "(?i)deck.?co"
  tags:
    - "tools"
    - "ai"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DECK_CO_CLIENT_ID, DECK_CO_CLIENT_SECRET, DECK_CO_BASE_URL]
---

# Deck.co API

Use the `http` tool. API key is automatically injected via `x-deck-client-id` header — **never construct auth headers manually**.

> Deck provides developer infrastructure for accessing credentialed user data from login‑gated websites as if there were an official API, enabling real‑time extraction, normalization, and write actions 

## Authentication

This integration uses **API Key** authentication via the `x-deck-client-id` header.
Format: `x-deck-client-id: ...`

## Required Credentials

- `DECK_CO_CLIENT_ID` — Client ID
- `DECK_CO_CLIENT_SECRET` — Client Secret / API Key
- `DECK_CO_BASE_URL` — Base URl

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
