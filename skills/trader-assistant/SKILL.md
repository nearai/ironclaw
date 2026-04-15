---
name: trader-setup
version: 0.3.0
description: One-time onboarding for the financial trader workflow — real-time alerts, position-aware relevance, decision journaling with outcome tracking. After successful setup this skill is excluded from selection until the marker file is deleted.
activation:
  setup_marker: projects/commitments/.trader-setup-complete
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
  max_context_tokens: 2500
requires:
  skills:
    - commitment-triage
    - commitment-digest
    - decision-capture
    - delegation-tracker
---

# Financial Trader — Commitment System Setup

You are configuring the commitments system for a financial trader. Their day involves:
- Pre-market: research, reviewing overnight moves, updating thesis
- Market hours: intense, real-time. Speed matters — seconds count for some signals
- Post-market: journaling, reviewing positions, reading research, planning next day
- Information velocity is extreme; contradictory signals are common

## Companion skills

This bundle relies on these skills activating during conversation (keyword-triggered):

| Skill | Activates when | What it does |
|---|---|---|
| `commitment-triage` | User mentions obligations, deadlines | Extracts signals, creates/resolves commitments |
| `commitment-digest` | User asks "show commitments" | Composes formatted summary |
| `decision-capture` | User makes a trade decision ("sold half my AAPL") | Records decision with rationale for journaling |
| `delegation-tracker` | User delegates research tasks | Tracks delegation follow-ups |

If any are missing from `skills/`, tell the user which ones are needed.

## Step 1: Ask configuration questions

1. **Markets**: Which markets/asset classes and market hours? (US equities, crypto, forex, options, futures)
2. **Position tracking**: Do you want me to track your current positions? If so, where do you log them? (I'll read from a workspace file you maintain)
3. **Alert threshold**: During market hours, should I alert immediately for position-relevant signals, or batch everything?
4. **Journal cadence**: Do you journal daily (post-market) or weekly?
5. **Risk signals**: Any specific tickers, sectors, or keywords that should always trigger immediate alerts?

## Step 2: Declare the project and create workspace structure

Writing any file under `projects/commitments/` is the declaration that
the project exists — the engine auto-registers it and scopes missions
to it. Start with:

```
memory_write(
  target: "projects/commitments/AGENTS.md",
  content: "# Commitments (Trader)\n\nThis project tracks trading signals, positions, and trade decisions.\n\n## Operating principles\n\n- Always read `positions.md` before scoring signal relevance — a headline is noise unless it matches a held position.\n- Position-relevant signals fire `immediacy=realtime` and broadcast immediately.\n- Market signals expire after 4h on trading days; research signals after 48h.\n- Contradictory signals on the same ticker flag as CONFLICT and surface together.\n- Trade decisions go in `projects/commitments/decisions/` with outcome tracking.\n",
  append: false
)
```

Then:

1. Check if `projects/commitments/README.md` exists via `memory_read`. If it does, skip to creating trader-specific files.
2. Write `projects/commitments/README.md` with the full schema — see `commitment-setup` skill for the complete content including immediacy levels, signal destinations, resolution paths, and trust calibration.
3. Create placeholder READMEs in each subdirectory: `open/`, `resolved/`, `signals/pending/`, `signals/expired/`, `decisions/`, `parked-ideas/`.
4. Create trader-specific files:

```
memory_write(target="projects/commitments/positions.md", content="# Current Positions\n\nMaintain your positions here. The agent reads this to score signal relevance.\n\n## Format\n\n- TICKER: SIZE, entry PRICE, thesis: BRIEF_THESIS\n\nExample:\n- AAPL: 500 shares, entry $175, thesis: AI integration undervalued\n- SPY Apr 520P: 10 contracts, thesis: hedging macro risk\n\n## Positions\n\n(Add your positions here)", append=false)
```

```
memory_write(target="projects/commitments/trade-journal/README.md", content="Daily trade journal entries. Each file: projects/commitments/decisions/<date>-<slug>.md with outcome tracking.", append=false)
```

## Step 3: Create tuned missions

### Triage mission — market-hours aware, position-sensitive

```
mission_create(
  name: "commitment-triage",
  goal: "Trader triage. Read projects/commitments/README.md for schema. Read projects/commitments/positions.md for current positions. Priority order: (1) Position-relevant signals — any signal mentioning a ticker in the positions list gets immediacy=realtime and urgency=critical. Broadcast immediately via message tool. (2) Contradictory signal detection — if two pending signals point in opposite directions on the same ticker or thesis, flag as CONFLICT and surface both together immediately. (3) Market signals expire after 4 hours during market days, 24 hours otherwise. Research/thesis signals expire after 48 hours. (4) Route market intelligence (analyst reports, macro data) to intelligence destination via MemoryDoc. (5) Check decisions older than 7 days without outcome — prompt for outcome assessment. (6) Append triage summary to projects/commitments/triage-log.md. (7) Alert on any position-relevant or conflicting signals.",
  cadence: "0 8,10,12,14,16,18 * * 1-5",
  project_id: "commitments"
)
```

Six runs on market days — every 2 hours from pre-market to post-market close.

### Digest mission — pre-market brief and post-market journal prompt

```
mission_create(
  name: "commitment-digest",
  goal: "Trader digest. Read projects/commitments/README.md for schema. Read projects/commitments/positions.md for current positions. If this is a morning run: (1) POSITION STATUS — list each position with any relevant signals from the last 24h. (2) OPEN RESEARCH — commitments tagged 'research' or 'thesis'. (3) PENDING DECISIONS — items with resolution_path=needs_decision. (4) CONFLICTING SIGNALS — any unresolved conflicts. If this is an evening run: (1) Summarize today's decisions from projects/commitments/decisions/ with today's date. (2) For each decision, note if outcome data is available. (3) Prompt: 'Any trades to journal? Any thesis updates?' End with 'Did I miss anything?' Send via message tool.",
  cadence: "0 7,18 * * 1-5",
  project_id: "commitments"
)
```

### Weekly review mission

```
mission_create(
  name: "trader-weekly-review",
  goal: "Weekly trading review. Read all files in projects/commitments/decisions/ from the past 7 days. For each decision: (1) What was decided and why. (2) If outcome data exists, was it positive or negative? (3) Which signals informed the decision — were those signal sources reliable? Also read projects/commitments/positions.md — for each position, has the original thesis changed based on this week's signals? Flag any position where contradictory evidence has accumulated. Write review summary to context/intel/weekly-review-<date>.md as durable intelligence. Send via message tool.",
  cadence: "0 10 * * 6",
  project_id: "commitments"
)
```

## Step 4: Write calibration memories

```
memory_write(
  target: "projects/commitments/calibration.md",
  content: "# Trader Calibration\n\n- Always read projects/commitments/positions.md before scoring signal relevance — a headline about AAPL is noise unless you hold AAPL\n- Position-relevant signals get immediacy=realtime — broadcast immediately, do not wait for digest\n- Market signals expire after 4 hours on trading days; research signals after 48 hours\n- When two signals contradict on the same ticker or thesis, flag as CONFLICT — never surface them independently\n- Trade decisions go in projects/commitments/decisions/ with the standard schema, plus outcome tracking\n- Prompt for outcome assessment on decisions older than 7 days: 'You decided X a week ago. How did it play out?'\n- Pre-market brief leads with position-relevant signals; post-market prompt leads with today's decisions\n- Weekly review on Saturday assesses signal source reliability and thesis drift — write to context/intel/ as durable intelligence\n- The user maintains positions.md manually — do not modify it, only read it\n- Start conservative: surface everything, ask before acting on agent_can_handle items",
  append: false
)
```

## Step 5: Confirm

> Your trading commitment system is ready:
> - **Triage** runs every 2 hours on market days (8am–6pm) — position-aware, contradictory signal detection, 4h market signal expiration, realtime alerts for position-relevant signals
> - **Pre-market brief** at 7am — position-relevant signals, open research, pending decisions
> - **Post-market journal** at 6pm — today's decisions, outcome prompts
> - **Weekly review** Saturday 10am — decision outcomes, signal reliability, thesis drift
> - Update `projects/commitments/positions.md` with your holdings for position-aware scoring
> - Say **"I sold half my AAPL because of the earnings miss"** to journal a trade decision
> - Say **"show commitments"** for current status, or **"any conflicts?"** for contradictory signals

## Step 6: Mark setup complete

After confirming with the user, write the setup completion marker so this skill stops competing for the activation budget on every subsequent message:

```
memory_write(
  path: "projects/commitments/.trader-setup-complete",
  content: "# Trader Setup Complete\n\nCompleted: <today's UTC date>\n\nMissions installed: trader-triage, trader-pre-market, trader-post-market, trader-weekly-review"
)
```

To re-trigger setup, delete `projects/commitments/.trader-setup-complete` first.
