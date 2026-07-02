-----

## name: crypto-market-intel
description: >
Use this skill whenever a user wants to monitor cryptocurrency prices, detect unusual
trading volume, track whale wallet movements, or get AI-summarized crypto market news.
Trigger on phrases like: “watch this token”, “alert me if”, “any whale activity”,
“what’s happening with [token]”, “volume spike”, “market summary”, “is [coin] moving”,
“set a price alert”, “monitor [symbol]”, “crypto news”, “market intelligence”.
Also trigger for any request combining price + news + on-chain signals, even if the
user doesn’t say “crypto” explicitly (e.g., “what’s BTC doing today?”).

# Crypto Market Intelligence Skill

A skill for monitoring tokens, detecting anomalies, tracking whale movements, and
summarizing market news — using free public APIs, no API key required for most endpoints.

-----

## APIs at a Glance

|Data Need                  |API                  |Base URL                          |Auth             |
|---------------------------|---------------------|----------------------------------|-----------------|
|Price + volume + market cap|CoinGecko            |`https://api.coingecko.com/api/v3`|None (free tier) |
|Historical OHLCV           |CoinGecko            |same                              |None             |
|On-chain whale txns        |Whale Alert (limited)|`https://api.whale-alert.io/v1`   |Free key required|
|DeFi TVL + protocol data   |DeFiLlama            |`https://api.llama.fi`            |None             |
|Crypto news                |CryptoPanic          |`https://cryptopanic.com/api/v1`  |Free key required|
|Fear & Greed Index         |Alternative.me       |`https://api.alternative.me/fng/` |None             |


> For keys: Whale Alert → whalescan.io (free tier, 10 req/min). CryptoPanic → cryptopanic.com/developers/api/

-----

## Workflow

When the user asks for market intelligence on a token or set of tokens, follow this sequence:

### 1. Resolve Token IDs

CoinGecko uses slugs (`bitcoin`, `ethereum`, `solana`), not ticker symbols. Resolve first:

```
GET https://api.coingecko.com/api/v3/search?query={SYMBOL}
```

Extract `coins[0].id` from the response. Cache it for subsequent calls.

### 2. Fetch Price + Volume Data

```
GET https://api.coingecko.com/api/v3/coins/markets
  ?vs_currency=usd
  &ids={comma-separated-ids}
  &order=market_cap_desc
  &sparkline=false
  &price_change_percentage=1h,24h,7d
```

Key fields to extract per coin:

- `current_price`
- `price_change_percentage_24h`
- `total_volume` — 24h volume in USD
- `market_cap`
- `high_24h`, `low_24h`

### 3. Detect Volume Anomalies

Fetch 14-day historical volume to establish a baseline:

```
GET https://api.coingecko.com/api/v3/coins/{id}/market_chart
  ?vs_currency=usd
  &days=14
  &interval=daily
```

Response includes `total_volumes: [[timestamp, volume], ...]`.

**Anomaly thresholds:**

|Signal              |Threshold                    |Severity              |
|--------------------|-----------------------------|----------------------|
|Volume spike        |Current > 2× 14-day avg      |🟡 Elevated            |
|Volume spike        |Current > 3× 14-day avg      |🔴 High Alert          |
|Price + volume combo|>5% price move AND >2× volume|🔴 Breakout / Breakdown|
|Low volume          |Current < 0.4× avg           |⚪ Suppressed          |

Compute: `avg_volume = mean(historical_volumes)`, then `ratio = current_volume / avg_volume`.

### 4. Check Whale Movements (if key available)

```
GET https://api.whale-alert.io/v1/transactions
  ?api_key={KEY}
  &min_value=500000
  &limit=20
  &cursor=0
```

Filter results by the token’s symbol. Flag:

- Transactions > $1M USD as **Whale Move**
- Exchange → Exchange as potential **arbitrage**
- Unknown wallet → Exchange as potential **sell pressure**
- Exchange → Unknown wallet as potential **accumulation**

If no Whale Alert key: skip this section and note it in output.

### 5. Fear & Greed Index

```
GET https://api.alternative.me/fng/?limit=2
```

Returns today’s and yesterday’s index (0–100). Interpret:

- 0–24: Extreme Fear
- 25–49: Fear
- 50–74: Greed
- 75–100: Extreme Greed

Include delta (today vs yesterday) as sentiment shift signal.

### 6. Fetch Market News (if CryptoPanic key available)

```
GET https://cryptopanic.com/api/v1/posts/
  ?auth_token={KEY}
  &currencies={SYMBOL1,SYMBOL2}
  &filter=hot
  &public=true
```

Pull top 5–8 headlines. Ask Claude (yourself) to summarize in 2–3 sentences, noting:

- Any regulatory news
- Major protocol updates or hacks
- Macro market drivers

If no key: fall back to DeFiLlama news endpoint:

```
GET https://defillama.com/news (web scrape as fallback, or skip)
```

### 7. DeFi-Specific Data (for DeFi tokens)

If the token is a DeFi protocol token, also fetch TVL trend:

```
GET https://api.llama.fi/protocol/{protocol-slug}
```

Flag if TVL dropped >10% in 7 days alongside price drop — this is a **bearish divergence** signal.

-----

## Output Format

Always structure the output as a **Market Intelligence Report**. Use this template:

```
## 🪙 [TOKEN NAME] ([SYMBOL]) — Market Intelligence Report
📅 [Date/Time UTC]

### 💰 Price Summary
- Current Price: $X,XXX.XX
- 24h Change: ▲/▼ X.X%
- 24h Range: $LOW — $HIGH
- Market Cap: $X.XXB

### 📊 Volume Analysis
- 24h Volume: $X.XXB
- 14-day Avg Volume: $X.XXB
- Volume Ratio: X.Xx  [🔴/🟡/✅]
- Signal: [Breakout / Suppressed / Normal / Elevated]

### 🐋 Whale Activity  [skip if no key]
- [Transaction summary or "No significant whale moves in last 6h"]

### 😨 Market Sentiment
- Fear & Greed: XX/100 — [Label]
- Trend: [Up/Down] X points from yesterday

### 📰 News Summary  [skip if no key]
> [2–3 sentence summary of key headlines]

### ⚠️ Alerts
- [List any triggered alert conditions, or "No alerts — market conditions nominal"]
```

For multiple tokens, repeat the block for each, then add a **Portfolio Summary** section at the end.

-----

## Price Alert Logic

When the user says “alert me if [token] hits $X” or “alert if volume spikes”:

1. Record the condition: `{token, metric, operator, threshold}`
1. After fetching current data, evaluate the condition
1. If triggered: surface it prominently with `🚨 ALERT TRIGGERED` at the top of output
1. If not triggered: confirm current value vs threshold (“BTC at $67,200 — watching for $70,000 ▲”)

Supported alert types:

- `price_above`, `price_below`
- `volume_ratio_above` (e.g., >2× baseline)
- `price_change_pct_above` (e.g., >5% in 24h)
- `whale_transaction_above` (e.g., >$1M)

-----

## Rate Limiting

CoinGecko free tier: **~10–30 req/min**. If monitoring multiple tokens:

- Batch coins into one `markets` call (supports up to 250 IDs)
- Use a single `market_chart` call per coin (count carefully)
- Add 1–2s delay between calls if hitting limits (HTTP 429 = back off)

-----

## Error Handling

|Error                 |Action                                            |
|----------------------|--------------------------------------------------|
|CoinGecko 429         |Wait 60s, retry once; inform user if still failing|
|Token not found       |Ask user to confirm symbol; try alternate spelling|
|No Whale Alert key    |Skip section, note in output                      |
|No CryptoPanic key    |Skip news section, note in output                 |
|API returns empty data|State “data unavailable” — never fabricate prices |


> ⚠️ **Never hallucinate prices or volume figures.** If an API call fails, say so explicitly.

-----

## Quick Reference: Common Token IDs (CoinGecko)

|Symbol|CoinGecko ID |
|------|-------------|
|BTC   |bitcoin      |
|ETH   |ethereum     |
|SOL   |solana       |
|BNB   |binancecoin  |
|XRP   |ripple       |
|ADA   |cardano      |
|AVAX  |avalanche-2  |
|DOGE  |dogecoin     |
|MATIC |matic-network|
|LINK  |chainlink    |
|UNI   |uniswap      |
|AAVE  |aave         |

For others, always resolve via the search endpoint.

-----

## Example User Requests & How to Handle

**“What’s ETH doing right now?”**
→ Fetch price + volume for ethereum, run anomaly check, pull F&G index, output report.

**“Alert me if BTC goes above $75,000”**
→ Fetch BTC price, compare to $75K threshold, output alert status.

**“Are there any whale moves on SOL today?”**
→ Fetch Whale Alert transactions filtered to SOL, summarize. If no key, say so.

**“Give me a market summary for BTC, ETH, SOL”**
→ Batch fetch all three, run anomaly checks, include news summary, end with portfolio view.

**“Is this a good time to buy LINK?”**
→ Run full intelligence report, include sentiment + volume signal, then clearly note: *“This is market data only — not financial advice.”*