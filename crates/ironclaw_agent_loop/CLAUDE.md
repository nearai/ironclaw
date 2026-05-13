# ironclaw_agent_loop guardrails

- Owns "what an agent loop is": strategy traits, the `AgentLoopPlanner` facade,
  the `AgentLoopExecutor` trait + canonical impl, and `LoopExecutionState`.
- Stays one layer above `ironclaw_turns` (which owns runner-facing turn
  contracts). Depends on `ironclaw_turns` for `LoopRunContext`, `LoopExit`,
  `LoopXxxPort` traits, and ref types.
- Does NOT depend on `ironclaw_reborn`. The framework crate has no knowledge
  of `AgentLoopDriver`; that bridge lives in `PlannedDriver` in
  `ironclaw_reborn`.
- Stores refs, cursors, counters, versions, and safe summaries only. Never
  raw prompts, raw model output, raw tool input, secrets, host paths, provider
  errors, or stack traces in `LoopExecutionState` or any strategy slot.
- Strategies are `&self`-only; `LoopExecutionState` is value-immutable. All
  mutation happens by the executor swapping a strategy's returned slot into
  the next whole state. There is no `&mut LoopExecutionState` API.
- New strategies, slots, and outcome enums must land typed (no string keys,
  no `serde_json::Value` interior in long-lived state). Per
  `.claude/rules/types.md`.
- Master spec: `docs/reborn/agent-loop-skeleton.md`. Workstream briefs:
  `docs/reborn/agent-loop-briefs/`.
