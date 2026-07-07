# Goal: let tools use configured service credentials without exposing secrets to the model

Source page: https://app.notion.com/p/36f29a6526bf806bac54f6dc3634ac13

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` for mediated credential use by skills and tools. Preserve the invariant that the agent can verify a credential exists and request mediated use, but cannot read secret material or smuggle registered auth headers through normal prompt-controlled arguments.

The spec must include:

- Channel-origin setup, including a Slack DM example like "configure Crisp".
- WebUI-origin setup.
- Credential mapping to allowed hosts or tools.
- Runtime injection into HTTP/tool execution.
- Audit, approval, expiration, denial, and redaction behavior.
- Negative cases for manual `Authorization`, wrong host, missing mapping, and expired lease.

## Target (outer loop)

Optimize credentialed tool success with secret-safety penalties:

- 40% correct setup flow from Slack/WebUI without LLM-visible secret content.
- 35% HTTP/tool call succeeds through mediated credential injection.
- 15% approval and audit event correctness.
- 10% negative cases deny safely and explain the fix without leaking data.

Bar: at least 0.94 holdout, zero secret leaks, zero manual auth-header bypasses.

## Eval design

Create 60 dev and 140 holdout fake-provider tasks across Crisp-like, Stripe-like, GitHub-like, and custom API shapes. Include bearer token, basic auth, custom header, query-token rejection, OAuth-like expired lease, wrong host, and multi-tenant credential collision cases.

Answers include setup state, secret visibility expectations, injected request headers seen by the fake provider, transcript redactions, approval events, and denial reasons.

## Harness design

Drive fake channel messages and WebUI setup through product workflow, then invoke the generic HTTP/tool path through host-runtime credential mediation. `harness/score.sh` must:

- Assert fake external servers see the expected credential only on allowed requests.
- Assert model transcript, tool arguments, logs, and events never contain the secret value.
- Assert manual registered auth headers are denied or rewritten only by trusted mediation.
- Score both correct injection and correct refusal.
- Calibrate with a known-bad implementation that serializes secrets into the transcript and confirm it fails.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $15 LLM/API spend; no live third-party credential use.
- Surface allowlist: `crates/ironclaw_host_runtime/src/egress/credential.rs`, `crates/ironclaw_host_runtime/src/obligations.rs`, `crates/ironclaw_authorization`, `crates/ironclaw_secrets`, product workflow setup surfaces, existing `tests/skill_credential_injection.rs`, `tests/integration/secret_injection.rs`, and related docs.
- Capacity caps: shared caps; fake provider credential fixtures capped at 20 and must never be real secrets.
- Do not add a Crisp-only product path. Crisp is one eval family, not the abstraction.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include one success case and one denial case. Before committing, run a transcript/log scan for fake secret canaries and ensure all detections remain outside model-visible surfaces.

## Entropy rules

- Rotate auth shapes every 3 cycles: bearer, basic, custom header, OAuth-like lease, and denied query token.
- If success rises while negative cases fall, stop feature work and tighten policy/lint before continuing.
- If the same provider family accounts for two gains in a row, the next cycle must use a different fake provider shape.

## Cheat audit

Lane-specific cheap wins to block:

1. Put the secret in model context; canary scan fails.
2. Allow user-supplied `Authorization` headers; negative cases and lint fail.
3. Hardcode Crisp host behavior; probe swaps provider and auth shape.
4. Inject credentials for every request; wrong-host cases fail.
5. Skip audit events; scorer requires setup, injection, denial, and approval audit.
6. Treat credential existence as permission; authorization decision is scored separately.
7. Store fake secrets in committed fixtures; lint scans known canary values.
8. Return secret values in errors; transcript/log scan fails.
9. Make WebUI setup work but Slack setup fail; eval includes both origins.
10. Hide failures by returning unauthenticated success; fake provider validates received headers.

## Stop conditions

Stop when holdout is at least 0.94 with zero secret leaks and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any real secret or credential canary appears in model-visible output.

