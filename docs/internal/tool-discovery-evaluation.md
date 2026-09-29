# Tool discovery evaluation contract

This document defines the evidence required by issue #7405 before changing
IronClaw's progressive tool-discovery interaction. Retrieval quality and
end-to-end model behavior are separate measurements; neither substitutes for
the other.

## Retrieval baseline

The crate-owned retrieval gate is
`tool_search::tests::committed_corpus_quality_gate_and_benchmark_report` in
`ironclaw_loop_host`. Its committed corpus contains 50 tools and 72 judged
intents spanning exact names, aliases, canonical IDs, provider names,
parameters, nested schemas, ambiguous queries, hard negatives, and no-match
queries.

`committed_scale_baseline_covers_100_500_and_1000_tools` retains all judged
tools and intents, then adds deterministic distractors across 20 synthetic
namespaces. The generator is seeded, distributes namespace membership evenly,
and builds the catalog twice to prove byte-equivalent definitions. The
committed baseline stores deterministic quality metrics only. Index-build and
query timings are printed for diagnosis but are not committed as gates because
they vary by host and build profile.

| Tools | Recall@1 | Recall@5 | Recall@10 | MRR | NDCG@10 | No-match |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.7865 | 0.9375 | 0.9661 | 0.9492 | 0.9426 | 1.0000 |
| 500 | 0.7865 | 0.9375 | 0.9557 | 0.9492 | 0.9404 | 1.0000 |
| 1,000 | 0.7865 | 0.9375 | 0.9557 | 0.9492 | 0.9404 | 1.0000 |

The synthetic additions are intentionally unjudged distractors. They test
ranking stability and expose index cost as the catalog grows; they do not
claim to represent 950 additional human-judged user intents. New semantic
domains require new judged tools and intents in the base corpus.

Run the retrieval evidence with:

```bash
cargo test -p ironclaw_loop_host committed_corpus_quality_gate_and_benchmark_report -- --nocapture
cargo test -p ironclaw_loop_host committed_scale_baseline_covers_100_500_and_1000_tools -- --nocapture
```

## End-to-end benchmark arms

Run the same task set and catalog seed for every arm:

1. Full advertised schemas.
2. Current `tool_search` → `tool_describe` → invocation protocol.
3. Bounded complete signatures returned from `tool_search`.
4. Namespace summaries plus bounded complete signatures.
5. Namespace summaries, bounded complete signatures, and reviewed profile
   pins.

All five arms are selectable from the same binary:

| Arm | `REBORN_TOOL_DISCLOSURE` |
| --- | --- |
| Full advertised schemas | `off` |
| Current compact search/describe/call | `compact` |
| Bounded complete signatures | `signatures` |
| Namespace summaries + signatures | `namespaces` (default) |
| Namespace summaries + signatures + pins | `bridged` (opt-in) |

Unknown values fail closed to `off`. A benchmark runner should restart the
service between arms, keep the model route and catalog seed fixed, and capture
the run's selected value with every observation.

Profile pins are supplied as canonical capability IDs in a JSON object keyed by
capability-surface profile. The initial reviewed benchmark map is:

```bash
REBORN_TOOL_DISCLOSURE_PROFILE_PINS='{"interactive_tools":["gmail.list_messages","google-calendar.list_events","github.search_code"],"mission_tools":["github.search_issues_pull_requests","github.get_file_content"],"subagent_tools":["github.search_issues_pull_requests","github.get_file_content"]}'
```

Invalid JSON or any invalid profile/capability ID rejects runtime startup with
the parse cause retained. An unset variable remains the empty pin map. A pin
absent from the effective authorized surface has no effect.

The 100-, 500-, and 1,000-tool catalogs must preserve the same judged tasks.
Each size may add deterministic distractors, but the report must record the
generator version and seed.

## End-to-end report schema

Each observation records one arm, catalog size, model route, temperature,
cold/warm class, and repetition. Aggregate reports must retain the underlying
per-task observations so a broad score cannot hide a failed capability.

```json
{
  "schema_version": 5,
  "catalog": {
    "generator_version": "tool-search-scale-v2",
    "seed": 7405,
    "tool_count": 500,
    "namespace_count": 20
  },
  "arm": "prefetch-semantic",
  "config": {
    "disclosure": "namespaces",
    "prefetch": "semantic",
    "retrieval": "hybrid",
    "prefetch_always": [],
    "prefetch_tuning": {"REBORN_TOOL_PREFETCH_MAX_TOOLS": null},
    "embeddings": {
      "provider": "openai_compatible",
      "model": "BAAI/bge-small-en-v1.5",
      "base_url": "http://127.0.0.1:8080",
      "dimension": 384,
      "api_key_env": null
    }
  },
  "model": {
    "provider": "provider-id",
    "model": "model-id",
    "temperature": 0.0
  },
  "run": {
    "thermal_class": "warm",
    "repetition": 1
  },
  "task": {
    "id": "email-to-calendar",
    "completed": true,
    "correct_tool_recalled": true,
    "unauthorized_tool_leaks": 0
  },
  "counts": {
    "model_turns": 3,
    "discovery_turns": 1,
    "tool_calls": 3,
    "tool_search_calls": 1,
    "tool_describe_calls": 0,
    "result_read_calls": 0,
    "bridged_tool_calls": 0
  },
  "tokens": {
    "input": 12000,
    "cached_input": 8000,
    "output": 600
  },
  "latency_ms": {
    "time_to_first_correct_tool_call": 900,
    "end_to_end": 2400
  },
  "cache": {
    "tool_definition_signature_changes": 0,
    "tool_bearing_model_requests": 3
  },
  "advertised": {
    "tool_count_per_request": [12, 12, 12],
    "schema_tokens_per_request": [1900, 1900, 1900],
    "schema_token_estimator": "compact JSON chars / 4"
  },
  "selection": {
    "turn0_tool_count": 12,
    "turn0_tools": ["tool_search", "..."],
    "used_tools": ["gmail__search_messages", "google_calendar__create_event"],
    "hit_rate": 0.5,
    "misses": 1,
    "missed_tools": ["google_calendar__create_event"]
  },
  "turns": null,
  "idle_gap": null,
  "failure": null
}
```

A multi-turn task (the two-turn topic-drift task) fills `turns` with one
entry per user message, numbered from 0. The second entry of a drift
observation looks like this:

```json
{
  "turn": 1,
  "started": true,
  "replied": true,
  "completed": true,
  "correct_tool_recalled": true,
  "expected_tools": ["google_calendar__create_event"],
  "called_tools": ["google_calendar__create_event"],
  "tool_calls": 2,
  "synthetic_tool_calls": 1,
  "tool_search_calls": 1,
  "bridged_tool_calls": 1,
  "selection": {
    "turn_start_tool_count": 12,
    "used_tools": ["google_calendar__create_event"],
    "hit_rate": 0.0,
    "misses": 1,
    "missed_tools": ["google_calendar__create_event"]
  },
  "time_to_first_correct_tool_call_ms": 1500,
  "tool_bearing_model_requests": 3,
  "tool_definition_signature_changes": 0,
  "tools_changed_at_turn_start": false
}
```

`idle_gap` is set only by the opt-in idle-gap variant of that task:
`{"seconds": 8.0, "server_env": {...}}`, the gap left between the turns and the
shortened tool-selection cache settings its server ran with.

`arm` names a row of the runner's arm table
(`scripts/tool_discovery_benchmark/README.md`); `config` records the settings
it ran with: the `REBORN_TOOL_DISCLOSURE`, `REBORN_TOOL_PREFETCH` and
`REBORN_TOOL_RETRIEVAL` values, the always-on selection extras, the selection
tuning variables, and, for a dense or hybrid ranker, the embeddings provider,
model, endpoint and dimension (never the key). For the five disclosure arms
`arm` equals the `REBORN_TOOL_DISCLOSURE` value.
`advertised` gives each tool-bearing request's tool count and estimated schema
tokens (compact JSON characters / 4, an estimate rather than a tokenizer
count). `selection` compares the first request's tools (the turn-0 selection)
with the tools the task called: `hit_rate` is the share of used tools that
were selected, and `misses` counts calls to tools outside the selection. The
runner README defines both precisely.
`cache.tool_definition_signature_changes` counts how often the SHA-256 of the
request's `tools` array changes between consecutive tool-bearing model requests
in one observation; the runner reads it from a loopback relay in front of the
model endpoint. It is `null` when the relay is off or saw no tool-bearing
request; it is never estimated.
Each `turns` entry measures one user message on its own: its tool calls, its
`tool_search` and bridged `tool_call` counts, its selection misses against
the tools advertised at the start of that turn, the time from the turn's
start to its first correct tool, signature changes within the turn, and
whether the tools array changed at the turn boundary
(`tools_changed_at_turn_start`). Without an idle gap the whole conversation's
`cache.tool_definition_signature_changes` must be 0. The summary's
`turn_aggregates` aggregates these per arm, catalog size, task and turn.

`failure`, when present, uses a stable category such as `retrieval_miss`,
`invalid_arguments`, `authorization_denied`, `approval_blocked`,
`provider_error`, or `task_incomplete`. The local synthetic fixture validates
task-owned argument fields, but raw prompts, user content, credentials, and tool
arguments are not retained in aggregate benchmark observations.

## Required scenarios

- Exact tool name and canonical capability ID.
- Alias and natural-language action queries.
- Ambiguous queries with multiple relevant tools.
- Argument-only vocabulary found in nested schemas.
- Relevant denied tools mixed with allowed distractors.
- Cross-namespace workflows, including finding an email and creating a
  calendar event.
- Topic drift: a second user message in the same conversation that needs a
  namespace the first message did not mention.
- No-match tasks where the correct behavior is to report that no authorized
  capability exists.

Every model/provider configuration runs at least one cold repetition and three
warm repetitions. Reports include median, worst case, spread, and
failure-category counts; cache-provider measurements additionally report
cached-input tokens and tool-definition signature changes. Missing provider
cache measurements remain explicitly `null`.

Deterministic repository tests gate catalog construction, retrieval quality,
protocol shape, authorization fitting, namespace fairness, and stable
serialization. Provider token usage and network/model latency are intentionally
not estimated from JSON bytes or local test timings; those fields are populated
only by the deployed cold/warm runner. This separation prevents a deterministic
CI proxy from being presented as end-to-end model evidence.

## Rollout gates

- Zero unauthorized namespace, signature, provider-reference, ranking, or
  callable-target leakage.
- Existing retrieval recall, MRR, NDCG, and no-match gates remain satisfied.
- Complete-signature tasks reduce discovery turns without increasing invalid
  calls attributable to missing schemas.
- Task completion does not materially regress at any catalog size.
- Providers with stable deferred loading retain one byte-identical advertised
  tool surface throughout discovery and invocation.

Bounded orchestration is not an arm until the preceding interaction changes
have shipped and this report shows that model round trips remain a dominant
latency source. It requires a separate design and issue.
