---
name: forex-timing
version: "0.1.0"
description: USD/INR forex transfer timing — volatility regime, RSI, DXY momentum → hit rate, target rate, and probability cone
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
  max_context_tokens: 3500
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

Three actions for USD/INR forex transfer analysis. All use the Massive API for OHLCV data (Bearer token auto-injected — never add an Authorization header manually) and Yahoo Finance v8 chart API for free DXY direction data (ticker: `DX-Y.NYB`).

**`analyze_transfer`** and **`validate_transfer_target`** are calibrated to USD/INR specifically.  
**`get_forex_historical_data`** works for any currency pair supported by Massive.

---

## Helper Functions

Copy all helpers verbatim into your `repl` block before calling any action. They are pure functions — no tool calls inside them.

**IMPORTANT**: Always run actions inside a single ` ```repl ` block. Do NOT call `http` as a direct tool — write Python code that calls `await http(...)` inside the repl block, computes everything, and ends with `FINAL(result)`.

```python
import math
import asyncio

# ── Date arithmetic ────────────────────────────────────────────────────────────
# Uses unix day numbers (days since 1970-01-01) throughout.

def unix_day_from_ymd(y, m, d):
    y2 = y - 1 if m <= 2 else y
    m2 = m + 12 if m <= 2 else m
    A = y2 // 100
    B = 2 - A + A // 4
    jd = int(365.25 * (y2 + 4716)) + int(30.6001 * (m2 + 1)) + d + B - 1524
    return jd - 2440588

def format_unix_day(n):
    z = n + 719468
    era = z // 146097
    doe = z - era * 146097
    yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    y = yoe + era * 400
    doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
    mp = (5 * doy + 2) // 153
    d = doy - (153 * mp + 2) // 5 + 1
    m = mp + (3 if mp < 10 else -9)
    y = y + (1 if m <= 2 else 0)
    return f"{y:04d}-{m:02d}-{d:02d}"

def today_unix_day(iso_date_str):
    # iso_date_str: "YYYY-MM-DD" from `await time(operation="now", timezone="UTC")["iso8601"][:10]`
    y, m, d = int(iso_date_str[:4]), int(iso_date_str[5:7]), int(iso_date_str[8:10])
    return unix_day_from_ymd(y, m, d)

def ms_to_date_str(ms):
    return format_unix_day(ms // 86400000)

# ── Massive API response parsing ───────────────────────────────────────────────

def parse_massive_bars(resp):
    """Returns (bars, error_str). bars is a list of dicts with date/open/high/low/close/volume."""
    if resp["status"] != 200:
        return None, "Massive API error: HTTP " + str(resp["status"])
    data = resp["body"]
    if not isinstance(data, dict):
        return None, "Massive API: unexpected response format"
    results = data.get("results")
    if not results:
        return None, "Massive API: no results returned"
    bars = []
    for r in results:
        bars.append({
            "date": ms_to_date_str(r["t"]),
            "open": r["o"],
            "high": r["h"],
            "low": r["l"],
            "close": r["c"],
            "volume": r["v"],
        })
    return bars, None

# ── Yahoo Finance DXY JSON parsing ────────────────────────────────────────────

def parse_dxy_direction(resp):
    """Returns 'up', 'down', or 'unknown'. Never raises.
    Expects a Yahoo Finance v8 chart JSON response."""
    try:
        if resp["status"] != 200:
            return "unknown"
        body = resp["body"]
        if not isinstance(body, dict):
            return "unknown"
        result = body.get("chart", {}).get("result")
        if not result:
            return "unknown"
        raw_closes = result[0]["indicators"]["quote"][0]["close"]
        closes = [c for c in raw_closes if c is not None]
        DXY_WINDOW = 5
        if len(closes) < DXY_WINDOW + 1:
            return "unknown"
        change = closes[-1] / closes[-(DXY_WINDOW + 1)] - 1.0
        return "up" if change >= 0.0 else "down"
    except Exception:
        return "unknown"

# ── Volatility ─────────────────────────────────────────────────────────────────

def log_returns(closes):
    rets = []
    for i in range(1, len(closes)):
        rets.append(math.log(closes[i] / closes[i - 1]))
    return rets

def sample_std(values):
    n = len(values)
    if n < 2:
        return 0.0
    mean = sum(values) / n
    variance = sum([(v - mean) ** 2 for v in values]) / (n - 1)
    return math.sqrt(variance)

def vol_bucket(vol):
    if vol < 0.00131:
        return "very_low"
    if vol > 0.00374:
        return "very_high"
    return "normal"

# ── RSI (Wilder smoothing) ─────────────────────────────────────────────────────

def rsi(closes, period=14):
    if len(closes) < period + 1:
        return None
    gains = []
    losses = []
    for i in range(1, period + 1):
        d = closes[i] - closes[i - 1]
        gains.append(max(d, 0.0))
        losses.append(max(-d, 0.0))
    avg_g = sum(gains) / period
    avg_l = sum(losses) / period
    for i in range(period + 1, len(closes)):
        d = closes[i] - closes[i - 1]
        avg_g = (avg_g * (period - 1) + max(d, 0.0)) / period
        avg_l = (avg_l * (period - 1) + max(-d, 0.0)) / period
    if avg_l == 0.0:
        return 100.0
    return 100.0 - 100.0 / (1.0 + avg_g / avg_l)

def rsi_bucket(r):
    if r < 50:
        return "low"
    if r <= 70:
        return "mid"
    return "high"

# ── Hit rate cube (vol × rsi × dxy → pct) ─────────────────────────────────────
# Lookup table calibrated to USD/INR historical data.

def hit_rate(vol, rsi_b, dxy):
    if vol == "very_low":
        if rsi_b == "low":
            if dxy == "up":   return 43.7
            if dxy == "down": return 32.2
            return (43.7 + 32.2) / 2.0
        if rsi_b == "mid":
            if dxy == "up":   return 45.3
            if dxy == "down": return 34.9
            return (45.3 + 34.9) / 2.0
        if rsi_b == "high":
            if dxy == "up":   return 53.4
            if dxy == "down": return 44.7
            return (53.4 + 44.7) / 2.0
    if vol == "normal":
        if rsi_b == "low":
            if dxy == "up":   return 35.0
            if dxy == "down": return 33.5
            return (35.0 + 33.5) / 2.0
        if rsi_b == "mid":
            if dxy == "up":   return 44.7
            if dxy == "down": return 36.8
            return (44.7 + 36.8) / 2.0
        if rsi_b == "high":
            if dxy == "up":   return 52.9
            if dxy == "down": return 47.0
            return (52.9 + 47.0) / 2.0
    if vol == "very_high":
        if rsi_b == "low":
            if dxy == "up":   return 37.9
            if dxy == "down": return 36.5
            return (37.9 + 36.5) / 2.0
        if rsi_b == "mid":
            if dxy == "up":   return 40.0
            if dxy == "down": return 32.5
            return (40.0 + 32.5) / 2.0
        if rsi_b == "high":
            if dxy == "up":   return 53.1
            if dxy == "down": return 41.2
            return (53.1 + 41.2) / 2.0
    return 40.0

# ── Projection cone ────────────────────────────────────────────────────────────

def regime_k(vb):
    if vb == "very_low":  return 1.0
    if vb == "very_high": return 0.5
    return 0.75

def compute_cone(current_rate, daily_vol, vb, today_day):
    CONE_Z = 1.645
    HORIZON_DAYS = 3
    k = regime_k(vb)
    target_rate = current_rate * math.exp(k * daily_vol)
    projection = []
    for t in range(HORIZON_DAYS + 1):
        date_str = format_unix_day(today_day + t)
        center = current_rate + (target_rate - current_rate) * t / HORIZON_DAYS
        if t == 0:
            upper = current_rate
            lower = current_rate
        else:
            upper = current_rate * math.exp(CONE_Z * daily_vol * math.sqrt(t))
            lower = current_rate * math.exp(-CONE_Z * daily_vol * math.sqrt(t))
        projection.append({"date": date_str, "center": center, "upper": upper, "lower": lower})
    return target_rate, projection

# ── Return percentile table (USD/INR calibrated) ───────────────────────────────

RETURN_PERCENTILES = [
    [3,   [[0.01,-0.016899],[0.05,-0.007893],[0.10,-0.005139],[0.25,-0.001845],
           [0.50, 0.000113],[0.75, 0.002414],[0.90, 0.005981],[0.95, 0.009035],
           [0.99, 0.019432]]],
    [7,   [[0.01,-0.023974],[0.05,-0.012192],[0.10,-0.007996],[0.25,-0.002875],
           [0.50, 0.000299],[0.75, 0.004106],[0.90, 0.009496],[0.95, 0.014416],
           [0.99, 0.028761]]],
    [30,  [[0.01,-0.053051],[0.05,-0.026386],[0.10,-0.017315],[0.25,-0.006388],
           [0.50, 0.001127],[0.75, 0.011128],[0.90, 0.023445],[0.95, 0.036908],
           [0.99, 0.069213]]],
    [90,  [[0.01,-0.072050],[0.05,-0.041678],[0.10,-0.029833],[0.25,-0.011556],
           [0.50, 0.005127],[0.75, 0.022799],[0.90, 0.044823],[0.95, 0.070926],
           [0.99, 0.140583]]],
    [180, [[0.01,-0.109389],[0.05,-0.056501],[0.10,-0.041380],[0.25,-0.013188],
           [0.50, 0.012416],[0.75, 0.038021],[0.90, 0.073889],[0.95, 0.114706],
           [0.99, 0.180774]]],
    [365, [[0.01,-0.150317],[0.05,-0.077316],[0.10,-0.054906],[0.25,-0.017094],
           [0.50, 0.030325],[0.75, 0.073258],[0.90, 0.111315],[0.95, 0.169161],
           [0.99, 0.225594]]],
]

def hit_rate_from_percentiles(required_move, table):
    """Bisect-interpolate: what % of historical moves exceeded required_move?"""
    idx = len(table)
    for i in range(len(table)):
        if table[i][1] >= required_move:
            idx = i
            break
    if idx >= len(table):
        return 0.0
    if idx == 0:
        return 100.0
    lo_pct = table[idx - 1][0]
    lo_ret = table[idx - 1][1]
    hi_pct = table[idx][0]
    hi_ret = table[idx][1]
    frac = (required_move - lo_ret) / (hi_ret - lo_ret) if hi_ret != lo_ret else 0.0
    at_pct = lo_pct + frac * (hi_pct - lo_pct)
    return round((1.0 - at_pct) * 1000.0) / 10.0
```

---

## Action: `get_forex_historical_data`

Fetch OHLCV bars for any currency pair from the Massive API.

**Parameters**: `from_currency` (required), `to_currency` (required), `start_date` YYYY-MM-DD (required), `end_date` YYYY-MM-DD (optional, defaults to today), `timespan` (optional, default `day`), `multiplier` (optional, default `1`).

```python
# After helpers above:
from_ccy = "USD"
to_ccy   = "INR"
start    = "2026-01-01"
# end defaults to today:
_now = await time(operation="now", timezone="UTC")
today_day = today_unix_day(_now["iso"][:10])
end = format_unix_day(today_day)

pair = (from_ccy + to_ccy).upper()
url  = f"https://api.massive.com/v2/aggs/ticker/C:{pair}/range/1/day/{start}/{end}?sort=asc&limit=5000"
resp = await http(method="GET", url=url)
bars, err = parse_massive_bars(resp)
if err:
    FINAL({"error": err})

FINAL(bars)
```

**Return shape**: list of `{"date", "open", "high", "low", "close", "volume"}`.

---

## Action: `analyze_transfer`

Recommend whether to transfer USD→INR now or wait, based on volatility regime, RSI(14), and DXY direction.  
**USD/INR calibrated only.**

**Parameters**: `from_currency` (e.g. `"USD"`), `to_currency` (e.g. `"INR"`), `amount` (optional, number of units of `from_currency` the user intends to send — used to compute `could_save`).

```python
# After helpers above:
_now = await time(operation="now", timezone="UTC")
today_day  = today_unix_day(_now["iso"][:10])
start_day  = today_day - 220
start_str  = format_unix_day(start_day)
end_str    = format_unix_day(today_day)

pair        = "USDINR"
massive_url = f"https://api.massive.com/v2/aggs/ticker/C:{pair}/range/1/day/{start_str}/{end_str}?sort=asc&limit=5000"
now_unix    = _now["unix"]
dxy_url     = f"https://query1.finance.yahoo.com/v8/finance/chart/DX-Y.NYB?interval=1d&period1={now_unix - 35*86400}&period2={now_unix}"

massive_resp, dxy_resp = await asyncio.gather(
    http(method="GET", url=massive_url),
    http(method="GET", url=dxy_url, headers={"User-Agent": "Mozilla/5.0"}),
)

bars, err = parse_massive_bars(massive_resp)
if err:
    FINAL({"error": err})
if len(bars) < 23:
    FINAL({"error": f"Insufficient data: need 23 bars, got {len(bars)}"})

closes = [b["close"] for b in bars]

VOL_WINDOW = 20
daily_vol  = sample_std(log_returns(closes[-VOL_WINDOW:]))
vb         = vol_bucket(daily_vol)

rsi_val = rsi(closes)
rsi_val = round(rsi_val, 1) if rsi_val is not None else None
rb      = rsi_bucket(rsi_val) if rsi_val is not None else "mid"

dxy_dir = parse_dxy_direction(dxy_resp)
hr      = hit_rate(vb, rb, dxy_dir if dxy_dir != "unknown" else None)

current_rate = closes[-1]
target_rate, projection = compute_cone(current_rate, daily_vol, vb, today_day)
recommend = "now" if hr < 45.0 else "wait"

historical = [{"date": b["date"], "close": b["close"]} for b in bars[-30:]]

amount = amount if isinstance(amount, (int, float)) else None
could_save = round((target_rate - current_rate) * amount, 2) if amount is not None else None

action_verb = "Transfer now" if recommend == "now" else "Wait — hold off"
FINAL({
    "message": (
        f"{action_verb}. USD/INR is at {current_rate:.4f}; target {round(target_rate, 4):.4f}. "
        f"Regime: {vb} volatility, RSI {rsi_val} ({rb}), DXY {dxy_dir}. "
        f"Hit-rate for this regime: {round(hr, 1)}%."
        + (f" If you wait, you could get ₹{could_save:,.2f} more on your transfer." if recommend == "wait" and could_save and could_save > 0 else "")
    ),
    "plot": {
        "historical":    historical,
        "projection":    projection,
        "current_rate":  current_rate,
        "target_rate":   round(target_rate, 4),
        "vol_regime":    vb,
        "daily_vol":     round(daily_vol, 6),
        "rsi":           rsi_val,
        "dxy_direction": dxy_dir,
        "hit_rate_pct":  round(hr, 1),
        "recommend":     recommend,
        "could_save":    could_save,
    },
})
```

---

## Action: `validate_transfer_target`

Given a user's desired USD/INR rate, compute the probability of hitting it across 6 horizons.  
**USD/INR calibrated only.**

**Parameters**: `target_rate` (number, required).

```python
# After helpers above:
target_rate_input = 86.0   # replace with user's value

_now = await time(operation="now", timezone="UTC")
today_day = today_unix_day(_now["iso"][:10])
start_str = format_unix_day(today_day - 5)
end_str   = format_unix_day(today_day)

url  = f"https://api.massive.com/v2/aggs/ticker/C:USDINR/range/1/day/{start_str}/{end_str}?sort=asc&limit=10"
resp = await http(method="GET", url=url)
bars, err = parse_massive_bars(resp)
if err:
    FINAL({"error": err})

current_rate  = bars[-1]["close"]
required_move = math.log(target_rate_input / current_rate)

horizons = []
for entry in RETURN_PERCENTILES:
    h_days = entry[0]
    table  = entry[1]
    hr     = hit_rate_from_percentiles(required_move, table)
    horizons.append({"horizon_days": h_days, "hit_rate_pct": hr})

rec_horizon = None
for h in horizons:
    if h["hit_rate_pct"] >= 20.0:
        rec_horizon = h["horizon_days"]
        break

horizon_note = f"earliest horizon with ≥20% probability: {rec_horizon}d" if rec_horizon else "no horizon reaches 20% probability"
FINAL({
    "message": (
        f"USD/INR is at {current_rate:.4f}. Hitting {target_rate_input:.4f} needs a "
        f"{round(required_move * 100, 4)}% move ({horizon_note})."
    ),
    "plot": {
        "current_rate":             current_rate,
        "target_rate":              round(target_rate_input, 4),
        "required_move_pct":        round(required_move * 100, 4),
        "horizons":                 horizons,
        "recommended_horizon_days": rec_horizon,
    },
})
```

---

## Rules

- **Two-field response contract**: every `FINAL()` call for `analyze_transfer` and `validate_transfer_target` MUST return exactly `{"message": "...", "plot": {...}}`. `message` is the plain-English summary shown to the user. `plot` holds all numeric/chart data for the frontend — do NOT include chart fields at the top level.
- **Execute everything in a single `repl` block**: copy the helpers, then the action code, into one fenced ` ```repl ` block and run it. Never call `http` as a standalone tool call outside of a repl block — the raw response bodies will pollute the context.
- **Never set an `Authorization` header** — `api.massive.com` credentials are injected automatically.
- Yahoo Finance DXY failure is always non-fatal: `parse_dxy_direction` returns `"unknown"` on any error.
- `analyze_transfer` and `validate_transfer_target` are USD/INR only; their static tables are not valid for other pairs.
- `get_forex_historical_data` is generic (any Massive-supported pair).
- Always uppercase currency codes before building Massive URLs (`C:USDINR`, not `C:usdinr`).
- Massive uses `?limit=5000` — covers 220 calendar days (~160 trading days) in a single request.
