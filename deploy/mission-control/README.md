# Mission Control Stack

This directory contains the local observability and staging assets for the WSL-first
IronClaw mission-control path.

## Local Monitoring

```bash
cd deploy/mission-control
docker compose up -d
```

Endpoints:

- Prometheus: `http://127.0.0.1:9090`
- Grafana: `http://127.0.0.1:3001`
- Router metrics: `http://127.0.0.1:9091/metrics`

Before bringing the stack up, start:

1. `llm-cluster-router` on `127.0.0.1:8080`
2. primary vLLM on `127.0.0.1:8001`
3. optional secondary vLLM on `127.0.0.1:8002`

## Benchmark Flow

```bash
cd scripts/llm-cluster-router
go run . bench \
  -url http://127.0.0.1:8080 \
  -model qwen3.5-27b \
  -requests 8 \
  -concurrency 2 \
  -output ~/.ironclaw/benchmarks/latest.json
```

The web gateway mission-control tab reads `~/.ironclaw/benchmarks/latest.json` by default.

For this workspace, the checked-in local artifact lives at:

- `target/mission-control-benchmark.json`

That file should continue to represent the last known-good baseline. Comparison runs that
are expected to fail should not replace the canonical operator snapshot without review.

For the current post-restart proof on this workstation, keep the paired GPU evidence nearby:

- `target/mission-control-benchmark.json`
- `target/mission-control-gpu-probe.json`
- `target/mission-control-benchmark-ollama-direct.json`

## Runtime Split

- Use Docker/Compose for the control-plane services in this directory.
- Keep model-serving on the host as the only special-case runtime until the GPU-backed app
  containers are promoted deliberately.
- Route all local model traffic through `llm-cluster-router` so IronClaw sees one
  OpenAI-compatible endpoint and Mission Control can keep one benchmark contract.

Current post-restart local proof:

- local Ollama serves `qwen3.5-9b`
- the router exposes the proof lane on `127.0.0.1:8080`
- the benchmark and `ironclaw_chat` smoke path are both green again
- the paired GPU probe shows the resident model load on the RTX 3090

## Ollama Comparison Track

The router now tolerates Ollama-style health checks by falling back from `/health` to
`/v1/models`. That allows a contained comparison track without editing the tracked router
config.

Current comparison outcome on this host after the restart:

- the router-backed proof lane is healthy
- direct Ollama also benchmarks successfully with the same small model
- keep the router-backed path as the primary IronClaw endpoint because it preserves the
  single-endpoint contract, queue metrics, and mission-control health surface
- treat direct Ollama as a latency/throughput comparison artifact, not as the default app
  wiring yet

## k3s / Terraform

The staged single-node k3s assets live in `deploy/mission-control/k3s/terraform`.

The local benchmark-first path is green again, but the single-node manifests should still
be promoted deliberately rather than automatically. Keep the same KPI contract when moving
to k3s:

1. the local benchmark is repeatable
2. the GPU runtime is trustworthy again
3. the same Mission Control KPI contract is preserved in-cluster
