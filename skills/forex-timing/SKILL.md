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
    - "(?i)(monitor|alert|check).*rate.*mission"
    - "(?i)rate.*exceed|threshold"
  tags:
    - finance
    - trading
    - forex
  max_context_tokens: 1500
terminal_actions:
  - validate_transfer_target
  - abound_send_wire
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
- **`abound_send_wire`** — Two-phase wire transfer. First call (with params) runs timing analysis and returns a `transfer_token`. Second call (with just `transfer_token`) executes the wire. Do NOT call `analyze_transfer` separately.
- **`forex_historical_data`** — Fetch OHLCV bars for any currency pair. Params: `from_currency`, `to_currency`, `start_date`, `end_date` (optional).

## When to Use

**CRITICAL: If the user wants to SEND money, always use `abound_send_wire` — NEVER call `analyze_transfer` directly for send/transfer/wire requests.** `abound_send_wire` runs the timing analysis internally.

- User asks "should I send now?" or "is this a good time?" (analysis only, no transfer) → call `analyze_transfer`
- User asks "can I get 86 INR per dollar?" or names a target rate → call `validate_transfer_target`
- User wants to send/transfer/wire money → call `abound_send_wire` (NOT `analyze_transfer`)
- User says "send now" / confirms after seeing analysis → call `abound_send_wire` with only the `transfer_token` from the phase 1 response (pass it exactly as-is)
- User says "wait" / declines → respond conversationally ("Got it, let's wait for a better rate")
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

## Missions & Recurring Monitoring

When the user wants to **monitor exchange rates** or get alerts on rate thresholds, create a mission with `mission_create` and set the goal to use `abound_rate_alert`:

- **`abound_rate_alert`** — Atomic check-and-notify tool. Fetches the current rate, compares against a threshold, and sends a notification if exceeded. All in one call — no parsing needed.
  Params: `threshold` (required), `from_currency` (default USD), `to_currency` (default INR), `message_id` (default rate_alert).

Example mission goal for rate monitoring:
> "Call abound_rate_alert(threshold=90) each run. Report the result via FINAL()."

**CRITICAL: For mission threads that monitor rates, always use `abound_rate_alert` — never chain `abound_exchange_rate` + `abound_create_notification` manually.** The single tool is deterministic and avoids parsing errors.

## Rules

- **Never call `analyze_transfer` before `abound_send_wire`** — the analysis is built into `abound_send_wire` and runs automatically. Calling both wastes a step and breaks the flow.
- `analyze_transfer` and `validate_transfer_target` are USD/INR only. Don't use them for other pairs.
- `forex_historical_data` works for any Massive-supported pair.
- Always uppercase currency codes (USD, INR, not usd, inr).
- Never expose raw API details, URLs, or internal field names to the user.
