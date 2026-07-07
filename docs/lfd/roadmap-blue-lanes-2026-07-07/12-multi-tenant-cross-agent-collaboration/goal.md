# Goal: enable explicit cross-agent collaboration without leaking private context

Source page: https://app.notion.com/p/36e29a6526bf8039b69fc48e16d5cbb7

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Threat model first. Write `spec.md` before implementation with collaboration envelope, authority passthrough, explicit invitation/request/response flow, data-flow labels, admin visibility, revocation, expiration, and TEE boundary assumptions.

The spec must state that agents owned by different users collaborate only through explicit, auditable channels. Private memory, credentials, and tool authority do not cross by default.

Stage 0 tests must include same-org allowed collaboration, denied collaboration, revoked collaboration, same user id across tenants, malicious prompt content, and admin audit visibility.

## Target (outer loop)

Optimize collaboration correctness and isolation:

- 30% explicit invitation/request/response workflow works.
- 25% tenant, user, project, and authorization scope are enforced at every hop.
- 20% only approved context crosses the boundary.
- 15% audit and admin visibility are complete without exposing private payloads.
- 10% revoked, expired, denied, or failed collaboration recovers safely.

Bar: at least 0.92 holdout, zero isolation failures, zero private memory or credential leakage.

## Eval design

Create 80 dev and 180 holdout multi-user/org scenarios. Inputs include two agents, two users, shared org, separate tenants, shared project, revoked grants, expired grants, delegated tasks, prompt injection from collaborator, private memory requests, credentialed tool requests, and admin audit queries.

Answers include allowed/denied decision, data-flow labels, emitted context, omitted private context, credential subject, audit events, and final collaboration state.

## Harness design

Drive fake agents/users through product workflow, conversations, authorization, subagent/capability paths, and event/audit stores. The scorer must inspect data-flow labels and emitted context, not only final response text.

`harness/score.sh` must:

- Create isolated tenants/users/agents with overlapping ids.
- Initiate collaboration through the allowed product workflow.
- Attempt private memory and credential boundary violations.
- Assert only approved context is emitted.
- Assert audit/admin views contain metadata without private payload leaks.

## Constraints

- Wall-clock budget: 16 hours.
- Spend ceilings: $20 LLM/API spend; no live external collaboration surfaces.
- Surface allowlist: `ironclaw_product_context`, `ironclaw_authorization`, `ironclaw_conversations`, `ironclaw_product_workflow`, `ironclaw_loop_support` subagent spawn paths, events/audit, tests, and docs.
- Capacity caps: shared caps; collaboration policy examples capped at 12.
- Product adapters and product workflow must not mint `TrustedInboundTurnRequest` or trusted trigger submitters.

## Cycle protocol

Follow the common cycle protocol. Each cycle must pair one allowed collaboration with one denied or leakage-attempt case. Any change that increases context sharing must run all cross-tenant probes before commit.

## Entropy rules

- Rotate scopes every 3 cycles: same org, shared project, cross-tenant, revoked/expired, and admin audit.
- If allowed workflows pass but denial cases fail, all cycles target isolation until zero failures.
- If audit visibility grows, the next cycle must include a payload-redaction probe.

## Cheat audit

Lane-specific cheap wins to block:

1. Use shared global memory; cross-tenant memory probes fail.
2. Pool credentials between agents; credential subject is scored.
3. Treat admin visibility as enforcement; denied side effects are scored.
4. Mint trusted inbound from adapters; lint rejects forbidden construction.
5. Pass whole transcript as context; context diff and token cap fail.
6. Ignore revocation or expiration; time-shifted probes fail.
7. Let prompt injection request private memory; boundary probes fail.
8. Hardcode org ids or user pairs; probe swaps ids and tenants.
9. Hide leaks in audit views; admin view is scanned for private payload.
10. Skip collaboration state persistence; restart and continuation cases fail.

## Stop conditions

Stop when holdout is at least 0.92 with zero isolation failures and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any private memory, credential, or unauthorized tool authority crosses the collaboration boundary.

