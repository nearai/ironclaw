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

## Default council line-up

Unless the user requests something specific, use this 4-model council:

```python
COUNCIL = [
    "anthropic/claude-opus-4-6",
    "google/gemini-3-pro",
    "zai-org/GLM-latest",
    "openai/gpt-5.4",
]
```

These four span the major frontier providers and reasoning styles. If the
user names specific models, use those instead.

## API

### Single call with a model override
```python
answer = llm_query(
    prompt="What is X?",
    context="Optional background",     # optional
    model="anthropic/claude-opus-4-6", # optional per-call override
)
```

### Parallel council (same prompt, many models)
```python
COUNCIL = [
    "anthropic/claude-opus-4-6",
    "google/gemini-3-pro",
    "zai-org/GLM-latest",
    "openai/gpt-5.4",
]
responses = llm_query_batched(
    prompts=["What are the main risks of X?"] * len(COUNCIL),
    models=COUNCIL,                    # parallel array, length must match prompts
    context="Answer in 3-5 bullet points.",
)
# `responses` is a list of strings in the same order as `models`.
# If a specific model is unavailable, that slot returns "Error: ..." —
# the rest of the batch still completes.
```

### Single model applied to many prompts
```python
results = llm_query_batched(
    prompts=["Q1", "Q2", "Q3"],
    model="anthropic/claude-opus-4-6", # singular: applies to every prompt
)
```

### Mixing `models=` slots with `None`
A `None` slot inside `models=[...]` means "no override for this prompt" —
that call uses the configured default model. The singular `model=` kwarg
does NOT backfill `None` slots; it is only used when `models=` is omitted
entirely.

## After collecting responses

1. **Identify consensus** — note where models agree
2. **Flag disagreements** — analyze where they diverge and why
3. **Synthesize** — produce a unified answer that accounts for all perspectives
4. **Cite** — reference which model contributed each insight

## Full example

```repl
COUNCIL = [
    "anthropic/claude-opus-4-6",
    "google/gemini-3-pro",
    "zai-org/GLM-latest",
    "openai/gpt-5.4",
]
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
- Errors from individual models (unavailable model, provider timeout, etc.) surface as `"Error: ..."` strings in the result list, so a single bad model does not fail the whole batch. Argument validation errors — wrong types, or `models`/`prompts` length mismatch — still raise exceptions.
