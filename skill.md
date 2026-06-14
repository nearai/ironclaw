---
name: news-summarizer
version: 0.1.0
description: Fetches today's top news headlines and summarizes them into clear 5-story summaries with context and key takeaways.
language: English
activation_keywords:
  - news
  - headlines
  - today's news
  - what's happening
  - current events
  - briefing
api_required:
  - NewsAPI (free tier at newsapi.org)
  - Anthropic Claude API
output_format: 5 story summaries with context and key takeaways
---

# News Summarizer Skill

A skill for IronClaw AI that fetches today's top headlines and generates clean, structured 5-story summaries with context and key takeaways — powered by NewsAPI and Claude.

## What It Does

- Fetches the latest top headlines from NewsAPI
- Sends them to Claude for intelligent summarization
- Returns a structured 5-story digest in a consistent, readable format

## Activation Patterns

Trigger this skill with phrases like:

- `What's the news today?`
- `Give me today's headlines`
- `What's happening in the world?`
- `Daily news briefing`
- `Summarize today's news`
- `Current events`

## Output Format

```
📰 TODAY'S TOP NEWS

1. HEADLINE TITLE
Summary: 2-sentence summary here.
Why it matters: Brief explanation of significance.

2. HEADLINE TITLE
Summary: 2-sentence summary here.
Why it matters: Brief explanation of significance.

3. HEADLINE TITLE
Summary: 2-sentence summary here.
Why it matters: Brief explanation of significance.

4. HEADLINE TITLE
Summary: 2-sentence summary here.
Why it matters: Brief explanation of significance.

5. HEADLINE TITLE
Summary: 2-sentence summary here.
Why it matters: Brief explanation of significance.

Want more details on any story?
```

## Setup

### 1. Get a NewsAPI Key

Sign up for free at [newsapi.org](https://newsapi.org) and grab your API key.

### 2. Set Environment Variables

```bash
export NEWS_API_KEY=your_newsapi_key_here
export ANTHROPIC_API_KEY=your_anthropic_key_here
```

### 3. Install Dependencies

```bash
pip install -r requirements.txt
```

### 4. Run

```bash
python skill.py
```

## File Structure

```
news-summarizer/
├── SKILL.md          # This file — skill spec and documentation
├── skill.py          # Core skill implementation
├── requirements.txt  # Python dependencies
├── example_output.md # Sample output for testing/demo
└── README.md         # Quick-start guide
```

## Configuration

| Field         | Value                              |
|---------------|------------------------------------|
| Name          | news-summarizer                    |
| Version       | 0.1.0                              |
| Language      | English                            |
| API Required  | NewsAPI (free), Anthropic Claude   |
| Activation    | Keywords: news, headlines, today   |
| Output        | 5 story summaries with context     |

## Notes

- NewsAPI free tier allows up to 100 requests/day
- Skill selects the top 10 headlines, then Claude picks and summarizes the 5 most significant
- Designed to be modular — swap NewsAPI for any headlines source by editing `fetch_headlines()`
-
