---
name: forex-timing
version: "0.2.0"
description: USD/INR forex transfer timing — behavioral guidance for the built-in forex analysis tools.
activation:
  keywords:
    - remittance
    - forex
    - transfer
    - exchange rate
    - usd
    - inr
    - rupee
    - dollar
    - india
    - indian
    - send money
    - wire
    - analyze transfer
    - validate rate
  patterns:
    - "(?i)(send|transfer|remit|convert|wire)\\s.*(usd|dollar|inr|rupee|india)"
    - "(?i)(send|transfer|remit|wire).*\\$"
    - "(?i)(good|best|right|optimal)\\s.*(time|moment|rate).*(transfer|send)"
    - "(?i)analyze.*transfer"
    - "(?i)validate.*rate"
    - "(?i)(hit rate|probability|cone|volatility).*forex"
  tags:
    - finance
    - trading
    - forex
  max_context_tokens: 1500
terminal_actions:
  - analyze_transfer
  - validate_transfer_target
credentials:
  - name: massive_api_key
    provider: massive
    location:
      type: bearer
    hosts:
      - "api.massive.com"
    setup_instructions: "Get an API key at massive.com. Set with: ironclaw secret set massive_api_key <YOUR_KEY>"
---

# Smart Remittance Skill

Use the built-in forex tools for USD/INR transfer analysis. Do NOT write Python code or use the `repl` tool — the math is handled by the tools.

## Available Tools

- **`analyze_transfer`** — Recommend whether to transfer USD→INR now or wait. Uses volatility regime, RSI(14), and DXY momentum. Returns a message, hit rate, target rate, and 3-day projection cone. Param: `amount` (optional, USD).
- **`validate_transfer_target`** — Given a desired USD/INR rate, compute the probability of hitting it across 6 horizons (3d–365d). Param: `target_rate` (required).
- **`forex_historical_data`** — Fetch OHLCV bars for any currency pair. Params: `from_currency`, `to_currency`, `start_date`, `end_date` (optional).

## When to Use

- User asks "should I send now?" or "is this a good time?" → call `analyze_transfer`
- User asks "can I get 86 INR per dollar?" or names a target rate → call `validate_transfer_target`
- User asks for historical data or charts → call `forex_historical_data`

## Presenting Results

Both `analyze_transfer` and `validate_transfer_target` return `{"message": "...", "plot": {...}}`:

- **`message`**: Plain-English summary — present this directly to the user.
- **`plot`**: Numeric/chart data for the frontend. Include it in your response so the UI can render charts, but don't dump raw JSON at the user.

### For `analyze_transfer`:
- Lead with the recommendation (transfer now vs. wait)
- Show the current rate and target rate
- Mention the regime (volatility, RSI, DXY direction)
- If `could_save` is present and positive, highlight potential savings
- Show the projection cone data for the frontend to render

### For `validate_transfer_target`:
- Show the required move percentage
- Present the horizon table (which horizons have reasonable probability)
- Highlight the recommended horizon if one exists

## Rules

- `analyze_transfer` and `validate_transfer_target` are USD/INR only. Don't use them for other pairs.
- `forex_historical_data` works for any Massive-supported pair.
- Always uppercase currency codes (USD, INR, not usd, inr).
- Never expose raw API details, URLs, or internal field names to the user.
