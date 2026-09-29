# Tool-RAG baseline benchmark (before any change)

> **Status: complete.** Full default matrix, run on 2026-09-27 from about 16:05 to
> 21:54 BST (about 6 hours), 420 of 420 observations. The raw results are
> archived next to this page in [`tool-rag-baseline/`](tool-rag-baseline/).

This is the "before" measurement for the embedded tool-RAG work. It
measures today's progressive tool-disclosure arms with the real `ironclaw serve`
binary and a real model, so the later turn-0 selection arms can be compared
against the same tasks, catalogs and model.

Background on the harness, the arms and the upstream retrieval-only results is
in [`tool-discovery-evaluation.md`](../tool-discovery-evaluation.md). The
harness itself is `scripts/tool_discovery_benchmark/run_benchmark.py` (see its
`README.md`).

## Summary

> **The cross-namespace workflow task is excluded from every figure on this
> page except the [Failures](#failures) table.** It failed all 60 of its runs,
> in every arm and at every catalog size, including `off`, where the model sees
> every schema. So it measures a model or task problem, not tool discovery.
> Its runs also looped until the harness's 180 s time limit, which distorted
> every timing and token figure they touched. Each cell below therefore covers
> 6 task classes × 4 repetitions = 24 observations, not 28. The archived
> `summary.json` is the harness's own summary and still includes that task.

- **Discovery gets slower as the catalog grows.** Under the shipping default
  (`namespaces`), the median time to the first correct tool call is 6.6 s at
  100 tools, 9.4 s at 500 and 10.4 s at 1,000.
- **Pre-advertising the right tools removes most of that.** `bridged` pins
  three of the tools the tasks need, and its median stays at 6.4–6.9 s at
  every size. At 1,000 tools that is 3.9 s (38%) faster than the default.
- **The difference is one search round trip.** The disclosure arms' median is
  one `tool_search` call per task (1.5 for `namespaces` at 100 tools). Removing it is what saves the 2.5–3.9 s.
- **Advertising everything does not scale.** `off` is fastest at 100 tools,
  but its median input grows from 76k tokens per task at 100 tools to 442k at
  1,000. At 1,000 tools it is also the slowest arm end to end (19.8 s median,
  against 12.0 s for `namespaces`).
- **Completion does not separate the arms.** Every cell completes 22–24 of 24
  tasks.
- **Upstream's #7410 is visible.** With complete signatures in `tool_search`
  results, the median `tool_describe` count drops from 1 per task (`compact`)
  to 0 (`signatures`, `namespaces`).
- **No unauthorized tool calls in any run.**

## Prognosis for turn-0 tool selection

Turn-0 selection advertises the tools predicted from the opening message, so a
correct prediction removes the `tool_search` round trip entirely. `bridged` is
the nearest thing this baseline has to that: a hand-picked, partial selection
of three pinned tools, which only some tasks use. So it gives a realistic
target, not a ceiling.

Medians, in seconds. The headroom is computed from the unrounded medians, so
it can differ by 0.1 s from subtracting the rounded values shown:

| Catalog | `namespaces` first correct tool | `bridged` first correct tool | Headroom | `namespaces` end to end | `bridged` end to end |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 6.6 | 6.5 | 0.1 (2%) | 8.7 | 10.2 |
| 500 | 9.4 | 6.9 | 2.5 (27%) | 12.5 | 10.0 |
| 1,000 | 10.4 | 6.4 | 3.9 (38%) | 12.0 | 10.9 |

What to expect, and what to watch:

1. **The gain is at 500 and 1,000 tools.** At 100 tools the default already
   matches the pinned arm, so a selection arm will not show a gain there. The
   case for upstream rests on the larger catalogs.
2. **Aim for about 6.5 s to the first correct tool at every size**, with a
   median of 0 `tool_search` calls per task. The time to the first correct
   tool is the clearest signal. End-to-end medians move less (1.1–2.5 s at 500
   and 1,000), because the model's own reasoning and answer dominate them.
3. **Token savings are not shown here.** `namespaces` and `bridged` both send a
   median of 54–56k input tokens per task at every size. `bridged` still
   advertises the full core tool set plus its pins, so it saves nothing on the
   prompt. Any token saving from turn-0 selection has to come from advertising
   fewer tools than the 29-tool core set, which this baseline does not
   measure. 76–92% of input tokens were already served from Venice's prompt
   cache, so the cost effect of any saving will be smaller still.
4. **Completion is unlikely to improve, and must not drop.** Pass rates are
   22–24 of 24 everywhere, so a selection arm can only show "no loss".
5. **The cache claim needs a multi-turn task.** The tools-array metric reports
   0 changes everywhere (see below), but promotion only changes the tools
   array on the *next* turn, and every task here is a single turn. So this
   baseline cannot show the promotion churn #6986 is about, and cannot show
   the selection arms avoiding it. A two-turn task, whose second turn moves to
   a different integration, is needed to make that part of the case.
6. **Watch the tail, not only the median.** The 80th percentile of end-to-end
   time is 22–113 s across the cells, because of runs that keep calling tools
   after they already have the answer (see [Failures](#failures)).

## Which numbers matter most

Upstream will judge any tool-RAG arm against two of the existing arms:

- **`namespaces`**: the shipping default. Namespace summaries plus bounded
  complete signatures from `tool_search`.
- **`signatures`**: bounded complete signatures from `tool_search`, without
  namespace summaries.

A tool-RAG arm has to beat these on model turns, discovery calls and time to
the first correct tool, without losing completion rate or leaking unauthorized
tools. `off` (every schema advertised) is the ceiling for "the model can see
everything" and the floor for prompt size. `compact` is the older
search-then-describe flow. `bridged` adds profile pins
(`REBORN_TOOL_DISCLOSURE_PROFILE_PINS`). The harness pins
`github__get_pull_request`, `google_calendar__list_events` and
`gmail__search_messages` (`run_benchmark.py`, `run_task_group`), which some
tasks need, so read it as a hand-picked selection rather than a fair
competitor.

## Run identity

| Field | Value |
| --- | --- |
| Binary commit | `0c4cf5153` (`summary.json` → `head`), a commit on the baseline branch. That branch changes only `scripts/` and `docs/`, so the binary is main's `6bc067725`. |
| Provider id | `openai_compatible` |
| Provider endpoint | `https://api.venice.ai/api/v1` |
| Model (pinned) | `deepseek-v4-flash-0731` |
| Build profile | `debug` (`cargo build -p ironclaw`, run with `--binary target/debug/ironclaw`); the MCP fixture rewrite exists only in debug builds |
| Temperature | 0.0 (recorded by the harness) |
| Catalog generator / seed | `tool-search-scale-v2` / `7405` |
| Matrix | 5 arms × 3 catalog sizes (100, 500, 1,000) × 7 tasks × 4 repetitions (1 cold + 3 warm) = 420 observations, none resumed. 360 are reported below, after excluding the cross-namespace workflow. |
| Observation / summary schema | 3 / 3 |
| Provider reports usage | Yes, for all 420 observations (`provider_usage_available: true`) |
| Provider reports cache reads | Yes: cached input tokens are non-zero in every cell |
| Date of run | 2026-09-27, about 16:05–21:54 BST |
| Host | Linux workstation; `example.com` mapped to its public address in `/etc/hosts` (see [Reproducing the run](#reproducing-the-run)) |

## Results

One table per catalog size. Each row is one arm, over its 24 observations
(6 task classes × 4 repetitions, **excluding the cross-namespace workflow**).

**Reading the triples:** a value written `a / b / c` is the **20th
percentile / median / 80th percentile** of the per-task values. The
percentiles use Python's `statistics.quantiles(values, n=5,
method="inclusive")` (linear interpolation between observations), so a
fractional value such as `4.4` turns can appear.

- **Done**: tasks whose required calls happened in order, with valid
  arguments and no unauthorized attempt (`task.completed`).
- **End to end (s)**: `latency_ms.end_to_end`, per task.
- **First correct tool (s)**: `latency_ms.time_to_first_correct_tool_call`.
  It exists only for tasks with a required tool, 4 of the 6 task classes
  (`no-match` and `denied-capability` have none), so it covers 15–16
  observations per cell.
- **Model turns**: `counts.model_turns` (model calls per task).
- **Searches / Describes**: `counts.tool_search_calls` and
  `counts.tool_describe_calls` per task.
- **Input tokens / Cached tokens**: `tokens.input` and `tokens.cached_input`
  per task. Every observation reported usage.
- **Tools-array changes**: the total of `cache.tool_definition_signature_changes`,
  counted within each task (see [What the tools-array metric shows](#what-the-tools-array-metric-shows)).
- **Leaks**: the total of `task.unauthorized_tool_leaks`; must be 0.

### 100 tools

| Arm | Done | End to end (s) | First correct tool (s) | Model turns | Searches | Describes | Input tokens | Cached tokens | Tools-array changes | Leaks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `off` | 24/24 | 6.1 / 9.7 / 22.3 | 4.1 / 4.3 / 5.3 | 1 / 2 / 4.4 | 0 / 0 / 0 | 0 / 0 / 0 | 38k / 76k / 169k | 27k / 65k / 157k | 0 | 0 |
| `compact` | 23/24 | 7.8 / 10.7 / 43.1 | 7.0 / 7.8 / 9.4 | 2 / 4 / 9.4 | 1 / 1 / 2 | 0 / 1 / 2 | 35k / 72k / 187k | 27k / 62k / 168k | 0 | 0 |
| `signatures` | 23/24 | 7.6 / 9.2 / 112.7 | 5.4 / 6.7 / 7.9 | 2 / 3 / 16.4 | 1 / 1 / 3.4 | 0 / 0 / 0 | 35k / 54k / 393k | 29k / 44k / 348k | 0 | 0 |
| **`namespaces`** (default) | 24/24 | 8.0 / 8.7 / 66.0 | 5.8 / 6.6 / 7.1 | 2 / 3 / 10.4 | 1 / 1.5 / 5.4 | 0 / 0 / 0 | 35k / 54k / 264k | 29k / 47k / 249k | 0 | 0 |
| `bridged` | 24/24 | 8.9 / 10.2 / 24.0 | 5.5 / 6.5 / 7.4 | 2 / 3 / 4.4 | 1 / 1 / 1.4 | 0 / 0 / 0 | 36k / 56k / 82k | 30k / 49k / 74k | 0 | 0 |

### 500 tools

| Arm | Done | End to end (s) | First correct tool (s) | Model turns | Searches | Describes | Input tokens | Cached tokens | Tools-array changes | Leaks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `off` | 24/24 | 7.6 / 12.3 / 26.5 | 6.0 / 7.0 / 8.0 | 1 / 2 / 4.4 | 0 / 0 / 0 | 0 / 0 / 0 | 100k / 200k / 441k | 69k / 169k / 409k | 0 | 0 |
| `compact` | 22/24 | 11.0 / 16.2 / 33.2 | 8.4 / 9.9 / 15.2 | 2 / 4 / 6.4 | 1 / 1 / 1.4 | 0 / 1 / 1 | 35k / 73k / 122k | 29k / 61k / 113k | 0 | 0 |
| `signatures` | 23/24 | 10.0 / 14.2 / 35.0 | 8.4 / 10.5 / 14.4 | 2 / 3 / 7 | 1 / 1 / 2 | 0 / 0 / 0 | 35k / 55k / 138k | 29k / 48k / 121k | 0 | 0 |
| **`namespaces`** (default) | 23/24 | 8.4 / 12.5 / 31.5 | 7.0 / 9.4 / 11.6 | 2 / 3 / 6.4 | 1 / 1 / 2 | 0 / 0 / 0 | 35k / 54k / 125k | 29k / 44k / 112k | 0 | 0 |
| `bridged` | 24/24 | 7.6 / 10.0 / 25.3 | 5.6 / 6.9 / 8.6 | 2 / 3 / 5 | 1 / 1 / 1.4 | 0 / 0 / 0 | 36k / 56k / 94k | 30k / 46k / 73k | 0 | 0 |

### 1,000 tools

| Arm | Done | End to end (s) | First correct tool (s) | Model turns | Searches | Describes | Input tokens | Cached tokens | Tools-array changes | Leaks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `off` | 23/24 | 12.3 / 19.8 / 35.8 | 9.3 / 11.7 / 15.5 | 1 / 2.5 / 4.4 | 0 / 0 / 0 | 0 / 0 / 0 | 177k / 442k / 780k | 121k / 359k / 602k | 0 | 0 |
| `compact` | 23/24 | 8.4 / 15.6 / 39.8 | 9.8 / 13.8 / 15.8 | 2 / 4 / 8 | 1 / 1 / 2 | 0 / 1 / 1 | 35k / 74k / 153k | 29k / 61k / 128k | 0 | 0 |
| `signatures` | 24/24 | 10.2 / 14.0 / 39.6 | 7.3 / 10.3 / 14.7 | 2 / 3 / 8.6 | 1 / 1 / 3 | 0 / 0 / 0 | 35k / 55k / 186k | 29k / 47k / 159k | 0 | 0 |
| **`namespaces`** (default) | 23/24 | 9.0 / 12.0 / 38.5 | 7.5 / 10.4 / 11.9 | 2 / 3 / 8.8 | 1 / 1 / 2.4 | 0 / 0 / 0 | 35k / 55k / 171k | 29k / 45k / 159k | 0 | 0 |
| `bridged` | 24/24 | 8.4 / 10.9 / 27.2 | 5.5 / 6.4 / 8.5 | 2 / 3 / 9.6 | 1 / 1 / 2 | 0 / 0 / 0 | 36k / 56k / 249k | 30k / 49k / 235k | 0 | 0 |

### Failures

This table includes the excluded task. It shows completed runs out of 12
(3 catalog sizes × 4 repetitions) per task class:

| Task | `off` | `compact` | `signatures` | `namespaces` | `bridged` |
| --- | ---: | ---: | ---: | ---: | ---: |
| `ambiguous-relevant-set` | 12 | 12 | 12 | 12 | 12 |
| `cross-namespace-workflow` (excluded above) | **0** | **0** | **0** | **0** | **0** |
| `denied-capability` | 12 | 10 | 11 | 10 | 12 |
| `exact-canonical-id` | 11 | 12 | 12 | 12 | 12 |
| `natural-language-alias` | 12 | 11 | 11 | 12 | 12 |
| `nested-argument-vocabulary` | 12 | 12 | 12 | 12 | 12 |
| `no-match` | 12 | 11 | 12 | 12 | 12 |

- **Why the cross-namespace workflow is excluded.** The task asks for
  `gmail__search_messages` then `google_calendar__create_event`, with a
  schedule taken from the email (10:00–10:30 on 2026-08-12). The model reaches
  `google_calendar__create_event` in 30 of the 60 runs, but its `schedule`
  never passes the task's argument check. In most runs it also, or instead,
  calls `google_calendar__list_events` (41 of 60). It fails in `off` too, so
  it says nothing about discovery. Its 60 runs used many more model calls than
  any other task, up to 75 in one run, before hitting the time limit. 41 of them hit the time limit. They made up most of the token total in
  several cells, for example 77% of the `off` arm's input tokens at 500 tools. It should be fixed or replaced before the results run, so that
  the comparison covers all seven task classes again.
- **The remaining failures.** Every failed observation has the category
  `task_incomplete`. That category also covers runs the browser probe gave up
  on at the 180 s limit. With the cross-namespace task excluded, 25 runs have a
  failure category, and 16 of them still made the required calls correctly
  (so **Done** counts them). They then kept calling tools until the limit, for
  example repeating `hubspot__search_contacts` up to five times. These runs
  are what push the 80th percentiles up.

### What the tools-array metric shows

The metric counts changes in the advertised tools array between consecutive
model requests **within one task**, measured by the harness's model relay. It
is 0 in every cell. That is expected rather than reassuring: promotion (the
#6986 cache problem) changes the tools array only on the *next* turn
(`crates/loop/ironclaw_loop_host/src/tool_disclosure_port.rs`, see the
"promote the target on the next turn" tests), and every task in this matrix is
a single turn. So the baseline confirms that the array is stable within a turn,
and says nothing about churn across turns. Measuring that needs a multi-turn
task.

## Reproducing the run

Requirements:

- A Linux host with outbound internet, where `example.com` resolves in DNS to a
  public address (see below). Nothing is ever sent to that address: the policy
  layer resolves the name and rejects private addresses before the test rewrite
  redirects the connection to the local fixture. So it must not be `127.0.0.1`.
  If DNS cannot resolve it, add a hosts entry with the address public DNS
  returns (`dig +short example.com` on another machine), for example
  `echo "<address>  example.com" | sudo tee -a /etc/hosts`.
- Python 3.11+ and the harness's Python dependencies, including Playwright with
  Chromium. The harness drives the real WebUI in a browser. The live-QA tools
  keep these in a virtualenv at `tests/e2e/.venv`:

  ```bash
  python3 -m venv tests/e2e/.venv
  tests/e2e/.venv/bin/python -m pip install -e tests/e2e
  tests/e2e/.venv/bin/python -m playwright install chromium   # add --with-deps on a fresh host
  ```
- An OpenAI-compatible endpoint and key. The run makes roughly 420 tasks'
  worth of model calls, several per task, and the `off` arm at 1,000 tools
  sends every schema on every call, so budget for several million input
  tokens, most of them from the `off` arm.
- Several hours: 105 task groups, each starting a fresh server and installing
  20 MCP packages before its four repetitions.

```bash
git checkout k-12/tool-rag-baseline        # or the commit that landed it

# 1. Build the binary under test (debug; see below).
cargo build -p ironclaw

# 2. Model settings.
export REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID=openai_compatible
export LIVE_OPENAI_COMPATIBLE_BASE_URL=https://api.venice.ai/api/v1
export LIVE_OPENAI_COMPATIBLE_MODEL=deepseek-v4-flash-0731
export LIVE_OPENAI_COMPATIBLE_API_KEY=...   # never commit this

# 3. Read the key from the variable above. NEARAI_API_KEY wins when it is set.
unset NEARAI_API_KEY
export REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV=LIVE_OPENAI_COMPATIBLE_API_KEY

# 4. Let the harness write its own config. An existing home with a
#    config.toml is used as-is, and the model settings above are ignored.
unset REBORN_WEBUI_V2_LIVE_QA_HOME

# 5. Run the full default matrix.
tests/e2e/.venv/bin/python scripts/tool_discovery_benchmark/run_benchmark.py \
  --binary target/debug/ironclaw \
  --output-dir "$HOME/.cache/tool-rag-baseline"
```

Why steps 3 and 4 are needed:

- **The key.** When `REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV` is unset, the
  harness takes the key from `NEARAI_API_KEY` if that variable exists, even
  when the provider is not NEAR AI (`_write_minimal_reborn_config` in
  `scripts/reborn_webui_v2_live_qa/run_live_qa.py`). The endpoint would then
  get the wrong key.
- **The home.** The harness writes a `config.toml` only into a home that has
  none. If `REBORN_WEBUI_V2_LIVE_QA_HOME` points at an existing home, its
  model settings win over the exports.

The output directory is off `/tmp` on purpose. On hosts where `/tmp` is a
small in-memory filesystem, a run of several hours that starts a fresh server
for each task group can fill it.

Build and run the **debug** binary. The benchmark's MCP packages live at
`https://example.com/benchmark/<n>`, and `IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP`
sends that traffic to the local fixture. That rewrite exists only in debug
builds: a release binary does not compile it in
(`crates/app/ironclaw_composition/src/factory/runtime_lane_assembly.rs`), and
the rewrite module refuses the map at runtime too
(`crates/substrates/ironclaw_network/src/test_rewrite.rs`). A release binary
would therefore try to reach the real `example.com`, and package setup would
fail.

A debug build is slower on the host side, such as `tool_search` ranking,
schema rendering and MCP dispatch, so absolute latencies are higher than in
production. Model time dominates each turn, so the comparison between arms
still holds, provided the results run uses the same debug build.

Do not pass `--arm`, `--tool-count`, `--task` or `--repetitions`: the
baseline is the full default matrix. An interrupted run resumes from
`observations.jsonl` when started again with the same `--output-dir`. The
runner puts a loopback relay in front of the model endpoint to measure
tools-array changes; it forwards traffic unchanged and never stores headers or
the key.

Then archive the small artifacts next to this page and fill in the tables,
computing them from `observations.jsonl` as described under
[Results](#results).
Replace the machine-local `observations_path` in `summary.json` with the
archived file name:

```bash
mkdir -p docs/internal/benchmarks/tool-rag-baseline
cp "$HOME/.cache/tool-rag-baseline/summary.json" \
   "$HOME/.cache/tool-rag-baseline/observations.jsonl" \
   docs/internal/benchmarks/tool-rag-baseline/
```

Keep the model traces, browser diagnostics and server logs out of the
repository; they are large and are not needed to read the result.
`observations.jsonl` holds no prompts, credentials or tool arguments.

## Run notes

- The first attempts failed at MCP package setup with HTTP 503
  (`/api/webchat/v2/extensions/mcp-benchmark-browser/setup` →
  `service_unavailable`). There were two causes: `example.com` did not resolve
  on the host, and one attempt used a release binary, which has no fixture
  rewrite. A hosts entry for `example.com` plus the debug binary fixed both.
- The output directory was `~/.cache/tool-rag-baseline`. The model traces,
  browser diagnostics and failure screenshots stay there and are not archived.
