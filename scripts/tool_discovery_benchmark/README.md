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

The default matrix runs every arm below except the opt-in `prefetch-jev`, all
three catalog sizes, all required scenario classes except the opt-in
`topic-drift-idle` task, and four repetitions (one cold plus three warm). Use
repeated `--arm`, `--tool-count`, or `--task` flags to run a subset;
`prefetch-jev` runs only when named with `--arm`, and `topic-drift-idle` only
when named with `--task`.

## Arms

| Arm | `REBORN_TOOL_DISCLOSURE` | `REBORN_TOOL_PREFETCH` | `REBORN_TOOL_RETRIEVAL` | `REBORN_TOOL_PREFETCH_ALWAYS` |
| --- | --- | --- | --- | --- |
| `off` | `off` | unset (`off`) | unset (`native`) | unset |
| `compact` | `compact` | unset | unset | unset |
| `signatures` | `signatures` | unset | unset | unset |
| `namespaces` | `namespaces` | unset | unset | unset |
| `bridged` | `bridged` (plus benchmark profile pins) | unset | unset | unset |
| `prefetch-lexical` | `namespaces` | `lexical` | `native` | unset: the four bridges only |
| `prefetch-lexical-floor` | `namespaces` | `lexical` | `native` | the load-bearing core tools from `.env.example` |
| `prefetch-semantic` | `namespaces` | `semantic` | `hybrid` | unset: the four bridges only |
| `prefetch-jev` (opt-in) | `namespaces` | `lexical`, plus `REBORN_TOOL_PREFETCH_CLASSIFIER=jev` | unset | unset: the four bridges only |

The variables in the header, `REBORN_TOOL_PREFETCH_CLASSIFIER` and the
`bridged` arm's `REBORN_TOOL_DISCLOSURE_PROFILE_PINS` belong to the arms: if
they are exported in your shell the runner removes them (and says so) before
starting any server. Every arm but `prefetch-jev` leaves the classifier unset,
which is `local`.
The selection tuning knobs `REBORN_TOOL_PREFETCH_MAX_TOOLS`,
`REBORN_TOOL_PREFETCH_TOKEN_BUDGET`, `REBORN_TOOL_PREFETCH_MIN_SIMILARITY` and
`REBORN_TOOL_PREFETCH_MIN_RELATIVE` pass through from your shell (unset means
the runtime default) and are recorded under `config.prefetch_tuning`.

To sweep the selection cap, repeat `--max-tools N` (for example
`--max-tools 25 --max-tools 50 --max-tools 100 --max-tools 150`). Each value
sets `REBORN_TOOL_PREFETCH_MAX_TOOLS` for the selection arms (`prefetch-*`),
overriding an exported value, and is recorded as `config.max_tools`. The
disclosure-only arms run once, since they have no selection to cap. The value
is part of the observation id (`arm:tools:max_tools=N:task:repetition`) and of
the case directory name, so every value of a sweep resumes separately and can
share one output directory. The summary's aggregates and turn aggregates are
grouped by arm, catalog size and `max_tools` (`null` is the runtime default).
Without the flag, ids and case names are unchanged and the runtime default
applies.

`prefetch-semantic` ranks with embeddings, so it needs the `EMBEDDING_*`
variables. If they are missing or incomplete, the run stops before any server
starts and names what is missing; it never falls back to lexical selection.
Each observation's `config.embeddings` records the provider, model, endpoint
(with any credentials in the URL removed) and dimension. The API key is never
recorded.

### A local embeddings endpoint

Hugging Face `text-embeddings-inference` on CPU, serving
`BAAI/bge-small-en-v1.5` (384 dimensions), gives a reproducible local
endpoint:

```bash
docker run --rm -p 127.0.0.1:8080:80 -v "$HOME/.cache/tei:/data" \
  ghcr.io/huggingface/text-embeddings-inference:cpu-1.8 \
  --model-id BAAI/bge-small-en-v1.5 --max-client-batch-size 64 --auto-truncate
```

`--max-client-batch-size 64` matches IronClaw's default embeddings batch size
(`EMBEDDING_MAX_BATCH_SIZE`, 64); the server's own default of 32 would reject
the larger batches. `--auto-truncate` truncates a long tool description to the
model's 512-token input limit instead of failing the request. Record the image
tag you ran with the results.

Point the arm at it:

```bash
export EMBEDDING_PROVIDER=openai_compatible
export EMBEDDING_BASE_URL=http://127.0.0.1:8080   # /v1 is appended once
export EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
export EMBEDDING_DIMENSION=384
# No key is needed for a local endpoint.
```

Check it answers before a long run:

```bash
curl -s http://127.0.0.1:8080/v1/embeddings -H 'content-type: application/json' \
  -d '{"model":"BAAI/bge-small-en-v1.5","input":["hello"]}' | head -c 200
```

### The Jev arm

`prefetch-jev` chooses the turn-0 tools with Jev, TypeSafe's hosted
classifier, instead of the local ranker. It calls TypeSafe's decisions API by
default, or any provider serving the same decisions API. It is opt-in because
it needs a paid key and because **it sends data to the configured Jev
provider, a third party**: for every task conversation, the opening message
(up to 16 KiB) and every candidate tool's name, description and parameter
names. The catalog is the benchmark's synthetic one, but the task prompts go
too. See the confidentiality note for `REBORN_TOOL_PREFETCH_CLASSIFIER` in
`.env.example`.

| Variable | Set by | Meaning |
| --- | --- | --- |
| `REBORN_TOOL_PREFETCH_CLASSIFIER` | the arm (`jev`) | which classifier chooses the tools |
| `TYPESAFE_API_KEY` | you | the provider's key (in the variable `REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV` names, when set); the server reads it host-side |
| `REBORN_TOOL_PREFETCH_JEV_ENDPOINT` | you, optionally | the decisions endpoint (default `https://api.typesafe.ai/v1/systemone`), passed to the server |
| `REBORN_TOOL_PREFETCH_JEV_MODEL` | you, optionally | the model (default `jev-latest`), passed to the server |
| `REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV` | you, optionally | the NAME of the key's variable (default `TYPESAFE_API_KEY`), passed to the server |
| `TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS` | you, optionally | the price recorded and used for cost (default 0.042) |
| `IRONCLAW_REBORN_LOG` | you, optionally | the server's stderr log filter; the selection arms add `ironclaw::reborn::tool_prefetch=debug` to it (default `info`) |

If the key variable is unset or blank, the run stops before any server starts
and names the variable; the arm never falls back to the local classifier. The
generated Reborn home has no `[tool_selection.jev]` table, so the server runs
the runtime defaults unless you set the overrides above: endpoint
`https://api.typesafe.ai/v1/systemone`, model `jev-latest`, key variable
`TYPESAFE_API_KEY`, and a 500 ms timeout for one classification. The arm sets
the model and key variable on the server explicitly, so what is recorded is
what ran. Each observation's `config.classifier` is `jev`, and `config.jev`
records the endpoint (without userinfo or query), the requested model, the
key variable's name (never the key), the timeout, and the price used for cost
(US$0.042 per million input tokens by default). `jev-latest` is an alias
that moves between Jev releases, so the requested model does not say which
release answered: each observation also records the models the server
reported (`selection_log.jev.served_models`). The default price is TypeSafe's
published figure for `jev-1.13.0`; another provider may charge differently
and publishes its own price, which
`TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS` records instead. Only this
arm has `config.classifier` and `config.jev`; their absence means the local
classifier. A unit test keeps these defaults and override names in step with
`crates/app/ironclaw_config/src/tool_prefetch.rs`.

To run against another provider serving the same decisions API:

```bash
export OTHER_JEV_KEY=...
export REBORN_TOOL_PREFETCH_JEV_ENDPOINT=https://decisions.example.com/api/v1/decisions
export REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV=OTHER_JEV_KEY
export REBORN_TOOL_PREFETCH_JEV_MODEL=jev-latest
export TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS=...
```

```bash
export TYPESAFE_API_KEY=...
python3 scripts/tool_discovery_benchmark/run_benchmark.py \
  --output-dir /tmp/ironclaw-tool-selection-jev \
  --arm prefetch-lexical --arm prefetch-jev
```

Then run the selection arms at all three sizes:

```bash
python3 scripts/tool_discovery_benchmark/run_benchmark.py \
  --output-dir /tmp/ironclaw-tool-selection-benchmark \
  --arm namespaces --arm prefetch-lexical --arm prefetch-lexical-floor \
  --arm prefetch-semantic
```

The selection metrics come from the request relay described below, so set
`REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL` (or `LIVE_OPENAI_COMPATIBLE_BASE_URL`
for a non-`nearai` provider); without it they are null and the runner warns.

Each observation is appended and synced as soon as it completes, and an
interrupted run resumes by stable observation id. Scoring checks required call
order and arguments, detects forbidden attempts in model traces, and measures
latency to the first correct tool rather than the first tool of any kind.

`cache.tool_definition_signature_changes` is the #6986 check: the advertised
tools array must stay byte-identical across the model calls of one run. The
recorded LLM trace does not keep request bodies, so when a model base URL is
configured (`REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL`, or
`LIVE_OPENAI_COMPATIBLE_BASE_URL` for a non-`nearai` provider) the runner puts a
loopback relay in front of it. The relay forwards requests and streamed
responses unchanged and keeps only a SHA-256 of each request's `tools` array; it
never stores headers or credentials. The metric counts how often that hash
changes between consecutive tool-bearing requests in one observation (0 is
compliant; null means no tool-bearing request was seen or the relay was off).
`--no-request-recorder` disables the relay.

One server runs every repetition of a case, and a repetition whose turn timed
out keeps calling the model while the next one runs. The relay therefore also
records which conversation each request belongs to, read from the completion
marker in its user messages (`BENCHMARK_DONE_<case>_<repetition>`, without the
`_T<n>` turn suffix). Every relay metric of an observation counts only its own
conversation's requests; before this, a leftover conversation's requests were
counted as changes of the next repetition's tools array. The fixture's tool
calls and the trace metrics are not split by conversation yet, so a timed-out
repetition can still inflate the next repetition's call counts.

## Scoring

A task with expected tools completes when those tools reached the fixture in
order with valid arguments; other calls in between are allowed, so reading a
calendar before creating an event still counts. A no-tool task (`no-match`,
`denied-capability`) completes when nothing reached the fixture, no forbidden
capability was attempted, and every model tool call was discovery:
`tool_search`, `tool_describe`, `capability_info`, or a `result_read` of one
of their results.

The `result_read` rule exists because a long result comes back to the model as
a reference, which it pages through with `builtin__result_read`; a search
followed by a read is still only a search. Each read (direct, or as the
`tool_call` bridge's target in either spelling) is resolved from the trace:
the read's `result_ref` argument names the reference, and the call whose
recorded result carries that `detail.result_ref` is its source, named by its
target when it went through the bridge. A read of a real tool's result is a
real call. The trace does not always record a step's tool results, so some
reads cannot be resolved; an unresolved read counts as discovery. That can
only let a no-tool task pass on a read whose source was a real tool, and
such a task has already failed on the real call itself. The reads still cost
a round trip, so `counts.result_read_calls` keeps counting all of them, and
`counts.discovery_result_read_calls` counts those scored as discovery.

The fixture answers every tool call with `benchmark tool <name> completed`,
except for the tools whose answer a task's reply has to use:
`gmail__search_messages` returns the meeting details, `github__list_issues`
two issues and `google_calendar__create_event` an event id
(`FIXTURE_RESULTS` in `run_benchmark.py`). The catalog comes from the shared
relevance corpus, with one repair: a `oneOf` whose object branches require
nothing matches every object in each branch, so it rejects every object.
Each such branch is made to require its own properties
(`exclusive_one_of`), which is what lets `google_calendar__create_event`
accept a correct `schedule`. That repair is catalog generator version
`tool-search-scale-v3`; do not resume a v2 output directory with it.

## Advertised tools and turn-0 selection

The relay also keeps, for each model request, the names in its `tools` array,
how many there are, and an estimate of the tokens they add. The estimate is
the length of the `tools` array serialized as compact JSON, divided by 4 and
rounded up: a rough characters-per-token rule, not a tokenizer count, so use it
to compare arms rather than to predict a bill. Each observation records:

- `advertised.tool_count_per_request` and
  `advertised.schema_tokens_per_request`: one value per tool-bearing model
  request, in order.
- `selection.turn0_tools` and `selection.turn0_tool_count`: the turn-0
  selection, meaning the `tools` array of the observation's first tool-bearing
  request. With turn-start selection on, this list stays fixed for the
  conversation.
- `selection.used_tools`: the distinct tools the model called, directly or as
  the target of a `tool_call`, leaving out `tool_search`, `tool_describe`,
  `tool_call`, `result_read` and `capability_info`. Spellings are folded
  together: a dotted capability id, its `__` provider form and the bare MCP
  tool name count as one tool.
- `selection.hit_rate`: the share of `used_tools` that were in the turn-0
  selection. It is null when the task used no tool (the no-match and denied
  tasks) or the relay was off.
- `selection.misses`: the number of calls, all after turn 0, to a used tool
  that was not in the turn-0 selection; such a tool is reachable only through
  `tool_search` and the `tool_call` bridge. `selection.missed_tools` lists them.
  A call to a tool name that does not exist also counts as a miss.

These are computed for every arm. For the arms without turn-start selection,
the "selection" is simply whatever the first request advertised.

`summary.json` aggregates each arm and catalog size: completion rate, latency,
leaks and failure categories, plus model turns, `tool_search`/`tool_describe`/
`result_read`/bridged `tool_call` counts (and
`discovery_result_read_calls_total`), input and cached-input tokens,
time to the first correct tool, and tool-definition signature changes (the
multi-turn tasks' per-turn figures are in `turn_aggregates`, described
below). Token
aggregates use only observations where the provider reported usage, and
`token_usage_observations` says how many that was. Each aggregate also carries
the arm's `config` (and `config_consistent`, false if a resumed run changed it),
the 20th percentile / median / 80th percentile of advertised tool count,
advertised schema tokens and turn-0 selection size (`*_p20_median_p80`, from
`statistics.quantiles(values, n=5, method="inclusive")`, as in the baseline
report), the mean hit rate, how many observations hit every used tool, and the
total misses. `tool_definition_signature_changes_total` must be 0 for every
arm: with turn-start selection on, the advertised list must still never
change within a conversation. The one allowed exception is the opt-in
idle-gap task described below, once the runtime re-selects after a cold
cache; leave it out of a run whose totals must be 0.

## Selection latency, fallbacks and Jev usage

Some facts about turn-0 selection exist only in the server's debug log, on the
`ironclaw::reborn::tool_prefetch` target: none of those lines carries user
text or keys. For every arm with turn-start selection, the runner raises that
target to `debug` through `IRONCLAW_REBORN_LOG`, notes where the server's
stderr log (`cases/<group>/ironclaw-reborn-serve.stderr.log`) ends before each
repetition, and parses what the repetition appended. Each repetition is one
conversation, so it holds one first selection. The observation's
`selection_log` (null for arms without selection) records:

- `turn0_latency_ms`: how long the classifier call for the first selection
  took, as the loop host measured it, including a failed Jev call before the
  core-set fallback.
- `classifier`: `local` or `jev`.
- `core_set_fallbacks` and `fallback_reasons`: whether the first selection fell
  back to the core tool set because the classifier failed, and the failure
  kind (`timeout`, `rate_limited`, `unauthorized`, and so on). Only a bound
  classifier such as Jev falls back; a failing local ranker keeps the ordinary
  surface and logs no selection.
- `jev` (null unless a Jev line was seen): the `model` requested, the
  `served_models` the server reported answering with (empty when every
  classification failed), the number of `classifications` and their
  `failures`, `slices` (how many requests the catalog was split into, summed
  over the conversation), `input_tokens` as the provider reported them, `cost_usd` (input tokens times
  the recorded price), and each classification's `latency_ms`.
- `index` (null unless the ranker has vectors, that is `dense` or `hybrid`):
  how the turn-0 fit came by its tool vectors. Of the corpus `documents`,
  `stored` already had a vector (in memory, or `loaded` from the per-user
  durable store), `embedded` were embedded while the turn waited, and
  `missing` had none when the fit returned; `dense_fallback` is the hybrid
  ranker's reason for ranking lexically (`timeout`, an error kind, or null
  when the dense side ran). With vectors persisted and catalogs indexed when
  they change, a steady-state conversation shows `embedded` and `missing` at
  zero.

The summary adds, per arm and catalog size, `selection_log_observations`, the
turn-0 selection latency (`turn0_selection_latency_ms_p20_median_p80`, `_max`),
`core_set_fallback_observations`, `core_set_fallback_rate` and
`core_set_fallback_reasons`, and for Jev `jev_models` (the served models,
not the requested alias), slices per conversation
(`jev_slices_per_conversation_p20_median_p80`, `_max`), `jev_input_tokens_total`
and `_mean`, and `jev_cost_usd_total` and `jev_cost_usd_per_conversation_mean`,
and for dense or hybrid rankers `index_observations`,
`index_embedded_at_selection_total`, `index_missing_at_selection_total`,
`index_dense_fallback_rate` and `index_dense_fallback_reasons`.
The hit rate and misses come from the request relay, as for the other
selection arms; after a core-set fallback they measure the core set.

What cannot be collected: a failed Jev classification logs no token count, so
tokens the provider may have billed for a failed call are missing from
`input_tokens` and `cost_usd` (both null when every classification failed).
The cost counts input tokens only, at the price recorded in `config.jev`. If
the log line formats change, these fields come back null or zero rather than
wrong; a unit test pins the parsed messages and field names to the Rust
sources.

The observation and summary schema is version 7 (version 7 made the Jev
arm's provider configurable and records its endpoint). The runner refuses to resume
an output directory written by an older schema; start a new directory.

## Two-turn topic drift

A selection made from the opening message is only tested when the
conversation later needs tools that message did not predict. The
`topic-drift` task holds a two-message conversation: the first message asks
for the open issues of a GitHub repository (`github__list_issues`); the
second, sent in the same conversation after the first reply, asks for a
Google Calendar event (`google_calendar__create_event`). The first message
never mentions a calendar, so a turn-start selection has no reason to include
one. The second message is sent with the live-QA helper's scripted follow-up
(`ScriptedFollowUp` and `scripted_follow_ups` on `_live_chat_case`), and each
reply is awaited with its own marker. The task runs in every arm.

The task is scored as a whole (both tools, in order, with valid arguments) and
each turn is also measured on its own. At each turn boundary the runner
snapshots how many fixture calls, relay requests and trace tool calls the
observation has, and the observation's `turns` list reports, per turn
(numbered from 0, so the drifted second message is turn 1):

- `started` and `replied`: whether the message was sent and its reply
  arrived. `correct_tool_recalled`: whether the turn's expected tool was
  called; `completed`: whether it was called with valid arguments.
- `tool_calls` (model tool calls in the trace), `synthetic_tool_calls`
  (calls that reached the fixture), `tool_search_calls` and
  `bridged_tool_calls`.
- `selection`: as the observation's `selection`, but measured against the
  tools advertised at the start of that turn (`turn_start_tool_count`).
  `selection.misses` counts calls to tools that were not advertised, which
  only `tool_search` and the `tool_call` bridge can reach. Without
  re-selection turn 1 starts with the turn-0 selection, so a missed Calendar
  tool shows up here.
- `time_to_first_correct_tool_call_ms`, measured from the turn's own start.
- `tool_bearing_model_requests` and `tool_definition_signature_changes`
  within the turn, and `tools_changed_at_turn_start`: whether the turn's
  first tools array differs from the last one before it.

Without an idle gap the advertised tools must not change anywhere in the
conversation, so the observation's `cache.tool_definition_signature_changes`
(which spans both turns and the boundary between them) must be 0 and
`tools_changed_at_turn_start` must be false. Metrics that need the relay are
null when it is off; trace-based ones are null for a turn whose boundary the
runner could not read from the trace. Single-turn tasks have `turns: null`.

`summary.json` has a `turn_aggregates` list with one entry per arm, catalog
size and multi-turn task: the conversation-wide signature-change total, and
per turn the started and replied counts, completion rate, tool-call,
`tool_search` and bridged `tool_call` totals, selection misses, the median
and worst time to the first correct tool, within-turn signature changes, and
how many observations changed the tools at the turn start.

### Idle-gap variant

`topic-drift-idle` is the same conversation with an 8-second idle gap between
the turns. Its server runs with a shortened tool-selection cache lifetime:

| Variable | Value |
| --- | --- |
| `REBORN_TOOL_PREFETCH_RESELECT` | `on` |
| `REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS` | `5` |
| `REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS` | `1` |

Lifetime plus margin is 6 seconds, so the gap puts the conversation past it
without waiting real minutes. Its observations record `idle_gap` (the
seconds and those variables), and its end-to-end latency leaves the gap out.
These three variables belong to the tasks: if you export them the runner
removes them, like the arm settings, so every other task runs with the
runtime defaults.

The variant only means something once the runtime re-selects the advertised
tools after the prompt cache has gone cold. Until then those variables do
nothing, and the second turn behaves exactly as in `topic-drift`, which is
why the task is opt-in: run it with `--task topic-drift-idle`. Once
re-selection exists, a re-selection shows up as
`tools_changed_at_turn_start: true` on turn 1, a conversation-wide signature
change of 1, and (if the new selection includes Calendar) no turn-1 misses.
The configured lifetime applies to providers whose cache lifetime the host
cannot know; a provider with a known lifetime (Anthropic's cache retention)
uses its own, so run the variant against another provider.

Runs on catalog generator `tool-search-scale-v2` completed no `topic-drift`
observation, for two fixture reasons rather than model behaviour. First, the
fixture answered `github__list_issues` with only a completion line, so the
model went looking for real issues (the GitHub API, then installing a GitHub
extension) until an authentication prompt or the turn timeout stopped it, and
turn 1 was never sent. Second, the `create_event` schema rejected every
correct `schedule` (see "Scoring"). Both are fixed; a re-score cannot recover
those observations, because the model's calls failed at the time.

Before this task, the cross-namespace workflow task (Gmail then Calendar in
one message) was the only measure of a partial selection; it still shows a
selection that picked only one of the two namespaces as a miss.

## Re-scoring a finished run

```bash
python3 scripts/tool_discovery_benchmark/run_benchmark.py \
  --rescore /tmp/ironclaw-tool-discovery-benchmark \
  --output-dir /tmp/ironclaw-tool-discovery-rescored
```

`--rescore RUN_DIR` recomputes scoring and `summary.json` from
`RUN_DIR/observations.jsonl` and `RUN_DIR/llm-traces/` without starting a
server or calling a model, and needs no API key. It only reads `RUN_DIR`; the
corrected `observations.jsonl` and `summary.json` go to `--output-dir`,
which must be a different directory without an `observations.jsonl`. It reads
observations of schema 5 or newer (`RESCORE_MIN_SCHEMA_VERSION`) and leaves
each one's `schema_version` as it was, so a run stays re-scorable after a
schema bump that a resume would refuse.

All repetitions of one arm, catalog size and task share a trace file, so the
trace is cut back into observations in run order by each one's stored
`counts.tool_calls`. A slice is trusted only if it reproduces the stored
`tool_search_calls`, `result_read_calls` and `bridged_tool_calls` and the
counts add up to the whole trace; a group that fails this keeps its stored
scoring and is listed, with the reason, in the summary's
`rescore.unverified_groups`. `rescore.completion_changes` lists every
observation whose `completed` changed.

What a re-score recomputes, and what it cannot:

- No-tool tasks: `task` scoring and `failure` are recomputed from the trace
  and the stored `task.called_tools`.
- Tasks with expected tools keep their stored `task`. Their argument checks
  need the arguments of the calls that reached the fixture, which an
  observation does not store, and the result_read rule does not affect them.
- Every re-scored observation gains `counts.discovery_result_read_calls`.
- Everything that depends on a run's live state is kept as stored: latency,
  tokens, relay and selection metrics, per-turn `turns`, and `ui_probe_success`.
  A failure caused by the fixture or catalog (such as the topic-drift causes
  above) cannot be re-scored away; it needs a new run.

## Output

The output contains per-observation JSONL, aggregate JSON, model traces, browser
diagnostics, and server logs. Provider token/cache fields are retained only when
the provider reports them. Zero or unavailable usage must not be replaced with
estimates derived from JSON bytes.
