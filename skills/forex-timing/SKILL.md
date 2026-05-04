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

## ⚠️ HARD FORMATTING RULE (read before answering)

**NEVER output a markdown table for any forex data. Zero exceptions.** No pipe characters `|`, no header-separator rows, no grid layouts — not for historical bars, not for horizons, not for OHLC comparisons, not for weekly summaries, not even when the user asks "can I see it in a table?". Always use a **bulleted or numbered list**, one bullet per row. Example format for multi-column data:

```
- 2026-04-15 (Wed) — close ₹93.39, high ₹93.47
- 2026-04-16 (Thu) — close ₹93.05, high ₹93.40
```

If you catch yourself about to emit `|`-separated cells, stop and rewrite as a list. This rule overrides the LLM default toward tabular rendering.

## Available Tools

When a tool is needed, use the provider's structured `tool_calls` interface. Do not print tool-call syntax, Python-style calls, JSON call blobs, `[[call_tool ...]]`, `<tool_call>`, or `<function_call>` in assistant text.

- **`analyze_transfer`** — Recommend whether to transfer USD→INR now or wait. Uses volatility regime, RSI(14), and DXY momentum. Returns a message, hit rate, target rate, and 3-day projection cone. Param: `amount` (optional, USD).
- **`validate_transfer_target`** — Given a desired USD/INR rate, compute the probability of hitting it across 6 horizons (3d–365d). Param: `target_rate` (required).
- **`abound_send_wire`** — Three-action wire transfer:
  - Phase 1 (with params): runs timing analysis, returns `transfer_token` + analysis.
  - `action='send'` (with `transfer_token`): executes the wire.
  - `action='wait'` (with `transfer_token`): creates an hourly rate monitoring mission that alerts when the target rate is reached.
  Do NOT call `analyze_transfer` separately — it's built into phase 1.
- **`forex_historical_data`** — Fetch OHLCV bars for any currency pair. Params: `from_currency`, `to_currency`, `start_date`, `end_date` (all required). **Always call the `time` tool first** to resolve today's date before invoking `forex_historical_data`, and pass concrete `start_date` / `end_date` values (YYYY-MM-DD) derived from that result — never guess.

## When to Use

**CRITICAL: If the user wants to SEND money, always use `abound_send_wire` — NEVER call `analyze_transfer` directly for send/transfer/wire requests.** `abound_send_wire` runs the timing analysis internally.

- User asks "should I send now?" or "is this a good time?" (analysis only, no transfer) → call `analyze_transfer`
- User asks "can I get 86 INR per dollar?" or names a target rate → call `validate_transfer_target`
- User wants to send/transfer/wire money → use the `abound_send_wire` tool through structured `tool_calls` (NOT `analyze_transfer`)
- User says "send now" / confirms after seeing analysis → use the `abound_send_wire` tool with the send action through structured `tool_calls`
- User says "wait" / declines → use the `abound_send_wire` tool with the wait action through structured `tool_calls`; this automatically creates an hourly rate monitoring mission using the target rate from the analysis. Present the tool's response message to the user.
- User asks for historical data or charts → use the `time` tool first, then `forex_historical_data` with explicit `start_date` and `end_date`

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
- Present the horizons as a **bulleted list** (one bullet per horizon with its probability) — do NOT render them as a markdown table
- Highlight the recommended horizon if one exists

### For `forex_historical_data`:
- **See the hard formatting rule at the top of this skill — no tables, ever.**
- **Default presentation: date + close only.** One bullet per bar, e.g. `- 2026-04-15 (Wed): ₹93.39`. Use the `weekday` field returned by the tool — never compute the day-of-week yourself, LLMs get it wrong.
- Only show additional OHLC fields (open / high / low) when the user **explicitly asks** for them (e.g. "show me open and close", "what was the daily range?"). When multiple fields are shown, still use a bullet list: `- 2026-04-15 (Wed) — close ₹93.39, high ₹93.47`. NOT a table.
- Volume is almost never useful in chat — omit unless asked.

## Missions & Recurring Monitoring

When the user wants to **monitor exchange rates** or get alerts on rate thresholds, create a mission with `mission_create` and set the goal to use `abound_rate_alert`:

- **`abound_rate_alert`** — Atomic check-and-notify tool. Fetches the current rate, compares against a threshold, and sends a notification if exceeded. All in one call — no parsing needed.
  Params: `threshold` (required), `from_currency` (default USD), `to_currency` (default INR), `message_id` (default rate_alert).

Example mission goal for rate monitoring:
> "On each run, use the `abound_rate_alert` tool through structured tool_calls with threshold 90, then report the returned message."

**CRITICAL: For mission threads that monitor rates, always use `abound_rate_alert` — never chain `abound_exchange_rate` + `abound_create_notification` manually.** The single tool is deterministic and avoids parsing errors.

## Rules

- **Never use `analyze_transfer` before `abound_send_wire` for a wire flow** — the analysis is built into `abound_send_wire` and runs automatically. Calling both wastes a step and breaks the flow.
- `analyze_transfer` and `validate_transfer_target` are USD/INR only. Don't use them for other pairs.
- `forex_historical_data` works for any Massive-supported pair.
- Always uppercase currency codes (USD, INR, not usd, inr).
- Never expose raw API details, URLs, or internal field names to the user.
