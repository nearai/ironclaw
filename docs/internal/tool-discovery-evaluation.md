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
  "schema_version": 2,
  "catalog": {
    "generator_version": "tool-search-scale-v2",
    "seed": 7405,
    "tool_count": 500,
    "namespace_count": 20
  },
  "arm": "signatures",
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
    "tool_describe_calls": 0
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
    "tool_definition_signature_changes": null
  },
  "failure": null
}
```

`arm` is always the exact canonical `REBORN_TOOL_DISCLOSURE` selector value.
`cache.tool_definition_signature_changes` is `null` when the provider trace
does not expose a trustworthy signature-change count; it is never estimated.

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
