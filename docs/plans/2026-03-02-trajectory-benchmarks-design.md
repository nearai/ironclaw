# Trajectory Benchmark System

**Date**: 2026-03-02
**Status**: Design
**Goal**: Evaluate agent quality across real user flows -- catch regressions when code changes, measure improvements when new tools/skills are added.

## Overview

A benchmark system that runs **real user scenarios** through the **real agent loop** with **real LLM calls**, then evaluates the resulting trajectory using two layers:

1. **Hard assertions** -- pass/fail checks on tool selection, response content, cost, latency
2. **LLM-as-judge** -- quality scoring for reasoning, tool use, and response helpfulness

This is an **eval system**, not a unit test suite. It answers: "Does the agent actually solve problems well?"

## Scenario Format

Each trajectory is a YAML file in `benchmarks/trajectories/`:

```yaml
name: schedule-meeting
description: User asks to schedule a meeting with workspace context
tags: [tools, scheduling, memory]

# Environment setup
setup:
  skills: [calendar-skill]
  tools: [time, shell, http, memory_search, memory_read]
  provider: nearai
  model: default

  # Pre-populate workspace memory before the scenario runs
  workspace:
    documents:
      - path: "context/team.md"
        content: |
          # Team
          - Alice (engineering lead, PST timezone)
          - Bob (product, EST timezone)
          - Carol (design, CET timezone)
      - path: "preferences/scheduling.md"
        content: |
          # Scheduling Preferences
          - Default meeting length: 30 minutes
          - Prefer afternoons for syncs
          - Always include Zoom link
    # Or load from fixture directory
    fixtures_dir: benchmarks/fixtures/scheduling/

  # Override identity files injected into system prompt
  identity:
    USER.md: |
      Name: Zaki
      Timezone: PST
      Preferred calendar: Google Calendar

# Conversation turns (multi-turn supported)
turns:
  - user: "Schedule a team sync for tomorrow at 2pm"
    assertions:
      tools_called: [time, memory_search]
      tools_not_called: [shell]
      response_contains: ["Alice", "Bob", "Carol"]
      response_not_contains: ["error", "sorry"]
      max_tool_calls: 8
      max_cost_usd: 0.10
      max_latency_secs: 30
    judge:
      criteria: |
        Did the agent search workspace memory for team and scheduling info?
        Did it account for timezone differences across the team?
        Is the response personalized using workspace context?
      min_score: 7
```

### Format Details

- **`setup.skills`**: Skills to activate for this scenario. Only these skills will be available.
- **`setup.tools`**: Tools to register beyond default builtins. Controls tool availability.
- **`setup.workspace.documents`**: Seed workspace memory with documents at specified paths. Torn down after scenario.
- **`setup.workspace.fixtures_dir`**: Load all files from a directory into workspace memory.
- **`setup.identity`**: Override AGENTS.md, USER.md, SOUL.md, IDENTITY.md for this scenario.
- **`turns`**: Ordered list of user messages. Each turn can assert on the agent's behavior.
- **`assertions`**: Hard pass/fail. The runner also uses `max_tool_calls`, `max_cost_usd`, and `max_latency_secs` as circuit breakers to kill runaway scenarios.
- **`judge`**: Criteria string sent to a separate LLM call. Produces a 1-10 score.

### Multi-Turn Example

```yaml
turns:
  - user: "Save a note: Project Alpha launches on March 15th"
    assertions:
      tools_called: [memory_write]
      response_contains: ["saved", "note"]

  - user: "When does Project Alpha launch?"
    assertions:
      tools_called: [memory_search]
      response_contains: ["March 15"]
    judge:
      criteria: |
        Did the agent retrieve the previously saved note?
        Is the answer accurate and concise?
      min_score: 8
```

## Directory Structure

```
benchmarks/
  trajectories/           # Scenario YAML files
    tool-selection/
      pick-time-tool.yaml
      pick-memory-tool.yaml
      pick-file-tool.yaml
    skill-activation/
      skill-improves-response.yaml
    multi-turn/
      save-and-recall.yaml
      conversation-coherence.yaml
    safety/
      prompt-injection.yaml
      secret-leak.yaml
    cost-efficiency/
      simple-question-minimal-turns.yaml
  fixtures/               # Shared workspace fixture files
    scheduling/
    common/
  results/                # Run output (gitignored)
    2026-03-02T10:30:00/
      summary.json
      schedule-meeting.json
  baselines/              # Golden scores to compare against
    latest.json
```

## Execution Architecture

### Runner

A Rust binary (`src/bin/benchmark.rs` or workspace crate) that:

1. **Discovers** scenario YAML files from `benchmarks/trajectories/`
2. **For each scenario** (isolated):
   - Creates a fresh libSQL database (reusing `TestHarnessBuilder` patterns)
   - Seeds workspace documents and identity files per `setup.workspace` and `setup.identity`
   - Constructs a **real `Agent`** with a **real LLM provider** (configured via env vars)
   - Registers only the tools listed in `setup.tools`
   - Activates only the skills listed in `setup.skills`
3. **Executes turns** sequentially:
   - Sends `IncomingMessage` through `agent.handle_message()`
   - Captures full trajectory: tool calls (name, params, output, duration), final response, token cost, wall-clock latency
   - Enforces circuit breakers: kills scenario if `max_tool_calls`, `max_cost_usd`, or `max_latency_secs` is exceeded
4. **Evaluates** each turn:
   - Runs hard assertions (pass/fail)
   - If `judge` block present: sends trajectory summary + criteria to a separate LLM call, records 1-10 score
5. **Reports**: Writes per-scenario JSON results + suite summary, compares against baselines

### Key Design Decisions

- **Real agent loop**: Not mocked. Uses the actual `Agent`, `dispatcher`, `worker`, `ToolRegistry`, `SafetyLayer`. Tests the whole system.
- **Isolated per scenario**: Each scenario gets its own libSQL database and workspace. No cross-contamination.
- **Parallel safe**: Because scenarios are isolated, they can run concurrently (limited by LLM API rate limits).
- **Judge model**: Use a cheaper/faster model for judging (e.g., Haiku, GPT-4o-mini) since it's scoring, not reasoning.

## Cost and Speed Controls

### Per-Scenario Guards

The `assertions` fields double as circuit breakers:

| Guard | Function |
|-------|----------|
| `max_tool_calls` | Kill agent loop if tool call count exceeds limit |
| `max_cost_usd` | Kill if token cost exceeds budget |
| `max_latency_secs` | Timeout per turn |

### CLI Interface

```bash
# Full suite (nightly/pre-release)
cargo run --bin benchmark

# Tagged subset (PR CI)
cargo run --bin benchmark -- --tags basic,tools

# Single scenario (development)
cargo run --bin benchmark -- --scenario schedule-meeting

# Budget cap for entire run
cargo run --bin benchmark -- --max-total-cost 5.00

# Assertions only, skip judge (cheaper)
cargo run --bin benchmark -- --no-judge

# Parallel execution
cargo run --bin benchmark -- --parallel 4

# Update baselines after a good run
cargo run --bin benchmark -- --update-baseline
```

### Cost Strategy

| Context | What to run | Estimated cost |
|---------|-------------|----------------|
| **Development** | Single scenario, `--no-judge` | Pennies |
| **PR CI** | `--tags basic` (5-10 scenarios), assertions only | $0.50-2.00 |
| **Nightly** | Full suite with judge scoring | $5-20 |
| **Pre-release** | Full suite, compare against frozen baselines, gate release | $5-20 |

## Reporting

### Per-Scenario Result

```json
{
  "scenario": "schedule-meeting",
  "timestamp": "2026-03-02T10:30:00Z",
  "provider": "nearai",
  "model": "claude-3-5-sonnet-20241022",
  "turns": [
    {
      "user_message": "Schedule a team sync for tomorrow at 2pm",
      "tool_calls": [
        {"tool": "time", "params": {}, "duration_ms": 12, "output_preview": "2026-03-03"},
        {"tool": "memory_search", "params": {"query": "team members"}, "duration_ms": 45, "output_preview": "Found 1 document"}
      ],
      "response": "I've prepared a meeting invite for...",
      "assertions": {
        "tools_called": {"pass": true, "expected": ["time", "memory_search"], "actual": ["time", "memory_search"]},
        "response_contains": {"pass": true},
        "max_cost_usd": {"pass": true, "actual": 0.03}
      },
      "judge_score": 8,
      "judge_reasoning": "Correctly identified scheduling task, searched memory for team info...",
      "cost_usd": 0.03,
      "latency_ms": 2400,
      "total_tool_calls": 3
    }
  ],
  "passed": true,
  "total_cost_usd": 0.04,
  "total_latency_ms": 2400
}
```

### Suite Summary

```json
{
  "timestamp": "2026-03-02T10:30:00Z",
  "git_sha": "abc123",
  "scenarios": 24,
  "passed": 22,
  "failed": 1,
  "errored": 1,
  "total_cost_usd": 1.47,
  "total_duration_secs": 85,
  "avg_judge_score": 7.8,
  "regressions": [
    {
      "scenario": "multi-tool-research",
      "metric": "judge_score",
      "baseline": 8,
      "current": 5,
      "delta": -3
    }
  ],
  "improvements": [
    {
      "scenario": "file-editing",
      "metric": "judge_score",
      "baseline": 6,
      "current": 9,
      "delta": 3
    }
  ]
}
```

### Baseline Management

- `benchmarks/baselines/latest.json` stores last known-good scores per scenario.
- Runner compares current scores against baselines.
- **Regression** = score drop > configurable threshold (default: -2 points).
- **Improvement** = score increase > threshold.
- Hard assertion failures = test failure regardless of scores.
- `--update-baseline` copies current results into baselines after a satisfactory run.

## Starter Scenarios

### 1. Tool Selection (does the agent pick the right tool?)

| Scenario | Prompt | Expected Tool |
|----------|--------|---------------|
| `pick-time-tool` | "What time is it?" | `time` (not `shell`) |
| `pick-memory-tool` | "What do I know about project X?" | `memory_search` |
| `pick-file-tool` | "Write 'hello' to /tmp/test.txt" | `file_write` (not `shell echo >`) |

### 2. Skill Activation (do skills improve behavior?)

| Scenario | Setup | Check |
|----------|-------|-------|
| `skill-improves-response` | Run same prompt with and without a skill | Score should be higher with skill active |

### 3. Multi-Turn Coherence

| Scenario | Turn 1 | Turn 2 |
|----------|--------|--------|
| `save-and-recall` | "Save note: Alpha launches March 15" | "When does Alpha launch?" -- must retrieve from memory |

### 4. Safety Boundaries

| Scenario | Prompt | Check |
|----------|--------|-------|
| `prompt-injection` | Message with injection attempt | Must NOT call dangerous tools |
| `secret-leak` | Input containing API key pattern | Leak detector should catch it |

### 5. Cost Efficiency

| Scenario | Prompt | Check |
|----------|--------|-------|
| `simple-question` | "What is 2+2?" | Should resolve in 1-2 tool calls max, low cost |

## Implementation Notes

### Reuse from Existing Infrastructure

- **`TestHarnessBuilder`** (`src/testing.rs`): Provides libSQL DB, tool registry, safety layer. Extend to accept real LLM providers instead of `StubLlm`.
- **`IncomingMessage`** (`src/channels/channel.rs`): Construct test messages directly.
- **`Agent.handle_message()`**: The entry point -- feed it messages, capture responses.
- **Workspace seeding**: Use existing `memory_write` tool or direct DB inserts.

### What Needs to Be Built

1. **Scenario parser**: YAML deserialization into scenario structs
2. **Scenario runner**: Orchestrates setup -> execute -> evaluate -> teardown per scenario
3. **Assertion engine**: Evaluates hard assertions against captured trajectory
4. **Judge caller**: Formats trajectory + criteria, calls LLM, parses score
5. **Reporter**: Generates per-scenario and summary JSON, compares baselines
6. **CLI**: Argument parsing for tags, parallelism, budget caps, etc.

### Dependencies

- `serde_yaml` (or `serde` with YAML feature) for scenario parsing
- Existing `tokio`, `serde_json`, libSQL deps
- No new heavy dependencies expected
