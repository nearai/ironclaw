# Experimental: llama.cpp 27B Track

**Status:** Experimental. Not the default IronClaw path. Do not promote to default unless it wins on the same benchmark + observability contract as the router-first baseline.

## Overview

This track evaluates `llama.cpp` / `llama-server` with Qwen 3.5 27B on asymmetric multi-GPU hardware (RTX 3090 + RTX 4070 Ti Super). It materially changes the serving architecture and tuning assumptions from the current repo-supported router-first baseline.

## Hardware Assumptions

| GPU | VRAM | Role |
|-----|------|------|
| RTX 3090 | 24 GB | Primary (tensor-split majority) |
| RTX 4070 Ti Super | 16 GB | Secondary |

## Why Experimental

- Different serving stack: `llama-server` instead of vLLM
- Different quantization and KV cache layout
- No router integration yet; would require a new upstream type
- Tuning assumptions (tensor-split, split-mode) differ from vLLM baseline
- Observability contract (Prometheus, queue metrics) not yet aligned with Mission Control

## Suggested Configuration

```bash
# UD-Q6_K quantization (~24GB), bf16 KV cache, 32K context
./llama-server \
  --model /path/to/Qwen3.5-27B-Instruct-UD-Q6_K.gguf \
  --tensor-split 60,40 \
  --split-mode row \
  --override-tensor "token_embd.weight=CUDA0" \
  --no-mmap \
  --fit on \
  --flash-attn on \
  --jinja \
  --chat-template-kwargs '{"enable_thinking": true}'
```

## Environment Values (for comparison runs only)

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://127.0.0.1:8080/v1   # or direct llama-server port
LLM_MODEL=qwen3.5-27b
LLM_API_KEY=local
```

## KPI Gate for Promotion

Before promoting this track to the supported baseline:

1. Same benchmark contract as router-first: TTFT p50/p95, latency, tokens/sec, success/failure counts
2. Same GPU evidence contract: UUID, PCI bus, VRAM, utilization, temperature, process binding
3. Mission Control integration: benchmark and probe artifacts consumable by the web gateway
4. `ironclaw_chat` smoke passes end-to-end via the same harness
5. No regression in queue depth, latency, or VRAM stability vs. current vLLM baseline

## Comparison Modes

| Mode | Status | Notes |
|------|--------|-------|
| Router-first (vLLM) | Supported | Primary path |
| Direct Ollama | Comparison | Tactical comparison lane |
| llama.cpp 27B | Experimental | This document |

---

*Last Updated: 2026-03-11*
