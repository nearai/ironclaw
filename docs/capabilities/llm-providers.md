---
title: Inference Providers
description: IronClaw readily supports multiple LLM providers
---

IronClaw ships with a catalog of more than twenty inference providers, including NEAR AI, Anthropic, OpenAI, Google Gemini, GitHub Copilot, Ollama, AWS Bedrock, and any OpenAI-compatible endpoint.

## Configuring a Provider

List what your install knows about, then pick one:

```bash
ironclaw models list
ironclaw models set-provider anthropic --model claude-sonnet-4-20250514
ironclaw models status
```

You can also choose a provider during [onboarding](/onboard), or from **Settings → Inference** in the [web interface](/using/webui).

Whichever route you take, the selection is written to `config.toml` as a model slot:

```toml
[llm.default]
provider_id = "anthropic"
model       = "claude-sonnet-4-20250514"
api_key_env = "ANTHROPIC_API_KEY"
```

`api_key_env` names the environment variable holding your key. Never paste the key itself — that is rejected at parse time. See [Configuration](/capabilities/configuration).

If no slot is configured, IronClaw falls back to whichever provider environment variables are set.

---

## Provider Overview

`provider_id` is the value used with `models set-provider` and in `[llm.default]`.

| Provider              | `provider_id`       | Requires               | Notes                                        |
|-----------------------|---------------------|------------------------|----------------------------------------------|
| NEAR AI               | `nearai`            | `NEARAI_API_KEY`       | Default; multi-model access via NEAR account |
| Anthropic             | `anthropic`         | `ANTHROPIC_API_KEY`    | Claude models                                |
| OpenAI                | `openai`            | `OPENAI_API_KEY`       | GPT models                                   |
| OpenAI (Codex)        | `openai_codex`      | ChatGPT subscription   | Plus, Pro, or Max plans                      |
| Google Gemini         | `gemini`            | `GEMINI_API_KEY`       | Native API; preserves thought signatures     |
| Google Gemini (OAuth) | `gemini_oauth`      | OAuth (browser)        | Official API via Gemini CLI sign-in          |
| GitHub Copilot        | `github_copilot`    | `GITHUB_COPILOT_TOKEN` | Copilot Chat API; token from IDE sign-in     |
| AWS Bedrock           | `bedrock`           | AWS credentials        | IAM or SSO                                   |
| Ollama                | `ollama`            | No                     | Local inference                              |
| Tinfoil               | `tinfoil`           | `TINFOIL_API_KEY`      | Hardware-attested TEE private inference       |
| OpenRouter            | `openrouter`        | `OPENROUTER_API_KEY`   | 200+ models; preserves reasoning across turns |
| Groq                  | `groq`              | `GROQ_API_KEY`         | LPU inference                                 |
| DeepSeek              | `deepseek`          | `DEEPSEEK_API_KEY`     | Preserves reasoning content                   |
| Mistral               | `mistral`           | `MISTRAL_API_KEY`      | Mistral models                                |
| Together AI           | `together`          | `TOGETHER_API_KEY`     | Together inference                            |
| Fireworks AI          | `fireworks`         | `FIREWORKS_API_KEY`    | Fireworks inference                           |
| Cerebras              | `cerebras`          | `CEREBRAS_API_KEY`     | Wafer-scale inference                         |
| SambaNova             | `sambanova`         | `SAMBANOVA_API_KEY`    | SambaNova Cloud                               |
| NVIDIA                | `nvidia`            | `NVIDIA_API_KEY`       | NIM API                                       |
| Venice                | `venice`            | `VENICE_API_KEY`       | Privacy-focused inference                     |
| Z.AI                  | `zai`               | `ZAI_API_KEY`          | GLM models                                    |
| MiniMax               | `minimax`           | `MINIMAX_API_KEY`      | MiniMax-M2 models                             |
| Cloudflare Workers AI | `cloudflare`        | `CLOUDFLARE_API_KEY`   | Workers AI                                    |
| io.net                | `ionet`             | `IONET_API_KEY`        | Intelligence API                              |
| Yandex AI Studio      | `yandex`            | `YANDEX_API_KEY`       | YandexGPT models                              |
| OpenAI-compatible     | `openai_compatible` | `LLM_API_KEY`          | vLLM, LiteLLM, LM Studio, any compatible host |

Run `ironclaw models list` for the catalog on your install, including the default model for each provider and any entries you have added yourself through `providers.json`.

---

## NEAR AI

```env
NEARAI_MODEL=claude-3-5-sonnet-20241022
NEARAI_BASE_URL=https://private.near.ai
```

Popular models: `Qwen/Qwen3.5-122B-A10B`, `black-forest-labs/FLUX.2-klein-4B`, `zai-org/GLM-5-FP8`

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

| Feature            | Status | Notes                                                                                         |
|--------------------|--------|-----------------------------------------------------------------------------------------------|
| Function calling   | ✅      | `functionDeclarations` / `functionCall` / `functionResponse`                                  |
| `generationConfig` | ✅      | `temperature`, `maxOutputTokens` passed from request                                          |
| `thinkingConfig`   | ✅      | `thinkingBudget`/`thinkingLevel` for thinking-capable models (does NOT set `includeThoughts`) |
| `toolConfig`       | ✅      | `functionCallingConfig.mode`: `AUTO`/`ANY`/`NONE`                                             |
| SSE streaming      | ✅      | Cloud Code API with `streamGenerateContent?alt=sse`                                           |
| Token refresh      | ✅      | Automatic via refresh token                                                                   |

### Popular models

| Model                       | ID                                   | Notes                       |
|-----------------------------|--------------------------------------|-----------------------------|
| Gemini 3.1 Pro              | `gemini-3.1-pro-preview`             | Latest, strongest reasoning |
| Gemini 3.1 Pro Custom Tools | `gemini-3.1-pro-preview-customtools` | Enhanced tool use           |
| Gemini 3 Pro                | `gemini-3-pro-preview`               | Preview                     |
| Gemini 3 Flash              | `gemini-3-flash-preview`             | Fast preview with thinking  |
| Gemini 3.1 Flash Lite       | `gemini-3.1-flash-lite-preview`      | Preview, lightweight        |
| Gemini 2.5 Pro              | `gemini-2.5-pro`                     | Stable, strong reasoning    |
| Gemini 2.5 Flash            | `gemini-2.5-flash`                   | Fast, good quality          |
| Gemini 2.5 Flash Lite       | `gemini-2.5-flash-lite`              | Fastest, lightweight        |

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

| Prefix    | Routing                                      |
|-----------|----------------------------------------------|
| `us`      | US regions (us-east-1, us-east-2, us-west-2) |
| `eu`      | European regions                             |
| `apac`    | Asia-Pacific regions                         |
| `global`  | All commercial AWS regions                   |
| _(unset)_ | Single-region only                           |

### Popular Bedrock model IDs

| Model             | ID                                          |
|-------------------|---------------------------------------------|
| Claude Opus 4.6   | `anthropic.claude-opus-4-6-v1`              |
| Claude Sonnet 4.5 | `anthropic.claude-sonnet-4-5-20250929-v1:0` |
| Claude Haiku 4.5  | `anthropic.claude-haiku-4-5-20251001-v1:0`  |
| Amazon Nova Pro   | `amazon.nova-pro-v1:0`                      |
| Llama 4 Maverick  | `meta.llama4-maverick-17b-instruct-v1:0`    |

---

## OpenAI-Compatible Endpoints

OpenRouter, Together AI, Fireworks and the rest now have their own `provider_id` entries — use those directly rather than the generic adapter.

Reach for `openai_compatible` when your endpoint has no dedicated entry: vLLM, LiteLLM, LM Studio, or an internal gateway. It needs a `base_url`, because the generic adapter has no default host of its own:

```toml
[llm.default]
provider_id = "openai_compatible"
base_url    = "http://localhost:8000/v1"
model       = "meta-llama/Llama-3.3-70B-Instruct"
api_key_env = "LLM_API_KEY"
```

Omitting `base_url` leaves the slot pointing nowhere and model resolution fails. `base_url` also works on any other provider when you need to route it through a proxy or a regional endpoint.

The sections below list model ids for popular hosts. Each is reachable through its own `provider_id`; set `model` to the id you want.

Several examples on this page use the `LLM_BACKEND` / `LLM_MODEL` / `LLM_BASE_URL` / `LLM_API_KEY` environment variables instead of a TOML slot. Both work: the environment form is the fallback IronClaw uses when `[llm.default]` is not configured. Prefer the TOML slot for a permanent install, and the environment form for one-off runs and containers.

### OpenRouter

[OpenRouter](https://openrouter.ai) routes to 300+ models from a single API key.

```toml
[llm.default]
provider_id = "openrouter"
model       = "anthropic/claude-sonnet-4"
api_key_env = "OPENROUTER_API_KEY"
```

Popular OpenRouter model IDs:

| Model            | ID                                         |
|------------------|--------------------------------------------|
| Claude Sonnet 4  | `anthropic/claude-sonnet-4`                |
| GPT-4o           | `openai/gpt-4o`                            |
| Llama 4 Maverick | `meta-llama/llama-4-maverick`              |
| Gemini 2.0 Flash | `google/gemini-2.0-flash-001`              |
| Mistral Small    | `mistralai/mistral-small-3.1-24b-instruct` |

Browse all models at [openrouter.ai/models](https://openrouter.ai/models).

### Together AI

[Together AI](https://www.together.ai) provides fast inference for open-source models.

```bash
ironclaw models set-provider together --model meta-llama/Llama-3.3-70B-Instruct-Turbo
export TOGETHER_API_KEY=...
```

Popular Together AI model IDs:

| Model         | ID                                        |
|---------------|-------------------------------------------|
| Llama 3.3 70B | `meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| DeepSeek R1   | `deepseek-ai/DeepSeek-R1`                 |
| Qwen 2.5 72B  | `Qwen/Qwen2.5-72B-Instruct-Turbo`         |

### Fireworks AI

[Fireworks AI](https://fireworks.ai) offers fast inference with compound AI system support.

```bash
ironclaw models set-provider fireworks --model accounts/fireworks/models/llama4-maverick-instruct-basic
export FIREWORKS_API_KEY=fw_...
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