# Real-model tool-discovery benchmark

This runner exercises the shipping `ironclaw serve` binary and configured live
model against deterministic 100-, 500-, and 1,000-tool catalogs. Twenty local
MCP namespaces provide read-only synthetic side effects and record the exact
tools invoked. IronClaw still performs normal authorization, approval, hooks,
safety, and MCP dispatch.

```bash
cargo build -p ironclaw
python3 scripts/tool_discovery_benchmark/run_benchmark.py \
  --output-dir /tmp/ironclaw-tool-discovery-benchmark
```

The default matrix runs all five disclosure arms, all three catalog sizes, all
required scenario classes, and four repetitions (one cold plus three warm).
Use repeated `--arm`, `--tool-count`, or `--task` flags only for diagnosis.

The output contains per-observation JSONL, aggregate JSON, model traces, browser
diagnostics, and server logs. Provider token/cache fields are retained only
when the provider reports them. Zero or unavailable usage must not be replaced
with estimates derived from JSON bytes.
