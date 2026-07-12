# Q-10 Slack Canary Reliability Design

## Objective

Make the Q-10 Slack suite a trustworthy evaluation of whether the live model can
discover and use the intended Slack capabilities, interpret their outputs, and
produce correct user-facing answers. A red result must identify a product/tool
contract problem, a model-quality problem, an infrastructure incident, or an
invalid fixture instead of collapsing every condition into the same flaky shard.

The change must not make the suite green by retrying failed answers or weakening
the audited user journeys.

## Current failure modes

Q-10 currently runs nine one-shot live-model cases in one all-or-nothing shard.
The historically unstable cases combine unrelated variables:

- 10D tests extension discovery and channel-membership reasoning in one turn.
- 10G asks a workspace-global freshness question while Slack search is eventually
  consistent and other canary shards write through the same personal account.
- 10I exposes raw identifiers needed for later tool calls and relies only on
  model instructions to keep those identifiers out of the final reply.
- A provider outage becomes nine marker timeouts rather than one infrastructure
  incident.
- A finalized reply with a mutated synthetic marker waits for the full chat
  timeout even though the UI has already declared the turn terminal.

The deterministic Python tests mostly pin source strings and helper behavior;
they do not drive these caller paths or classify their outcomes.

## Design principles

1. Preserve the real-model evaluation. No whole-case retry may turn a failed
   first answer into an unqualified pass.
2. Test one responsibility per journey. Extension lifecycle, Slack capability
   correctness, and model answer quality must be attributable separately.
3. Protect the user independently of model quality. A safety backstop may
   prevent a raw identifier from reaching the user, but the behavioral result
   must still report that the model needed intervention.
4. Use final-turn state for liveness and seeded/authoritative data for
   correctness.
5. Treat live LLM tests as supplemental quality evidence; deterministic product
   contracts remain the blocking regression layer.

## Architecture

### 1. Typed case tier and failure classification

Extend the live-QA case metadata and result payload with:

- `tier`: `contract` or `behavioral`.
- `failure_class`: `product`, `model_quality`, `infrastructure`,
  `precondition`, or `none`.
- `expected_capabilities`: the capability IDs that prove the intended caller
  path was exercised.

Contract failures retain blocking semantics. Behavioral failures remain visible
in the manifest, JSON, Markdown summary, workflow annotations, and Slack
notification, but do not masquerade as deterministic code regressions.
Infrastructure and precondition failures remain non-successful and are rendered
as their own incident classes. For exact-PR validation, infrastructure failures
are inconclusive and must be rerun; they are never counted as product passes.

### 2. Separate extension lifecycle from Slack correctness

The existing extension/connect journeys continue to validate discovery,
installation, activation, and authentication.

Q-10 Slack correctness cases establish the Slack extension prerequisite through
the same production UI lifecycle helper before submitting their natural-language
prompt. This removes extension discovery as an uncontrolled variable from
membership, history, status, thread, and entity assertions.

Each correctness case that depends on a specific capability records the actual
capability run statuses and fails if the expected capability did not complete.
A lucky or fabricated final answer is not a pass.

### 3. Journey-specific changes

#### 10D: channel membership

- Keep the natural user question and authoritative `users.conversations`
  ground truth.
- Start with Slack active.
- Require a completed `slack.list_conversations` invocation.
- Preserve the positive member and negative non-member assertions.
- Clarify the model-visible tool contracts so `is_member` is authoritative and
  outbound delivery targets explicitly do not read Slack membership or content.

#### 10G: most recently sent message

Split the current ambiguous journey:

- The contract journey asks for the newest self-authored message in the seeded
  conversation. It requires `slack.get_conversation_history` (and allows
  `slack.whoami`) and verifies the exact seeded nonce.
- The workspace-global "most recent message I sent anywhere" journey remains a
  live behavioral evaluation. It preserves the real product question and
  reports model/search/shared-state quality without blocking deterministic
  regression validation.

The Slack search capability description must carry the freshness warning in the
actual model-visible description, not only in its prompt document.

#### 10I: raw entity hygiene

Keep raw Slack identifiers in tool results and capability-call arguments because
history chaining, user lookup, conversation lookup, and encoded mentions require
them. Continue enriching the same results with display names and
`is_current_user`.

Add a product-live model-output decorator owned by Reborn composition. When the
Slack capability surface is active, it sanitizes Slack-shaped identifiers only
in streamed assistant text, reasoning text exposed to the UI, and finalized
assistant replies. It must not alter `ParentLoopOutput::CapabilityCalls`, tool
arguments, stored capability results, or Slack message bodies sent intentionally
through the Slack capability.

The replacement is an explicit bounded marker such as
`[Slack identifier redacted]`. The live 10I behavioral case fails if either a
raw identifier or this intervention marker appears, and still requires the
resolved display name. Therefore:

- users never receive the raw identifier;
- the model does not receive an undeserved pass when the backstop intervenes;
- tool chaining and mention encoding remain intact.

The decorator is scoped to product-live assistant output and is not added to the
neutral `ironclaw_turns` contracts, which must not parse product-specific Slack
identity.

### 4. Terminal-state and provider-error handling

Expose structured failure category/status attributes on WebUI error messages.
The harness waits for one of two terminal conditions:

- a finalized assistant reply; or
- a structured terminal run error.

A finalized reply is returned immediately for content evaluation even when the
synthetic answer marker is absent or reformatted. Marker presence remains a
diagnostic/content assertion where useful, not a liveness primitive.

A terminal provider failure becomes one typed infrastructure result. The runner
short-circuits the remaining cases for provider-unavailable/provider-transient
incidents and writes explicit inconclusive results instead of spending nine
240-second timeouts.

### 5. Model-visible tool-contract corrections

Update and pin these descriptions:

- `builtin.outbound_delivery_targets_list` is only for routing final replies and
  trigger/routine results; it cannot read Slack conversations, messages,
  membership, status, or profiles.
- `slack.search_messages` is indexed search and must not be used to determine
  the single newest message when conversation history is available.
- `slack.list_conversations` returns visible conversations, not only membership;
  `is_member` is the membership source of truth.
- Slack read tools tell the model to use humanized message text,
  `user_display_name`, and `is_current_user` in prose, while retaining raw IDs
  only for subsequent tool calls.

Prompt documents and manifest descriptions must agree.

## Result flow

1. The harness establishes case prerequisites and records their status.
2. It submits the natural user prompt through the real WebUI route.
3. The model selects capabilities from the real surface.
4. The harness observes the terminal reply or typed terminal error.
5. It verifies expected capability execution before evaluating answer content.
6. It assigns the case tier and failure class.
7. The runner aggregates blocking contract failures separately from behavioral
   quality failures and infrastructure/precondition incidents.
8. Artifacts and notifications render all categories explicitly.

## Testing strategy

All behavior changes follow red-green-refactor.

### Harness caller tests

Replace or augment source-string pins with direct tests that drive:

- Slack prerequisite activation through the shared chat caller;
- 10D success/failure based on completed `slack.list_conversations` calls;
- scoped versus global 10G classification;
- 10I raw-ID and sanitizer-intervention detection;
- finalized replies whose synthetic marker was reformatted;
- structured provider errors and remaining-case short-circuiting;
- result aggregation for contract, behavioral, infrastructure, and precondition
  outcomes.

### Product and UI contracts

- Frontend component tests pin structured failure attributes.
- Composition/runner caller tests prove streamed and finalized assistant text is
  sanitized while capability-call arguments remain byte-for-byte unchanged.
- The real bundled Slack WASM contract continues to prove that display names and
  raw IDs coexist in capability output for chaining.
- Tool-surface tests pin the corrected descriptions through the actual catalog
  and caller-visible tool definitions.

### Recorded behavior

Add scrubbed Q-10 fixtures and assertions for the tool-choice/request-shape
contracts that can be replayed hermetically:

- membership uses `slack.list_conversations`;
- scoped recent-message retrieval uses conversation history rather than indexed
  search;
- entity output uses display names, and capability-call IDs remain usable.

Fixtures must pass `scripts/ci/check-reborn-qa-fixtures.sh` and must not contain
live Slack identifiers, names, secrets, or other PII.

## Validation and PR readiness

Local validation includes the targeted Python harness suite, frontend component
tests, relevant Rust crate/caller contracts, recorded behavior replay, formatting,
Clippy, and repository boundary checks.

The PR opens as draft after deterministic checks pass. Live validation must use
the exact PR head with `use_target_harness=true`; the canonical-main harness is
not sufficient for a harness-changing PR.

Ready-for-review requires:

1. deterministic local checks green;
2. PR CI green for the exact head;
3. at least three consecutive exact-head Q-10 contract runs passing;
4. no provider-unavailable run counted as a pass or a failure of the fix;
5. artifact traces confirming the intended Slack capabilities actually ran;
6. behavioral results reported without retries or hidden first-attempt failures.

## Non-goals

- Making every live-model answer deterministic.
- Removing raw IDs from Slack capability results or tool arguments.
- Adding blanket whole-case retries or increasing the 240-second timeout.
- Building a new cross-workspace Slack search/index service.
- Reworking unrelated QA shards or Slack delivery behavior.

## Rollback and compatibility

The result schema additions are backward-compatible optional fields for artifact
consumers. Existing case identifiers remain stable where possible; the new
workspace-global 10G behavioral case receives its own identifier.

The assistant-output guard is limited to product-live Slack-aware output. If it
causes false positives, it can be disabled independently without removing the
case classification, terminal-state, or tool-contract improvements.
