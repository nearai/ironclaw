# Goal: tools and skills use configured service credentials without exposing secrets

Use profile `secrets_skills_tools`.

## Stage 0 - Build to spec

Implement `spec.md`. Do not score until Stage 0 is green. Required checks: `cargo fmt`, clippy, `cargo test -p ironclaw_secrets`, `cargo test -p ironclaw_authorization`, `cargo test -p ironclaw_host_runtime`, `cargo test --test skill_credential_injection`, `cargo test --features integration --test secret_injection`, lease-expiry coverage, and `tests/integration/lfd/profiles/secrets_skills_tools.rs` returning `status: "ran"` for every visible dev case.

## Target

Score credentialed setup, authorization, one-shot lease/staging, mediated injection, egress capture, redaction, expiry resume, and revocation. Missing required evidence lowers recall. Forbidden leaks, raw secret egress, stale-cache use, manual auth-header bypass, wrong-host egress, and route-by-credential-name setup lower precision.

Bar: 0.95 on holdout, zero secret leaks, zero manual auth-header bypasses. Score with `harness/score.sh`; VOID means a constraint violation. Holdout is aggregate-only, max 3 calls per 24h, and acceptance is holdout-only.

Small-eval warning: this is a 10 dev + 3 holdout minimal slice for verification, below the portfolio target and far below the ~200 enumerability threshold. Expand before a long optimization run.

## Constraints

- Wall-clock budget: 12 h.
- Spend ceiling: $15; no real third-party credentials.
- Read/write surface for optimization: `crates/**`, relevant `src/**` maintenance only, `tests/**`, `lfd/secrets-skills-tools/LOG.md`, and `tests/integration/lfd/profiles/secrets_skills_tools.rs`.
- Read-only: this goal, `spec.md`, `harness/**`, `eval/**`, `lfd/_shared/**`, and shared integration support.
- Banned: reading `harness/answers.dev.json`, `$LFD_STATE_ROOT/**`, off-repo holdout answers, or any other lane package.
- Caps: no case-id branching, no fake secret literals in product/test code, no new production `SecretStoreLease` shortcuts, no new `unwrap()` in `ironclaw_secrets`, and no test weakening.

## Cycle Protocol

Run dev score, run probe, log hypothesis/expected failure/diagnostic before changing code, change one scoped seam, run relevant Stage-0 checks, log result, then commit each cycle.

## Entropy Rules

Rotate auth shapes every 3 cycles. If success improves while denial cases regress, stop and tighten policy. If probe gap grows, remove eval-shaped artifacts instead of adding them. Every 5 cycles try a structurally different approach.

## Stop Conditions

Stop when holdout reaches 0.95 with Stage 0 green and zero leaks/bypasses, any budget is exhausted, marginal dev gain is <0.01 for 4 cycles, a fake or real secret appears in model-visible output, or the scorer is invalid.
