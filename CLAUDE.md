# IronClaw — Claude Code adapter

@AGENTS.md

Everything above (from `AGENTS.md`) is the canonical, tool-neutral contract.
The rest of this file is Claude-specific.

## Skills and rules

- Project skills live in `.claude/skills/` — start from
  `ironclaw-reborn-orientation`; use `reborn-feature` for cross-layer product
  work, `reborn-extension-surfaces` for integrations,
  `ironclaw-reborn-testing` for test tiers, and
  `ironclaw-reborn-architecture-review` for boundary changes.
- Path-scoped rules in `.claude/rules/*.md` load automatically when you read
  matching files — they are canonical for their topics (testing, database,
  types, cargo-features, review discipline, guidance maintenance, …); do not
  restate them here. Guidance edits (`.claude/**`, any `AGENTS.md`/`CLAUDE.md`,
  runtime `skills/`) are governed by `.claude/rules/guidance-maintenance.md`.

## Codebase knowledge graph (MCP)

The `codebase-memory` MCP server indexes `crates/` into a knowledge graph;
prefer it over `Grep` for *code structure* (cross-crate call chains are
invisible to text search). `.codebase-memory/graph.db.zst` is the committed
bootstrap snapshot; local databases and
`.codebase-memory/artifact.json` <!-- check-guidance: path-ok --> stay
git-ignored (per-environment state, deliberately untracked). Check
freshness first: `bash scripts/codebase-graph.sh status`.

- Missing → `index_repository(repo_path=".", persistence=true)` once.
- Stale → `detect_changes(since="<indexed-commit>")`, or re-run
  `index_repository` to refresh the shared snapshot.
- Where is a symbol → `search_graph(name_pattern=…)`, then
  `get_code_snippet(qualified_name=…)`.
- Who calls X / what X calls → `trace_path(function_name=…, mode="calls")`;
  value flow → `mode="data_flow"`; cross-crate feature path →
  `mode="cross_service"`.
- Structure of an area → `get_architecture(…)`; graph-augmented text search →
  `search_code(pattern=…)`; arbitrary queries → `query_graph(<Cypher>)`.

The graph is a point-in-time index — verify anything it asserts against live
code before acting. `Grep`/`Glob`/`Read` remain correct for text, config, and
non-code files. For narrative *what/why* orientation, read the relevant
`openwiki/` page (generated — never hand-edit).

## REPL/TUI logging rule

`info!` and `warn!` output appears in the REPL and corrupts the terminal UI.
Use `debug!` for internal diagnostics (trace analysis, reflection results,
engine internals); reserve `info!` for user-facing status the REPL
intentionally renders. Background tasks must NEVER use `info!`.
