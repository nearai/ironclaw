---
name: llm-council
version: "1.0.0"
description: Query multiple LLM models in parallel from CodeAct and cross-reference their responses
activation:
  keywords:
    - "council"
    - "compare models"
    - "multiple models"
    - "cross-reference"
    - "second opinion"
    - "opinions"
    - "consensus"
    - "different models"
    - "model comparison"
    - "diverse perspectives"
    - "ask several"
    - "ask multiple"
    - "vote"
  exclude_keywords:
    - "routine"
    - "schedule"
  patterns:
    - "(?i)(ask|query|consult|compare)\\s.*(models|llms|ais)"
    - "(?i)(what do|how do)\\s.*(different|other|multiple)\\s.*(models|llms|ais)\\s*(think|say)"
    - "(?i)council"
    - "(?i)cross[- ]?referenc"
  tags:
    - "llm"
    - "analysis"
    - "research"
  max_context_tokens: 1200
---

# LLM Council

You can query multiple LLM models with the same prompt directly from CodeAct
using the built-in `llm_query()` and `llm_query_batched()` functions. Both
accept a `model=` (or `models=`) keyword that overrides the configured model
for that call. Providers that support per-request model overrides (NEAR AI,
Anthropic OAuth, GitHub Copilot, Bedrock) honor it; others fall back to their
configured model.

## When to use a council

- The user wants diverse perspectives on a question or analysis
- Cross-referencing answers to increase confidence
- Comparing reasoning approaches across models
- Getting a "second opinion" from different AI models
- Research or evaluation tasks that benefit from multiple viewpoints

## API

### Single call with a model override
```python
answer = llm_query(
    prompt="What is X?",
    context="Optional background",     # optional
    model="claude-sonnet-4-20250514",  # optional per-call override
)
```

### Parallel council (same prompt, many models)
```python
COUNCIL = ["gpt-4o", "claude-sonnet-4-20250514", "llama-3.1-70b-instruct"]
responses = llm_query_batched(
    prompts=["What are the main risks of X?"] * len(COUNCIL),
    models=COUNCIL,                    # parallel array, length must match prompts
    context="Answer in 3-5 bullet points.",
)
# `responses` is a list of strings in the same order as `models`.
```

### Single model applied to many prompts
```python
results = llm_query_batched(
    prompts=["Q1", "Q2", "Q3"],
    model="gpt-4o",                    # singular: applies to every prompt
)
```

## Recommended council line-ups (NEAR AI backend)

- **Quick (3 models):** `["gpt-4o", "claude-sonnet-4-20250514", "llama-3.1-70b-instruct"]`
- **Deep (5 models):** add `"qwen-2.5-72b-instruct"`, `"deepseek-chat"`
- **Reasoning-focused:** `["o3-mini", "claude-sonnet-4-20250514", "deepseek-reasoner"]`

## After collecting responses

1. **Identify consensus** — note where models agree
2. **Flag disagreements** — analyze where they diverge and why
3. **Synthesize** — produce a unified answer that accounts for all perspectives
4. **Cite** — reference which model contributed each insight

## Full example

```repl
COUNCIL = ["gpt-4o", "claude-sonnet-4-20250514", "llama-3.1-70b-instruct"]
question = "What are the main risks of relying on a single LLM provider?"

responses = llm_query_batched(
    prompts=[question] * len(COUNCIL),
    models=COUNCIL,
    context="Answer concisely in 3-5 bullet points.",
)

# Build a synthesis prompt
labelled = "\n\n---\n\n".join(
    f"**{model}**:\n{resp}" for model, resp in zip(COUNCIL, responses)
)

synthesis = llm_query(
    prompt=f"Synthesize a balanced answer from these {len(COUNCIL)} expert opinions:\n\n{labelled}",
    context="Identify consensus, flag disagreements, and produce a unified answer.",
)

FINAL(synthesis)
```

## Notes

- `llm_query_batched()` runs all calls in parallel — total latency is roughly the slowest model.
- If `models=` is provided, it must be the same length as `prompts`.
- Use `model=` (singular) when you want one model applied to every prompt; use `models=` (plural list) for the council pattern.
- Errors from individual models surface as `"Error: ..."` strings in the result list — the batch never raises.
