# IronClaw Mission Control — Manual Verification Checklist

Use this checklist after restart, config change, or before relying on the local agent path.

## 1. GPU Probe

```bash
cd ironclaw/scripts/llm-cluster-router
go run . probe-gpu -output ../../target/mission-control-gpu-probe.json
```

- [ ] Output file exists and contains GPU UUID, PCI bus, VRAM used/total, utilization, temperature
- [ ] Intended GPUs present (RTX 3090, RTX 4070 Ti Super)

## 2. Router Health

```bash
curl -sf http://127.0.0.1:8080/healthz
curl -sf http://127.0.0.1:8080/v1/models
```

- [ ] `/healthz` returns 200
- [ ] `/v1/models` lists expected models (e.g. `qwen3.5-9b`, `qwen3.5-27b`)

## 3. Upstream Health (if vLLM/Ollama)

```bash
curl -sf http://127.0.0.1:8001/health   # or /v1/models for Ollama-style
curl -sf http://127.0.0.1:8002/health   # secondary
```

- [ ] Primary upstream (8001) healthy
- [ ] Secondary upstream (8002) healthy if configured

## 4. Benchmark Freshness

```bash
cd ironclaw/scripts/llm-cluster-router
go run . bench -url http://127.0.0.1:8080 -model qwen3.5-9b -requests 8 -concurrency 2 -output ../../target/mission-control-benchmark.json
```

- [ ] Benchmark completes without errors
- [ ] `target/mission-control-benchmark.json` exists and has valid TTFT/latency/tokens data

## 5. Mission Control API/UI

```bash
cd ironclaw/deploy/mission-control
docker compose up -d
```

- [ ] Prometheus: `http://127.0.0.1:9090`
- [ ] Grafana: `http://127.0.0.1:3001`
- [ ] Router metrics: `http://127.0.0.1:9091/metrics`
- [ ] IronClaw web gateway Mission Control tab shows benchmark and GPU probe data

## 6. Chat Smoke

```bash
cd ironclaw-mcp
SMOKE_STATEFUL_TOOL=ironclaw_chat make smoke
```

- [ ] `ironclaw_health` passes
- [ ] `ironclaw_chat` completes (or skip if router/model not running)

## 7. Prometheus/Grafana

- [ ] Prometheus scraping router metrics
- [ ] Grafana dashboards load
- [ ] No critical alerts firing

## 8. Git Remote/Branch Sanity

- [ ] `ironclaw`: `feat/mission-control-next-phase` on `fork`
- [ ] `ironclaw-mcp`: `feat/local-chat-smoke` on `origin`
- [ ] `global-kb`: `main` in sync

---

*Last Updated: 2026-03-11*
