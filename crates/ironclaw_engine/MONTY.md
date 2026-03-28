# Monty Integration

Monty is the embedded Python interpreter used for Tier 1 (CodeAct) execution. It's a lightweight Rust-native Python implementation — not CPython — so it has a restricted feature set.

**Source**: `git = "https://github.com/pydantic/monty.git", branch = "main"`
**Pinned at**: `6053820` (2026-03-27, "Support max() kwargs/default")

## Upgrade Process

1. **Update the pin**: `cargo update -p monty`
2. **Check for new features**: `cd ~/.cargo/git/checkouts/monty-*/*/` and `git log --oneline` since last pin
3. **Update the preamble**: If a previously-unsupported feature now works, remove it from the "Runtime environment" section in `prompts/codeact_preamble.md`
4. **Update this file**: Record the new pin and what changed
5. **Run tests**: `cargo test -p ironclaw_engine`
6. **Watch traces**: After deploying, check traces for new `NotImplementedError` patterns (self-improvement mission catches these)

## Current Limitations (as of pin `6053820`)

These are documented in `prompts/codeact_preamble.md` so the LLM avoids them:

### Syntax not supported
| Feature | Workaround |
|---------|-----------|
| `import a, b, c` (multi-module) | Use separate `import a` / `import b` statements |
| `class Foo:` | Use functions and dicts |
| `with` statements | Use try/finally or direct calls |
| `match` statements | Use if/elif chains |
| `del` statement | Reassign to None |
| `yield` / `yield from` | Use lists and list comprehensions |
| `*expr` (starred expressions) | Unpack explicitly |
| `async` / `await` | Not available; tool calls suspend the VM automatically |
| Type aliases (`type X = ...`) | Omit type annotations |
| Template strings (t-strings) | Use f-strings |
| Complex number literals | Use floats |
| Exception groups (`try*/except*`) | Use regular try/except |

### No standard library
`import datetime`, `import csv`, `import json`, `import os`, `import io`, etc. all fail.

Available built-in modules:
- `math` — standard math functions
- `re` — regex (basic)
- `sys` — system info (limited)
- `os.path` — path manipulation (limited)
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
| 2026-03-20 | `6053820` | Initial integration. max() kwargs support. |
