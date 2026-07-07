# Goal: let non-expert users build focused custom tools safely

Source page: https://app.notion.com/p/36e29a6526bf80b48306e15daad54fa7

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for one supported first tool shape: a credentialed HTTP API wrapper generated from user intent plus lightweight API documentation. Arbitrary code generation, unsandboxed shell tools, and broad plugin marketplaces are out of scope for this lane.

The spec must define:

- Requirements extraction into a typed tool spec.
- Supported auth methods and secret binding.
- Generated artifact format and registry conventions.
- Sandbox and validation tests.
- Failure diagnostics that a non-expert can act on.
- Removal/update flow for generated tools.

## Target (outer loop)

Optimize supported custom-tool build success:

- 30% user request and docs become a correct typed tool spec.
- 30% generated tool passes sandbox and fake API contract tests.
- 20% credential setup and injection work without leaks.
- 10% failure diagnostics are actionable.
- 10% generated artifact follows registry and lifecycle conventions.

Bar: at least 0.85 holdout for supported simple integrations, zero sandbox or secret policy bypass.

## Eval design

Create 60 dev and 140 holdout fake API tasks. Inputs include user request, small docs or OpenAPI-like fragments, auth variant, endpoint schema, pagination, errors, malformed docs, and user over-asking beyond supported scope.

Answers include typed tool spec, generated tool manifest/code expectations, fake API calls, credential use, sandbox permissions, validation results, and diagnostic class.

## Harness design

Build tools in an isolated temp registry and run fake API contract tests. Do not let generated tools reach the host network except the fake provider.

`harness/score.sh` must:

- Generate or update a tool artifact from the request and docs.
- Validate manifest, permissions, auth binding, and schema.
- Run fake API happy and error paths.
- Scan generated code and logs for secrets, host escapes, shell execution, and eval-shaped hardcoding.
- Calibrate against known-bad variants: hardcoded answers, unsandboxed shell, ignored auth, and no error handling.

## Constraints

- Wall-clock budget: 16 hours.
- Spend ceilings: $30 LLM/API spend for generation/eval; no live third-party API calls.
- Surface allowlist: tool/extension registries, WASM or sandboxed extension paths, `ironclaw_first_party_extensions`, `ironclaw_extensions`, `ironclaw_host_runtime`, secret injection, tests, and docs.
- Capacity caps: shared caps; generated examples in prompts capped at 8; fake API seed docs capped at 40.
- Generated tools must be deterministic artifacts with tests, not prompt-only skills that claim capability.

## Cycle protocol

Follow the common cycle protocol. Each cycle must run at least one generated tool against a fake API and one malformed or over-scoped request. Commit only after sandbox and secret scans pass.

## Entropy rules

- Rotate auth, pagination, error, and schema complexity every 4 cycles.
- If success comes from more prompt examples, the next cycle must improve typed validation or sandboxing instead.
- If unsupported requests are accepted too often, stop adding features and tighten scope refusal.

## Cheat audit

Lane-specific cheap wins to block:

1. Hardcode fake API answers; probe swaps endpoint names and fields.
2. Generate unsandboxed shell tools; sandbox scan fails.
3. Ignore credentials and call public endpoints; fake provider requires auth where specified.
4. Put secrets in generated code; secret canary scan fails.
5. Accept arbitrary-code requests; unsupported-scope cases must deny.
6. Skip error paths; fake provider returns structured errors.
7. Require manual Rust edits; lifecycle convention score fails.
8. Store tool in temp only; registry visibility and invocation are scored.
9. Overfit OpenAPI examples; docs are perturbed and incomplete in holdout.
10. Hide failures behind generic diagnostics; diagnostic class is scored.

## Stop conditions

Stop when holdout is at least 0.85 with zero sandbox/secret bypasses and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or generated artifacts can escape sandbox or expose secrets.

