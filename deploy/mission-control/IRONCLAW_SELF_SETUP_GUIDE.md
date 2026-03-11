# IronClaw Self-Setup Guide (WSL Workstation)

Exact values for this workstation. Align with [src/setup/README.md](../../src/setup/README.md) for onboarding behavior.

## Environment Variables

### Router-First (Supported Path)

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=http://127.0.0.1:8080/v1
LLM_MODEL=qwen3.5-9b
LLM_API_KEY=local
LLM_REQUEST_TIMEOUT_SECS=120
```

For 27B primary tier (when vLLM 27B upstream is running):

```env
LLM_MODEL=qwen3.5-27b
```

### Direct Ollama (Comparison Lane)

```env
LLM_BACKEND=ollama
OLLAMA_BASE_URL=http://127.0.0.1:11434
OLLAMA_MODEL=qwen3.5:9b
```

## Router Config

`scripts/llm-cluster-router/router.sample.yml` (or custom `router.yml`):

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
```

## GPU UUIDs

Get from host:

```bash
nvidia-smi -L
```

Use in router node config if you need explicit GPU binding (router forwards to upstream URLs; vLLM/Ollama handle GPU binding).

## Benchmark / Probe Commands

```bash
# GPU probe
cd ironclaw/scripts/llm-cluster-router
go run . probe-gpu -output ../../target/mission-control-gpu-probe.json

# Router benchmark (small-model proof)
go run . bench -url http://127.0.0.1:8080 -model qwen3.5-9b -requests 8 -concurrency 2 -output ../../target/mission-control-benchmark.json

# Optional: Ollama direct comparison
go run . bench -url http://127.0.0.1:11434/v1 -model qwen3.5:9b -requests 8 -concurrency 2 -output ../../target/mission-control-benchmark-ollama-direct.json
```

## Docker Compose Startup (Control Plane)

```bash
cd ironclaw/deploy/mission-control
docker compose up -d
```

Services: Prometheus (9090), Grafana (3001), node-exporter, dcgm-exporter.

## Host Prerequisites (Unavoidable)

1. **llm-cluster-router** — run on host (port 8080)
2. **vLLM or Ollama** — run on host (ports 8001, 8002, or 11434)
3. **IronClaw** — run on host (port 3000)
4. **NVIDIA drivers** — for GPU probe and model serving

## Startup Order

1. Start vLLM/Ollama upstreams (8001, 8002 or 11434)
2. Start `llm-cluster-router` with config
3. Start IronClaw (`cargo run` or `ironclaw`)
4. Start Mission Control stack: `docker compose up -d`
5. Run benchmark and probe to refresh artifacts
6. Verify via [MANUAL_VERIFICATION_CHECKLIST.md](./MANUAL_VERIFICATION_CHECKLIST.md)

---

*Last Updated: 2026-03-11*
