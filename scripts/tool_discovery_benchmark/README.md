# Real-model tool-discovery benchmark

This runner exercises the shipping `ironclaw serve` binary and configured live
model against deterministic 100-, 500-, and 1,000-tool catalogs. Twenty local
MCP integrations use stable semantic identities such as `github`, `gmail`, and
`google-calendar`; distractors are distributed as evenly as the fixed relevance
corpus permits. They provide read-only synthetic side effects and record the
exact tools and arguments invoked. IronClaw still performs normal authorization,
approval, hooks, safety, and MCP dispatch.

```bash
cargo build -p ironclaw
# Requires NEARAI_API_KEY or LIVE_OPENAI_COMPATIBLE_API_KEY.
export NEARAI_API_KEY=...
python3 scripts/tool_discovery_benchmark/run_benchmark.py \
  --output-dir /tmp/ironclaw-tool-discovery-benchmark
```

The default matrix runs all five disclosure arms, all three catalog sizes, all
required scenario classes, and four repetitions (one cold plus three warm).
Use repeated `--arm`, `--tool-count`, or `--task` flags only for diagnosis.

Each observation is appended and synced as soon as it completes, and an
interrupted run resumes by stable observation id. Scoring checks required call
order and arguments, detects forbidden attempts in model traces, and measures
latency to the first correct tool rather than the first tool of any kind.

The output contains per-observation JSONL, aggregate JSON, model traces, browser
diagnostics, and server logs. Provider token/cache fields are retained only when
the provider reports them. Zero or unavailable usage must not be replaced with
estimates derived from JSON bytes.
