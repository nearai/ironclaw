# LLM Provider Configuration

IronClaw defaults to NEAR AI for model access, but supports any OpenAI-compatible
endpoint as well as Anthropic, Ollama, and Google Gemini directly. This guide covers
the most common configurations.

Shared retry / circuit-breaker / response-cache settings are configured on the
top-level `LlmConfig` with `LLM_*` environment variables. Legacy `NEARAI_*` and
bare historical names remain accepted for backward compatibility.

## Provider Overview

| Provider | Backend value | Requires API key | Notes |
|---|---|---|---|
| NEAR AI | `nearai` | OAuth (browser) | Default; multi-model |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Claude models |
| OpenAI | `openai` | `OPENAI_API_KEY` | GPT models |
| Google Gemini | `gemini_oauth` | OAuth (browser) | Gemini models; function calling |
| io.net | `ionet` | `IONET_API_KEY` | Intelligence API |
| Mistral | `mistral` | `MISTRAL_API_KEY` | Mistral models |
| Yandex AI Studio | `yandex` | `YANDEX_API_KEY` | YandexGPT models |
| MiniMax | `minimax` | `MINIMAX_API_KEY` | MiniMax-M2.7 models |
| Cloudflare Workers AI | `cloudflare` | `CLOUDFLARE_API_KEY` | Access to Workers AI |
| GitHub Copilot | `github_copilot` | `GITHUB_COPILOT_TOKEN` | Multi-models |
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

### Shared LLM decorator settings

These apply to all backends, not just NEAR AI:

```env
LLM_MAX_RETRIES=3
LLM_CIRCUIT_BREAKER_THRESHOLD=5
LLM_CIRCUIT_BREAKER_RECOVERY_SECS=30
LLM_RESPONSE_CACHE_ENABLED=false
LLM_RESPONSE_CACHE_TTL_SECS=3600
LLM_RESPONSE_CACHE_MAX_ENTRIES=1000
```

Legacy `NEARAI_MAX_RETRIES`, `CIRCUIT_BREAKER_THRESHOLD`,
`CIRCUIT_BREAKER_RECOVERY_SECS`, `RESPONSE_CACHE_ENABLED`,
`RESPONSE_CACHE_TTL_SECS`, and `RESPONSE_CACHE_MAX_ENTRIES` are still read as
fallbacks for compatibility.

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

## Google Gemini (OAuth)

Uses Google OAuth with PKCE (S256) for authentication — no API key required.
On first run, a browser opens for Google account login. Credentials (including
refresh token) are saved to `~/.gemini/oauth_creds.json` with `0600` permissions.

```env
LLM_BACKEND=gemini_oauth
GEMINI_MODEL=gemini-2.5-flash
```

### Supported features

| Feature | Status | Notes |
|---|---|---|
| Function calling | ✅ | `functionDeclarations` / `functionCall` / `functionResponse` |
| `generationConfig` | ✅ | `temperature`, `maxOutputTokens` passed from request |
| `thinkingConfig` | ✅ | `thinkingBudget`/`thinkingLevel` for thinking-capable models (does NOT set `includeThoughts`) |
| `toolConfig` | ✅ | `functionCallingConfig.mode`: `AUTO`/`ANY`/`NONE` |
| SSE streaming | ✅ | Cloud Code API with `streamGenerateContent?alt=sse` |
| Token refresh | ✅ | Automatic via refresh token |

### Popular models

| Model | ID | Notes |
|---|---|---|
| Gemini 3.1 Pro | `gemini-3.1-pro-preview` | Latest, strongest reasoning |
| Gemini 3.1 Pro Custom Tools | `gemini-3.1-pro-preview-customtools` | Enhanced tool use |
| Gemini 3 Pro | `gemini-3-pro-preview` | Preview |
| Gemini 3 Flash | `gemini-3-flash-preview` | Fast preview with thinking |
| Gemini 3.1 Flash Lite | `gemini-3.1-flash-lite-preview` | Preview, lightweight |
| Gemini 2.5 Pro | `gemini-2.5-pro` | Stable, strong reasoning |
| Gemini 2.5 Flash | `gemini-2.5-flash` | Fast, good quality |
| Gemini 2.5 Flash Lite | `gemini-2.5-flash-lite` | Fastest, lightweight |

### Cloud Code API vs standard API

Models containing `-preview` (with hyphen) or `gemini-3` in the name, as well
as any `gemini-` model with major version >= 2, route through the Cloud Code
API (`cloudcode-pa.googleapis.com`) which supports SSE streaming
and project-scoped access. Other models use the standard Generative Language
API (`generativelanguage.googleapis.com`).

---

## GitHub Copilot

GitHub Copilot exposes chat endpoint at
`https://api.githubcopilot.com`. IronClaw uses that endpoint directly through the
built-in `github_copilot` provider.

```env
LLM_BACKEND=github_copilot
GITHUB_COPILOT_TOKEN=gho_...
GITHUB_COPILOT_MODEL=gpt-4o
# Optional advanced headers if your setup needs them:
# GITHUB_COPILOT_EXTRA_HEADERS=Copilot-Integration-Id:vscode-chat
```

`ironclaw onboard` can acquire this token for you using GitHub device login. If you
already signed into Copilot through VS Code or a JetBrains IDE, you can also reuse
the `oauth_token` stored in `~/.config/github-copilot/apps.json`. If you prefer,
`LLM_BACKEND=github-copilot` also works as an alias.

Popular models vary by subscription, but `gpt-4o` is a safe default. IronClaw keeps
model entry manual for this provider because GitHub Copilot model listing may require
extra integration headers on some clients. IronClaw automatically injects the standard
VS Code identity headers (`User-Agent`, `Editor-Version`, `Editor-Plugin-Version`,
`Copilot-Integration-Id`) and lets you override them with
`GITHUB_COPILOT_EXTRA_HEADERS`.

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

## MiniMax

[MiniMax](https://platform.minimax.io) provides high-performance language models with 204,800 token context windows.

```env
LLM_BACKEND=minimax
MINIMAX_API_KEY=...
```

Available models: `MiniMax-M2.7` (default), `MiniMax-M2.7-highspeed`, `MiniMax-M2.5`, `MiniMax-M2.5-highspeed`

To use the China mainland endpoint, set:

```env
MINIMAX_BASE_URL=https://api.minimaxi.com/v1
```

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
