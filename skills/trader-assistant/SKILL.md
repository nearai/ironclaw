---
name: trader-assistant
version: 0.1.0
description: Commitment tracking tuned for financial traders — real-time alerts, position-aware relevance, decision journaling with outcome tracking.
activation:
  keywords:
    - trader assistant
    - trading workflow
    - portfolio tracking
    - market alerts
    - trading setup
    - position tracking
    - trade journal
  patterns:
    - "(?i)I'm a (trader|investor|portfolio manager|fund manager)"
    - "(?i)set ?up.*(trading|portfolio|market|position)"
    - "(?i)help me (track|manage) my (trades|positions|portfolio)"
  tags:
    - commitments
    - trading
    - finance
    - setup
  max_context_tokens: 2000
---

# Financial Trader — Commitment System Setup

You are configuring the commitments system for a financial trader. Their day involves:
- Pre-market: research, reviewing overnight moves, updating thesis
- Market hours: intense, real-time. Speed matters — seconds count for some signals
- Post-market: journaling, reviewing positions, reading research, planning next day
- Information velocity is extreme; contradictory signals are common

## Step 1: Ask configuration questions

1. **Timezone and channel**: What timezone and market hours? Which channel for alerts and digests?
2. **Markets**: Which markets/asset classes? (US equities, crypto, forex, options, futures)
3. **Position tracking**: Do you want me to track your current positions? If so, where do you log them? (I'll read from a workspace file you maintain)
4. **Alert threshold**: During market hours, should I alert immediately for position-relevant signals, or batch everything?
5. **Journal cadence**: Do you journal daily (post-market) or weekly?
6. **Risk signals**: Any specific tickers, sectors, or keywords that should always trigger immediate alerts?

## Step 2: Create workspace structure

Create the full commitments workspace if it doesn't exist, plus trader-specific files:

```
memory_write(target="commitments/positions.md", content="# Current Positions\n\nMaintain your positions here. The agent reads this to score signal relevance.\n\n## Format\n\n- TICKER: SIZE, entry PRICE, thesis: BRIEF_THESIS\n\nExample:\n- AAPL: 500 shares, entry $175, thesis: AI integration undervalued\n- SPY Apr 520P: 10 contracts, thesis: hedging macro risk\n\n## Positions\n\n(Add your positions here)", append=false)
```

```
memory_write(target="commitments/trade-journal/README.md", content="Daily trade journal entries. Each file: decisions/<date>-<slug>.md with outcome tracking.", append=false)
```

## Step 3: Create tuned routines

### Triage routine — market-hours aware, position-sensitive

```
routine_create(
  name: "commitment-triage",
  description: "Trader triage — position-aware signal scoring, contradictory signal detection, fast expiration",
  prompt: "You are triaging commitments for a financial trader. Read commitments/README.md for schema. Read commitments/positions.md for current positions. Priority order: (1) Position-relevant signals — any signal mentioning a ticker in the positions list gets urgency=critical. (2) Contradictory signal detection — if two pending signals point in opposite directions on the same ticker or thesis, flag as CONFLICT and surface both together. (3) Market signals expire after 4 hours during market days, 24 hours otherwise. (4) Research/thesis signals expire after 48 hours. (5) Check for decisions without outcome tracking (in commitments/decisions/) older than 7 days — prompt for outcome assessment. (6) Append triage summary to commitments/triage-log.md. (7) If position-relevant or conflicting signals found, alert immediately.",
  request: { kind: "cron", schedule: "0 8,10,12,14,16,18 * * MON-FRI", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 7, context_paths: ["commitments/README.md", "commitments/positions.md"] }
)
```

Six runs on market days — every 2 hours from pre-market to post-market close.

### Digest routine — pre-market brief and post-market journal prompt

```
routine_create(
  name: "commitment-digest",
  description: "Trader digest — pre-market brief with position-relevant signals, post-market journal prompt",
  prompt: "Compose a trader digest. Read commitments/README.md for schema. Read commitments/positions.md for current positions. If this is a morning run: (1) POSITION STATUS — list each position with any relevant signals from the last 24h. (2) OPEN RESEARCH — commitments tagged 'research' or 'thesis'. (3) PENDING DECISIONS — items needing a trade decision. (4) CONFLICTING SIGNALS — any unresolved conflicts. If this is an evening run: (1) Summarize today's decisions from commitments/decisions/ with today's date. (2) For each decision, note if outcome data is available. (3) Prompt: 'Any trades to journal? Any thesis updates?' Send via message tool.",
  request: { kind: "cron", schedule: "0 7,18 * * MON-FRI", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md", "commitments/positions.md"] }
)
```

7am pre-market brief, 6pm post-market journal prompt.

### Weekly review routine

```
routine_create(
  name: "trader-weekly-review",
  description: "Weekly review — decision outcomes, signal source reliability, position thesis check",
  prompt: "Compose a weekly trading review. Read all files in commitments/decisions/ from the past 7 days. For each decision: (1) What was decided and why. (2) If outcome data exists, was it positive or negative? (3) Which signals informed the decision — were those signal sources reliable? Also check commitments/positions.md — for each position, has the original thesis changed based on this week's signals? Flag any position where contradictory evidence has accumulated. Send via message tool.",
  request: { kind: "cron", schedule: "0 10 * * SAT", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md", "commitments/positions.md"] }
)
```

## Step 4: Write calibration memories

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Trader Calibration\n\n- Always read commitments/positions.md before scoring signal relevance — a headline about AAPL is noise unless you hold AAPL\n- Market signals expire after 4 hours on trading days; research signals after 48 hours\n- When two signals contradict on the same ticker or thesis, flag as CONFLICT — never surface them independently\n- Trade decisions go in commitments/decisions/ with the standard schema, plus an 'outcome' section to be filled later\n- Prompt for outcome assessment on decisions older than 7 days: 'You decided X a week ago. How did it play out?'\n- Pre-market brief leads with position-relevant signals; post-market prompt leads with today's decisions\n- During market hours, position-relevant signals are always urgency=critical\n- Weekly review on Saturday morning assesses signal source reliability and thesis drift\n- The user maintains positions.md manually — do not modify it, only read it",
  append: false
)
```

## Step 5: Explain the positions file

Tell the user: "I've created `commitments/positions.md` — update it with your current positions so I can score signal relevance. Format: `- TICKER: SIZE, entry PRICE, thesis: BRIEF`. I'll read it during every triage run but never modify it."

## Step 6: Confirm

> Your trading commitment system is ready:
> - **Triage** runs every 2 hours on market days (8am–6pm) — position-aware, contradictory signal detection, 4h market signal expiration
> - **Pre-market brief** at 7am — position-relevant signals, open research, pending decisions
> - **Post-market journal** at 6pm — today's decisions, outcome prompts
> - **Weekly review** Saturday 10am — decision outcomes, signal reliability, thesis drift
> - Update `commitments/positions.md` with your holdings for position-aware scoring
> - Say **"I sold half my AAPL because of the earnings miss"** to journal a trade decision
> - Say **"show commitments"** for current status, or **"any conflicts?"** for contradictory signals
