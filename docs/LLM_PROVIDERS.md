# LLM Provider Configuration

IronClaw defaults to NEAR AI for model access, but supports any OpenAI-compatible
endpoint as well as Anthropic and Ollama directly. This guide covers the most common
configurations.

## Provider Overview

| Provider | Backend value | Requires API key | Notes |
|---|---|---|---|
| NEAR AI | `nearai` | OAuth (browser) | Default; multi-model |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Claude models |
| OpenAI | `openai` | `OPENAI_API_KEY` | GPT models |
| Google Gemini | `gemini` | `GEMINI_API_KEY` | Gemini models |
| io.net | `ionet` | `IONET_API_KEY` | Intelligence API |
| Mistral | `mistral` | `MISTRAL_API_KEY` | Mistral models |
| Yandex AI Studio | `yandex` | `YANDEX_API_KEY` | YandexGPT models |
| Cloudflare Workers AI | `cloudflare` | `CLOUDFLARE_API_KEY` | Access to Workers AI |
| Ollama | `ollama` | No | Local inference |
| AWS Bedrock | `bedrock` | AWS credentials | Native Converse API |
| OpenRouter | `openai_compatible` | `LLM_API_KEY` | 300+ models |
| Together AI | `openai_compatible` | `LLM_API_KEY` | Fast inference |
| Fireworks AI | `openai_compatible` | `LLM_API_KEY` | Fast inference |
| vLLM / LiteLLM | `openai_compatible` | Optional | Self-hosted |
| LM Studio | `openai_compatible` | No | Local GUI |

---

## NEAR AI (default)

No additional configuration required. On first run, `ironclaw onboard` opens a browser
for OAuth authentication. Credentials are saved to `~/.ironclaw/session.json`.

```env
NEARAI_MODEL=claude-3-5-sonnet-20241022
NEARAI_BASE_URL=https://private.near.ai
```

---

## Anthropic (Claude)

```env
LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...
```

Popular models: `claude-sonnet-4-20250514`, `claude-3-5-sonnet-20241022`, `claude-3-5-haiku-20241022`

---

## OpenAI (GPT)

```env
LLM_BACKEND=openai
OPENAI_API_KEY=sk-...
```

Popular models: `gpt-4o`, `gpt-4o-mini`, `o3-mini`

---

## Ollama (local)

Install Ollama from [ollama.com](https://ollama.com), pull a model, then:

```env
LLM_BACKEND=ollama
OLLAMA_MODEL=llama3.2
# OLLAMA_BASE_URL=http://localhost:11434   # default
```

Pull a model first: `ollama pull llama3.2`

---

## AWS Bedrock (requires `--features bedrock`)

Uses the native AWS Converse API via `aws-sdk-bedrockruntime`. Supports standard AWS
authentication methods: IAM credentials, SSO profiles, and instance roles.

> **Build prerequisite:** The `aws-lc-sys` crate (transitive dependency via AWS SDK)
> requires **CMake** to compile. Install it before building with `--features bedrock`:
> - macOS: `brew install cmake`
> - Ubuntu/Debian: `sudo apt install cmake`
> - Fedora: `sudo dnf install cmake`

### With AWS credentials (IAM, SSO, instance roles)

```env
LLM_BACKEND=bedrock
BEDROCK_MODEL=anthropic.claude-opus-4-6-v1
BEDROCK_REGION=us-east-1
BEDROCK_CROSS_REGION=us
# AWS_PROFILE=my-sso-profile   # optional, for named profiles
```

The AWS SDK credential chain automatically resolves credentials from environment
variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`), shared credentials file
(`~/.aws/credentials`), SSO profiles, and EC2/ECS instance roles.

### Cross-region inference

Set `BEDROCK_CROSS_REGION` to route requests across AWS regions for capacity:

| Prefix | Routing |
|---|---|
| `us` | US regions (us-east-1, us-east-2, us-west-2) |
| `eu` | European regions |
| `apac` | Asia-Pacific regions |
| `global` | All commercial AWS regions |
| _(unset)_ | Single-region only |

### Popular Bedrock model IDs

| Model | ID |
|---|---|
| Claude Opus 4.6 | `anthropic.claude-opus-4-6-v1` |
| Claude Sonnet 4.5 | `anthropic.claude-sonnet-4-5-20250929-v1:0` |
| Claude Haiku 4.5 | `anthropic.claude-haiku-4-5-20251001-v1:0` |
| Amazon Nova Pro | `amazon.nova-pro-v1:0` |
| Llama 4 Maverick | `meta.llama4-maverick-17b-instruct-v1:0` |

---

## OpenAI-Compatible Endpoints

All providers below use `LLM_BACKEND=openai_compatible`. Set `LLM_BASE_URL` to the
provider's OpenAI-compatible endpoint and `LLM_API_KEY` to your API key.

### Local llm-cluster-router Baseline

For the supported local IronClaw path, place the Go `llm-cluster-router` in front of
all local vLLM backends and point IronClaw at the router as a single
OpenAI-compatible endpoint:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://127.0.0.1:8080/v1
LLM_API_KEY=local
LLM_MODEL=qwen3.5-27b
LLM_REQUEST_TIMEOUT_SECS=120
```

Recommended host baseline for the current workstation:

| Role | GPU | Model | Purpose | Guardrails |
|---|---|---|---|---|
| Primary agent tier | RTX 3090 24 GB | `Qwen 3.5 27B` | Default IronClaw reasoning and tool calling | `--gpu-memory-utilization 0.88`, `--max-model-len 32768`, queue depth `<= 8`, concurrency `1-2` |
| Secondary fast tier | RTX 4070 Ti Super 16 GB | `Qwen 3.5 9B` or `Qwen 3.5 35B-A3B` | Burst capacity, drafts, lighter tasks | `--gpu-memory-utilization 0.85`; prefer `9B` for stability, keep `35B-A3B` as optional secondary capacity only |

Operational rules for this host:

- Keep `Qwen 3.5 27B` as the default IronClaw model. Do not promote `35B-A3B` to the primary agent tier.
- Route all local traffic through the external Go router so IronClaw sees one stable endpoint and upstream health/queue state can be enforced centrally.
- Treat the 4070 Ti Super as a swap-able secondary tier: run either `9B` for safer headroom or `35B-A3B` when throughput matters more than first-pass tool reliability.
- Cap router queue depth and concurrency before raising vLLM memory limits. Backpressure is safer than running GPUs above their stable VRAM envelope.
- Consider any sustained GPU memory usage above 90%, queue growth without drain, or repeated request timeouts as a failing configuration that should trigger a lower `max-model-len` or lower concurrency.

Recommended `llm-cluster-router` baseline for this workstation:

```yaml
listen: ":8080"
metrics_addr: ":9091"
log_level: info

defaults:
  max_queue_depth: 8
  max_concurrency: 2
  request_timeout: 120s
  max_body_size: 1048576

nodes:
  - name: workstation-3090-agent
    url: http://127.0.0.1:8001
    tier: agent
    weight: 4
    models: ["qwen3.5-27b"]
  - name: workstation-4070-fast
    url: http://127.0.0.1:8002
    tier: fast
    weight: 2
    models: ["qwen3.5-9b"]

tiers:
  agent:
    models: ["qwen3.5-27b"]
    prefer_nodes: ["workstation-3090-agent"]
  fast:
    models: ["qwen3.5-9b"]
    prefer_nodes: ["workstation-4070-fast"]
  heavy:
    models: ["qwen3.5-27b"]
    prefer_nodes: ["workstation-3090-agent"]

health_check:
  interval: 15s
  timeout: 5s
  path: /health
  unhealthy_threshold: 3
  healthy_threshold: 1
```

Notes for the mixed-GPU host:

- Start with `Qwen 3.5 9B` on the 4070 Ti Super, not `35B-A3B`. Only promote `35B-A3B` after the `9B` path is stable and you still have headroom for the required context window.
- Keep the primary 3090 queue shallow. A queue depth of `8` and concurrency of `1-2` is preferred over a larger queue that hides overload until latency spikes.
- Use router backpressure as the first safety valve. If requests start timing out or queue depth stops draining, lower concurrency or `max-model-len` before touching GPU memory utilization.

Validation checklist for the local 27B path:

1. Upstreams are healthy:
   - `curl -sf http://127.0.0.1:8001/health`
   - `curl -sf http://127.0.0.1:8002/health`
   - `curl -sf http://127.0.0.1:8080/healthz`
2. The router exposes the expected models:
   - `curl -sf http://127.0.0.1:8080/v1/models`
   - Confirm `qwen3.5-27b` is present and is the model IronClaw uses via `LLM_MODEL`
3. VRAM stays inside the supported envelope during a real IronClaw run:
   - RTX 3090 remains below ~90% VRAM
   - RTX 4070 Ti Super remains below ~88% VRAM
   - No CUDA OOM or repeated allocator warnings appear in vLLM logs
4. Queue and latency remain bounded:
   - router queue depth returns to zero after the burst
   - first-token latency stays acceptable for the agent tier
   - request timeouts remain rare or zero
5. IronClaw remains stable under tool-calling load:
   - `ironclaw_chat` completes through the gateway
   - the end-to-end smoke harness finishes without OOM

If the validation checklist fails, reduce `max-model-len`, reduce router concurrency, or demote the 4070 tier back to `9B` before changing any other variable.

### Gemini CLI (secondary operator track)

`gemini-cli` is not a first-class `LLM_BACKEND` in the current Rust provider registry.
Treat it as an operator-side or MCP-adjacent fallback, not as the primary local
runtime path.

Use this positioning rule:

- Primary local path: `ironclaw-mcp -> IronClaw -> llm-cluster-router -> local Qwen 27B`
- Secondary fallback path: Gemini CLI invoked externally by the operator or bridged through a separate tool/MCP layer

Operational guidance:

- Do not replace the default local `openai_compatible` router path with Gemini CLI in the overnight proof path.
- Do not add a `gemini_cli` backend to `providers.json` or `src/config/llm.rs` without a separate implementation task.
- If Gemini CLI is introduced later, keep it modular so future IronClaw installs can enable or disable it without changing the core local Qwen path.

### OpenRouter

[OpenRouter](https://openrouter.ai) routes to 300+ models from a single API key.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

Popular OpenRouter model IDs:

| Model | ID |
|---|---|
| Claude Sonnet 4 | `anthropic/claude-sonnet-4` |
| GPT-4o | `openai/gpt-4o` |
| Llama 4 Maverick | `meta-llama/llama-4-maverick` |
| Gemini 2.0 Flash | `google/gemini-2.0-flash-001` |
| Mistral Small | `mistralai/mistral-small-3.1-24b-instruct` |

Browse all models at [openrouter.ai/models](https://openrouter.ai/models).

### Together AI

[Together AI](https://www.together.ai) provides fast inference for open-source models.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.together.xyz/v1
LLM_API_KEY=...
LLM_MODEL=meta-llama/Llama-3.3-70B-Instruct-Turbo
```

Popular Together AI model IDs:

| Model | ID |
|---|---|
| Llama 3.3 70B | `meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| DeepSeek R1 | `deepseek-ai/DeepSeek-R1` |
| Qwen 2.5 72B | `Qwen/Qwen2.5-72B-Instruct-Turbo` |

### Fireworks AI

[Fireworks AI](https://fireworks.ai) offers fast inference with compound AI system support.

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://api.fireworks.ai/inference/v1
LLM_API_KEY=fw_...
LLM_MODEL=accounts/fireworks/models/llama4-maverick-instruct-basic
```

### vLLM / LiteLLM (self-hosted)

For self-hosted inference servers:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:8000/v1
LLM_API_KEY=token-abc123        # set to any string if auth is not configured
LLM_MODEL=meta-llama/Llama-3.1-8B-Instruct
```

LiteLLM proxy (forwards to any backend, including Bedrock, Vertex, Azure):

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:4000/v1
LLM_API_KEY=sk-...
LLM_MODEL=gpt-4o                 # as configured in litellm config.yaml
```

### LM Studio (local GUI)

Start LM Studio's local server, then:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://localhost:1234/v1
LLM_MODEL=llama-3.2-3b-instruct-q4_K_M
# LLM_API_KEY is not required for LM Studio
```

---

## Using the Setup Wizard

Instead of editing `.env` manually, run the onboarding wizard:

```bash
ironclaw onboard
```

Select **"OpenAI-compatible"** for OpenRouter, Together AI, Fireworks, vLLM, LiteLLM,
or LM Studio. You will be prompted for the base URL and (optionally) an API key.
The model name is configured in the following step.
