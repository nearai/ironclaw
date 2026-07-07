# Goal: non-expert users can build credentialed HTTP API wrapper tools safely in Reborn

This is the lane-09 launch target. It merges `docs/lfd/roadmap-blue-lanes-2026-07-07/09-custom-build-tools/goal.md`, `../COMMON.md`, the lane-09 `LANE-ADDENDA.md` entry, and `lfd/_briefs/custom-build-tools.md`. The ADDENDA scope delta applies: this wave narrows custom tools to one supported shape, a credentialed HTTP API wrapper generated from user intent plus lightweight API docs. Arbitrary code generation, shell tools, generic plugin marketplaces, and broad tool builders are out of scope.

## Stage 0 - Build to spec (inner loop)

Implement `spec.md`. Make the test suite pass. Do not score against the eval until tests are green. Tests stay green every cycle thereafter.

Stage-0 command list, additive to `cargo fmt` and `cargo clippy --all --benches --tests --examples --all-features` with zero warnings:

1. `cargo test -p ironclaw_wasm` and the available `ironclaw_wasm*` crate tests, including limiter coverage for fuel, memory, and denied network.
2. `cargo test -p ironclaw_extensions` for manifest, install, update, remove, and lifecycle invariants.
3. Builder-owned crate tests for typed HTTP-wrapper spec extraction, auth binding, artifact validation, fake API contract tests, diagnostics, and removal/update flow.
4. Toolchain probe: verify `wasm32-wasip2` is installed if the selected artifact format compiles Rust components. If absent, install it in Stage 0 or make the profile return `status: "unsupported"` with a clear error until installed.
5. Make `tests/integration/lfd/profiles/custom_build_tools.rs` execute every dev case with profile name `custom_build_tools` and `status: "ran"`. The skeleton may start as `unsupported`, which scores 0 and is expected before the profile is implemented.

Only then begin descending on the eval.

## Target (outer loop)

Metric: supported custom-tool build success for credentialed HTTP API wrappers, scored by sealed contracts over real runner outcomes. The metric prices both directions: missing build/spec/artifact/install/invoke behavior starves the required numerator, while forbidden raw secret leakage, echo-only artifacts, missing provenance, committed binary blobs, off-allowlist egress, and fabricated installs halve the case score per violation class. Harness errors and unsupported profile cases score 0.

Weights in the case set approximate the lane target: 30 percent request/docs to typed tool spec, 30 percent generated artifact passes sandbox and fake API contract tests, 20 percent credential setup and injection without leaks, 10 percent actionable diagnostics, 10 percent registry/lifecycle conventions.

Bar: **0.85 on holdout**, with zero sandbox or secret-policy bypasses. Score with `harness/score.sh`. A VOID result means a constraint was violated; find and remove the violation. The harness will not tell you which constraint failed. Holdout is aggregate-only, max 3 calls per 24 hours, audit-logged. Acceptance is measured on holdout exclusively.

Small-eval warning, verbatim per portfolio COMMON: Per-feature evals are 30-60 dev + 10-15 holdout cases: far below the ~200 enumerability threshold. The compensating controls are (a) contract-style scoring (satisfying a behavioral contract usually requires the machinery, unlike data-lookup evals), (b) probe gap as the memorization gauge, (c) feedback capped to aggregate + <=5 worst case ids, (d) holdout answers off-repo.

## Constraints

- Wall-clock budget: **16 h**. Check `harness/status.sh` every cycle for elapsed time, score history, spend, holdout-call budget, and trend.
- Spend ceiling: **$30** LLM/API spend. No live third-party API calls; fake APIs only. If live model generation is added later, it must go through the portfolio live wrapper and spend ledger.
- Surface allowlist for the optimizer: `crates/ironclaw_wasm*/**`, `crates/ironclaw_extensions/**`, `crates/ironclaw_first_party_extensions/**`, `crates/ironclaw_host_runtime/**`, `crates/ironclaw_authorization/**`, new Reborn-side builder code under `crates/**`, relevant tests, `lfd/custom-build-tools/LOG.md`, and exactly one runner profile file: `tests/integration/lfd/profiles/custom_build_tools.rs`.
- Read-only during optimization: this `goal.md`, `spec.md`, `lfd/custom-build-tools/harness/**`, `lfd/custom-build-tools/eval/**`, `lfd/_shared/**`, and `tests/integration/lfd/**` except the one profile file. Eval inputs may be read where the harness exposes them; eval answers never.
- BANNED entirely: reading `lfd/custom-build-tools/harness/answers.dev.json`, anything under `$LFD_STATE_ROOT/**`, any other lane package, or any holdout input/answer file.
- Capacity caps in `harness/caps.json`: eval case-id branching in product/test code = 0; fake provider and credential trap literals in implementation diff = 0; committed `.wasm` artifacts = 0; large base64 blobs = 0; `#[ignore]` and `#[cfg(never)]` test weakening additions = 0.
- Methodology: deterministic code owns routing, validation, retries, status-code handling, auth placement, idempotency, sandbox decisions, and lifecycle state. Models may draft or extract candidate specs from unstructured docs, but must not decide permissions, secret access, egress allowlists, or whether a denial is safe.
- Generated tools must be deterministic artifacts with manifests, provenance, tests, and lifecycle state. Prompt-only skills that claim capability do not score.

## Cycle protocol

1. Score dev with `harness/score.sh --outcomes <dir>` after producing outcomes.
2. Reflect with `harness/probe.sh`; then score perturbed outcomes with `harness/score.sh --probe lfd/custom-build-tools/eval/probe/map.json --outcomes <probe-outcomes>`. If the probe gap grows, the next change must remove an eval-shaped artifact, not add one.
3. Write the LOG.md hypothesis, expected failure mode, and diagnostic before changing code.
4. Make one scoped change and run the Stage-0 tests affected by that change plus at least one generated wrapper against a fake API and one malformed or over-scoped request.
5. Log the result, including dev score before and after.
6. Checkpoint every cycle: `git commit -am "cycle <n>: <score>"`, gain or no gain.

## Entropy rules

- Rotate auth method, pagination/error shape, schema nesting, and transform family every 4 cycles.
- Stall rule: if the metric did not move last cycle, the next attempt must be a structural change. Same-knob-harder is banned.
- Exploration quota: every 5 cycles, try a structurally different approach even if the current one is still improving.
- If success comes from adding more prompt examples, the next cycle must improve typed validation, provenance, sandboxing, or fake API contract execution instead.
- If unsupported requests are accepted too often, stop adding features and tighten scope refusal.

## Cheat audit

1. Read sealed dev or holdout answers -> canary scan and banned surface; score VOIDs.
2. Hardcode fake API answers or case ids -> caps on `cbt_dev_`/`cbt_hold_` and fake-provider literals; probe renames inputs.
3. Generate echo tools that ignore provider responses -> per-case transformed IO contracts plus `artifact.echo_only` forbidden.
4. Copy bundled or uploaded WASM as the build output -> provenance state predicates and committed-binary caps.
5. Omit provenance and claim success -> `artifact.provenance.request_id` is required in every success case; missing provenance is an explicit denial class.
6. Leak raw secrets through source, logs, replies, events, or diagnostics -> `leak` forbidden matcher and raw secret trap cases.
7. Pass sandbox by granting no capability -> success cases require allowlisted egress and manifest secret binding; denial cases require no install.
8. Accept arbitrary shell/code/plugin requests -> denial contracts require actionable unsupported-scope diagnostics and no egress/install.
9. Hide failures behind generic diagnostics -> denial cases score diagnostic `class` and `actionable = true`.
10. Edit scorer, eval, goal, or answers -> read-only surface, pins, and holdout accepted only from pristine scorer/runner state.
11. Mine miss lists -> dev feedback is aggregate plus <=5 case ids, never matcher details.
12. Declare victory on dev -> stop conditions define holdout-only acceptance.

## Stop conditions

Stop when holdout is at least 0.85 with zero sandbox or secret bypasses and Stage 0 remains green; any budget is exhausted; marginal dev gain is < 0.01 for 4 consecutive cycles; generated artifacts can escape sandbox or expose secrets; the profile/scorer is found invalid and cannot be repaired in budget. On stop, write a final LOG.md report with best dev and holdout scores, what generalized, what was abandoned, and highest-leverage next steps.
