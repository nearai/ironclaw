---
name: autoverify
version: 0.1.0
description: Layered self-verification harness for autonomous coding work. Runs ironclaw verify tiers after changes, records machine-readable verdicts, and treats flaky results as unresolved until rerun or fixed.
activation:
  keywords:
    - autoverify
    - verify my work
    - prove it works
    - backtest
    - autonomous verification
    - overnight
    - ship PR
    - test tiers
  patterns:
    - "(?i)(prove|verify|test|backtest).*(work|change|PR|feature)"
    - "(?i)(ship|merge).*(after|with).*(tests|verification|backtest)"
    - "(?i)run .*verify"
  tags:
    - developer
    - testing
    - verification
  max_context_tokens: 1800
requires:
  bins:
    - git
    - cargo
---

# Autoverify

Use this skill when autonomous coding work needs evidence before it is called done. The core command is:

```bash
ironclaw verify --target <repo> --tier smoke --compact
```

IronClaw reads `.ironclaw-verify.json` first and falls back to Hermes-compatible `.autoverify.json`. The command runs named tiers in order, writes `.autoverify.state.json`, and emits a structured verdict with attempt metadata:

- `pass`: the requested tiers passed
- `flaky`: a retry passed after an initial failure; rerun or investigate before trusting it
- `fail`: at least one command failed, timed out, or could not start

## Operating Loop

1. After each code change, run the fastest meaningful tier:

```bash
ironclaw verify --target . --tier smoke --compact
```

2. If you are unsure what the repo defines, inspect the plan without running commands:

```bash
ironclaw verify --target . --list
```

3. Before shipping a PR, run all tiers needed for the touched surface:

```bash
ironclaw verify --target . --upto replay --compact
```

4. If `verdict` is `flaky`, do not hand-wave it. Rerun the same tier. If it flakes again, inspect the failing command and either fix the flake or record it as an explicit risk.

5. If `verdict` is `fail`, fix the cause and rerun the smallest tier that proves the fix. Then rerun the ship tier before committing.

6. After committing, rerun at least `smoke` against the committed tree so the final state, not an intermediate edit, is what passed.

## Config Shape

```json
{
  "version": 1,
  "tiers": [
    {
      "name": "smoke",
      "timeout_s": 60,
      "retry_on_fail": true,
      "commands": [
        { "name": "fmt", "run": "cargo fmt --all --check" },
        { "name": "unit", "run": "cargo test -p ironclaw_skills" }
      ]
    }
  ]
}
```

Keep tiers short and evidence-focused. A good suite has a cheap `smoke` tier, a targeted `unit` or `integration` tier, and a replay/backtest tier for behavior that could regress without compiler errors.
