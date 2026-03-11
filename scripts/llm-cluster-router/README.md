# llm-cluster-router

Minimal Go router and benchmark harness for the local IronClaw GPU path.

## Commands

```bash
go run . serve -config ./router.sample.yml
go run . bench -url http://127.0.0.1:8080 -model qwen3.5-27b -requests 8 -concurrency 2
go run . probe-gpu
```

## Endpoints

- `/healthz`: router health plus per-node status
- `/v1/models`: healthy-model inventory
- `/v1/chat/completions`: OpenAI-compatible proxy
- `/metrics`: Prometheus metrics
- `/debug/pprof/*`: optional debug server when `debug_addr` is set

Health probing is compatible with both:

- vLLM-style upstreams that expose `/health`
- Ollama-style upstreams that expose `/v1/models` but not `/health`

The router will probe the configured health path first and fall back to `/v1/models` only
when the primary path returns `404`.

## Benchmark Report

`bench` writes a JSON report with:

- TTFT p50/p95
- end-to-end latency p50/p95
- average prompt tokens/sec
- average generation tokens/sec
- success/failure counts
- observed max queue depth
- cancellation probe result
- router health and model snapshots

`probe-gpu` writes a JSON report with:

- GPU UUID
- PCI bus identity
- VRAM used and total
- GPU utilization
- GPU temperature
- active compute-process bindings when available

The default output path is a temp file. For the web mission-control view, point it at:

```bash
go run . bench \
  -url http://127.0.0.1:8080 \
  -model qwen3.5-27b \
  -output ~/.ironclaw/benchmarks/latest.json
```

## Comparison Guidance

For this mixed-GPU WSL workstation:

- keep the last known-good benchmark artifact separate from experimental comparison runs
- use the same benchmark contract for Ollama comparisons
- pair benchmark artifacts with a `probe-gpu` snapshot when validating which GPU actually
  handled the load

## Current Post-Restart Proof

After the WSL restart on this workstation, the smallest stable proof lane is:

- `qwen3.5-9b` via the local Ollama service
- `llm-cluster-router` in front of that lane on `127.0.0.1:8080`
- a temporary local forwarder from `127.0.0.1:8002` to `127.0.0.1:11434`
- paired artifacts:
  - `target/mission-control-benchmark.json`
  - `target/mission-control-gpu-probe.json`

The direct Ollama comparison artifact currently lives at:

- `target/mission-control-benchmark-ollama-direct.json`