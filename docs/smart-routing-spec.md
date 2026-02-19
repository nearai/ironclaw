# Smart Model Routing for IronClaw

**Status:** Proposed  
**Author:** Microwave  
**Date:** 2026-02-19  

## What

Automatic model selection based on request complexity. The router analyzes each user message and selects an appropriate model tier (flash/standard/pro/frontier), then maps that tier to a configured model.

## Why

1. **Cost optimization** — Simple requests ("hi", "what time is it") don't need expensive models
2. **User experience** — Simple requests return faster with lightweight models
3. **NEAR AI native** — Default backend uses NEAR AI inference where costs vary by model
4. **Zero-config value** — Users benefit immediately without configuration
5. **Not just power users** — Everyone gets smart defaults, power users can override

## How

### Architecture

```
User Message
     │
     ▼
┌──────────────────┐
│ Pattern Overrides │  ← Fast-path for obvious cases (greetings, security audits)
└────────┬─────────┘
         │ no match
         ▼
┌──────────────────┐
│ Complexity Scorer │  ← 13-dimension analysis
└────────┬─────────┘
         │ score 0-100
         ▼
┌──────────────────┐
│   Tier Mapping   │  ← 0-15: flash, 16-40: standard, 41-65: pro, 66+: frontier
└────────┬─────────┘
         │ tier
         ▼
┌──────────────────┐
│  Model Selection │  ← Config maps tier → model (e.g., flash → "haiku-latest")
└────────┬─────────┘
         │
         ▼
    LLM Provider
```

### Complexity Scorer (13 Dimensions)

Each dimension produces a 0-100 score. Weighted sum determines total.

| Dimension | Weight | Signals |
|-----------|--------|---------|
| Reasoning Words | 14% | "why", "explain", "compare", "trade-offs" |
| Token Estimate | 12% | Prompt length |
| Code Indicators | 10% | Backticks, syntax, "implement", "PR" |
| Multi-Step | 10% | "first", "then", "after", "steps" |
| Domain Specific | 10% | Technical terms (configurable) |
| Creativity | 7% | "write", "summarize", "tweet", "blog" |
| Question Complexity | 7% | Multiple questions, open-ended starters |
| Precision | 6% | Numbers, "exactly", "calculate" |
| Ambiguity | 5% | Vague references |
| Context Dependency | 5% | "previous", "you said" |
| Sentence Complexity | 5% | Commas, conjunctions, clause depth |
| Tool Likelihood | 5% | "read", "deploy", "install" |
| Safety Sensitivity | 4% | "password", "auth", "vulnerability" |

**Multi-dimensional boost:** +30% when 3+ dimensions score above threshold.

### Tier Boundaries

| Score | Tier | Typical Use Case |
|-------|------|------------------|
| 0-15 | flash | Greetings, acknowledgments, quick lookups |
| 16-40 | standard | Writing, comparisons, defined tasks |
| 41-65 | pro | Multi-step analysis, code review |
| 66+ | frontier | Critical decisions, security audits |

### Pattern Overrides

Fast-path rules that bypass scoring for obvious cases:

```yaml
# Force flash tier
- "^(hi|hello|hey|thanks|ok|sure|yes|no)$"
- "^what.*(time|date|day)"

# Force frontier tier
- "security.*(audit|review|scan)"
- "vulnerabilit(y|ies).*(review|scan|check|audit)"

# Force pro tier
- "deploy.*(mainnet|production)"
```

### Configuration

**Default (zero-config):**
```yaml
llm:
  routing:
    enabled: true  # default
```

**Power user overrides:**
```yaml
llm:
  routing:
    enabled: true
    tiers:
      flash: "claude-3-5-haiku-latest"
      standard: "claude-sonnet-4-5-latest"
      pro: "claude-sonnet-4-5-latest"
      frontier: "claude-opus-4-5-latest"
    thinking:
      pro: "low"
      frontier: "medium"
    overrides:
      - pattern: "my-custom-pattern"
        tier: "pro"
    domain_keywords:  # Custom keywords for your domain
      - "mycompany"
      - "myproduct"
      - "internal-tool"
```

If `domain_keywords` is not set, uses `DEFAULT_DOMAIN_KEYWORDS` which covers common web3/infra terms.

**Disable routing (pin model):**
```yaml
llm:
  routing:
    enabled: false
  model: "claude-opus-4-5"
```

**Bring your own keys:**
```yaml
llm:
  backend: anthropic
  api_key: "sk-..."
  routing:
    enabled: true  # still works with external providers
```

### Integration Points

1. **RoutingProvider** — New wrapper implementing `LlmProvider` trait (like `FailoverProvider`)
2. **Scorer** — Pure function, no I/O, fast (~1ms)
3. **Config schema** — Extend `LlmConfig` with `routing` section
4. **Telemetry** — Log routing decisions for observability

### Model Agnosticism

**Critical:** No hardcoded model names in the router logic itself.

- Tier→model mappings come from config
- Default mappings use `-latest` patterns where supported
- NEAR AI backend handles actual model resolution
- Router only knows about tiers

### Layers of Control

| Layer | User Type | Config |
|-------|-----------|--------|
| 1. Zero-config | Everyone | `routing.enabled: true` (default) |
| 2. Tier tuning | Power users | Custom `routing.tiers` mapping |
| 3. Pattern overrides | Power users | Custom `routing.overrides` |
| 4. Model pinning | Power users | `routing.enabled: false` + `model: X` |
| 5. Own API keys | Power users | `backend: anthropic` + `api_key` |

## Implementation Plan

1. [ ] Port scorer to Rust (`src/llm/routing/scorer.rs`)
2. [ ] Implement `RoutingProvider` wrapper (`src/llm/routing/mod.rs`)
3. [ ] Extend config schema (`src/config.rs`)
4. [ ] Wire into provider creation (`src/llm/mod.rs`)
5. [ ] Add telemetry/logging
6. [ ] Tests with real conversation samples
7. [ ] Codex + Gemini security review
8. [ ] Documentation update

## Expected Outcomes

- **50-70% cost reduction** for typical usage patterns
- **Faster responses** for simple requests
- **Zero config required** for default benefits
- **Full control** for power users who want it
