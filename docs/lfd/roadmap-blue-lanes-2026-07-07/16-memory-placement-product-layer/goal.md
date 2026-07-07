# Goal: move memory authority to the product/provider layer

Source page: https://app.notion.com/p/38729a6526bf81a4896fca39784c347f

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for memory placement. The spec must define the product-facing memory provider boundary, native provider default, host/admin policy, authorization, audit, storage, sandboxing, streaming, and network mediation.

The spec must identify existing low-level memory behavior that must keep working and the dependency directions that are now forbidden.

## Target (outer loop)

Optimize architecture and behavior parity:

- 35% provider boundary exists and native memory works through it by default.
- 25% host/admin policy can allow, deny, or constrain providers.
- 25% existing memory reads, writes, search, and prompt behavior remain behaviorally equivalent.
- 15% audit, auth, sandboxing, storage, streams, and network remain host-mediated.

Bar: at least 0.95 holdout, architecture tests green, zero host-mediation bypasses.

## Eval design

Create 50 dev and 120 holdout placement checks. Inputs include native provider operation, fake provider operation, denied provider, constrained provider, memory read/write/search, prompt inclusion, audit event, dependency path, and host mediation attempt.

Answers include allowed/denied provider decision, memory operation result, audit event, dependency compliance, and parity outcome.

## Harness design

Combine static dependency checks with behavior parity tests. `harness/score.sh` must:

- Run architecture/dependency lints for forbidden low-level authority shortcuts.
- Run native provider parity tests.
- Swap in a fake provider to prove product code does not special-case native memory.
- Run denied/constrained provider policy cases.
- Assert audit and host mediation events.

## Constraints

- Wall-clock budget: 14 hours.
- Spend ceilings: $10 LLM/API spend; no live external memory provider.
- Surface allowlist: `ironclaw_memory`, `ironclaw_memory_native`, memory provider traits, product workflow, Reborn composition, host policy/auth/audit surfaces, tests, and docs.
- Capacity caps: shared caps; provider-specific branches capped at 5.
- Do not re-export old internals under new names as a substitute for a boundary.

## Cycle protocol

Follow the common cycle protocol. Each cycle must run both one behavior parity case and one architecture/dependency case. If a fake provider fails, do not add native-specific behavior until the provider boundary is corrected.

## Entropy rules

- Rotate between provider API, policy, parity, and dependency boundaries every 3 cycles.
- If native provider passes but fake provider fails, next cycle targets abstraction leakage.
- If dependency lint is noisy, fix the lint calibration rather than disabling architecture checks.

## Cheat audit

Lane-specific cheap wins to block:

1. Re-export old internals under a new provider name; dependency lint fails.
2. Special-case native provider in product code; fake provider probe fails.
3. Bypass host policy for speed; denied-provider cases fail.
4. Preserve reads but break writes; parity eval includes both.
5. Preserve storage but drop audit; audit score fails.
6. Move memory authority into prompts; boundary and leakage checks fail.
7. Delete old behavior without parity evidence; parity tests fail.
8. Add provider branches for each eval case; capacity cap and probe fail.
9. Hide network access inside provider; host mediation cases fail.
10. Treat architecture score as docs-only; static checks and runtime probes are required.

## Stop conditions

Stop when holdout is at least 0.95 with zero mediation bypasses and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or memory operations can bypass product/provider policy.

