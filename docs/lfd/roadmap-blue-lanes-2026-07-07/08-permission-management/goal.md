# Goal: manage agent permissions through natural language without giving model text authority

Source page: https://app.notion.com/p/36e29a6526bf8042b417f0f2f7da5c0c

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

The roadmap page is sparse. Write `spec.md` before implementation. The spec must define a closed command model for permission changes. LLMs may classify intent, but deterministic policy code must apply and persist permission changes.

The spec must cover:

- Grant, revoke, deny, temporary approval, and view-current-permissions.
- Ambiguous scope requiring clarification.
- Broad or risky grant requiring confirmation.
- Cross-tenant and cross-user attempts denied.
- Prompt-injection content treated as untrusted.
- Rollback or undo affordance.

## Target (outer loop)

Optimize permission-change correctness:

- 35% intended permission delta is extracted into a typed request and confirmed when required.
- 25% deterministic policy decision is correct.
- 20% resulting capability access changes as expected when invoked.
- 10% ambiguous or risky requests are clarified or denied.
- 10% audit trail, explanation, and rollback affordance are correct.

Bar: at least 0.93 holdout, zero unauthorized grants, zero model-text direct mutations.

## Eval design

Create 100 dev and 200 holdout natural-language permission requests. Inputs include channel messages, WebUI turns, grant/revoke/deny, temporary approvals, "let it use X once", ambiguous tool names, broad "give all access", cross-tenant asks, malicious prompt-injection, and user corrections.

Answers include typed permission request, confirmation requirement, policy decision, persisted delta, effective invocation outcome, audit event, and user-facing response class.

## Harness design

Feed requests through channel or product workflow to a fake permission store, then invoke representative tools to assert effective access. The scorer must not accept parser-only success.

`harness/score.sh` must:

- Run classifier/extractor under deterministic schema validation.
- Apply policy through authorization code.
- Persist to a fake or hermetic permission store.
- Invoke affected tools before and after the change.
- Assert audit and rollback metadata.

## Constraints

- Wall-clock budget: 12 hours.
- Spend ceilings: $20 LLM/API spend for classifier development/eval only.
- Surface allowlist: `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_capabilities`, product workflow, channel ingress, permission storage, tests, and docs.
- Capacity caps: shared caps; natural-language pattern lists capped at 25 and cannot encode eval requests.
- Model output is never authority. It must be parsed into typed data and validated by deterministic policy.

## Cycle protocol

Follow the common cycle protocol. Each cycle must include a positive permission change, a denial or clarification, and an invocation that proves effective access changed or did not change.

## Entropy rules

- Every fourth cycle must use a different language style or channel source.
- If extraction improves while policy errors remain, all further cycles target deterministic policy until unauthorized grants are zero.
- If prompt-injection cases fail, do not add more examples; add a stricter boundary or typed validation.

## Cheat audit

Lane-specific cheap wins to block:

1. Map every permission request to allow; unauthorized-grant hard gate fails.
2. Let model text mutate permissions directly; lint rejects direct mutation path.
3. Skip invocation after update; scorer verifies effective access.
4. Treat ambiguous requests as broad grants; clarification cases fail.
5. Ignore temporary permission expiry; holdout includes time-shifted checks.
6. Store permissions in prompt-visible memory only; persistence and policy checks fail.
7. Hardcode tool names; probe swaps tool names and ids.
8. Collapse user and admin authority; cross-user/admin cases fail.
9. Drop audit to avoid conflicts; audit score fails.
10. Confirm broad grants with generic "ok"; scorer requires typed confirmation target.

## Stop conditions

Stop when holdout is at least 0.93 with zero unauthorized grants and Stage 0 tests green, budget is exhausted, score is flat for 3 cycles, or any prompt can bypass deterministic policy validation.

