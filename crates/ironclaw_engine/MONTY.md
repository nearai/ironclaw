# Monty Integration

Monty is the embedded Python interpreter used for Tier 1 (CodeAct) execution. It's a lightweight Rust-native Python implementation — not CPython — so it has a restricted feature set.

**Source**: `git = "https://github.com/pydantic/monty.git", tag = "v0.0.11"`
**Pinned at**: `v0.0.11` (2026-04-10)

## Upgrade Process

1. **Update the pin**: `cargo update -p monty`
2. **Check for new features**: `cd ~/.cargo/git/checkouts/monty-*/*/` and `git log --oneline` since last pin
3. **Update the preamble**: If a previously-unsupported feature now works, remove it from the "Runtime environment" section in `prompts/codeact_preamble.md`
4. **Update this file**: Record the new pin and what changed
5. **Run tests**: `cargo test -p ironclaw_engine`
6. **Watch traces**: After deploying, check traces for new `NotImplementedError` patterns (self-improvement mission catches these)

## Current Limitations (as of pin `v0.0.11`)

These are documented in `prompts/codeact_preamble.md` so the LLM avoids them:

### Syntax not supported
| Feature | Workaround |
|---------|-----------|
| `class Foo:` | Use functions and dicts (host-provided dataclasses work) |
| `with` statements | Use try/finally or direct calls |
| `match` statements | Use if/elif chains |
| `del` statement | Reassign to None |
| `yield` / `yield from` statements | Generator expressions (`x for x in ...`) work; use lists for the rest |
| Type aliases (`type X = ...`) | Omit type annotations |
| Template strings (t-strings) | Use f-strings |
| Complex number literals | Use floats |
| Exception groups (`try*/except*`) | Use regular try/except |

### Limited standard library
`import csv`, `import io`, etc. still fail.

`import os` succeeds but all operations (`os.getenv()`, `Path.*`) are **blocked** by the executor — `OSError: OS operations are not permitted in CodeAct scripts`. This is intentional: agents must use injected tools (`shell`, `read_file`, etc.) instead.

Available built-in modules:
- `asyncio` — `asyncio.gather()` for parallel execution
- `datetime` — date and time handling
- `json` — JSON encoding/decoding
- `math` — standard math functions
- `os.path` — path string manipulation only (no I/O)
- `re` — regex (basic)
- `sys` — system info (limited)
- `typing` — type hints (limited, for annotation only)

### Available builtins
`abs`, `all`, `any`, `bin`, `chr`, `divmod`, `enumerate`, `filter`, `getattr`, `hash`, `hex`, `id`, `isinstance`, `len`, `map`, `min`, `max`, `next`, `oct`, `ord`, `pow`, `print`, `repr`, `reversed`, `round`, `sorted`, `sum`, `type`, `zip`

### Host-provided functions (always available)
These are injected by the IronClaw executor, not by Monty:
- `FINAL(answer)` / `FINAL_VAR(name)` — terminate with result
- `llm_query(prompt, context)` — recursive LLM sub-call
- `llm_query_batched(prompts)` — parallel sub-calls
- `rlm_query(prompt)` — full sub-agent with tools
- `globals()` / `locals()` — returns dict of known tool names
- All tool functions (web_search, http, time, etc.)

## Upgrade Changelog

| Date | Pin | Notable changes |
|------|-----|-----------------|
| 2026-04-10 | `v0.0.11` | JSON perf improvements (~2x loads, ~1.6x dumps), filesystem mounting, Rust-side async API, mount edge case fixes. |
| 2026-03-29 | `7a0d4b7` | Multi-module imports, `datetime` module, `json` module, nested subscript assignment, `str.expandtabs()`. |
| 2026-03-20 | `6053820` | Initial integration. max() kwargs support. |
