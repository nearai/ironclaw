# Live Canary Signal Reliability Design

## Objective

Make the canary system answer three separate questions without conflating them:

1. Did every deterministic product contract pass?
2. Did live providers, OAuth flows, and external side effects remain available?
3. Did the live model produce the desired user-facing behavior?

A scheduled run must be able to report all three signals without presenting an
advisory model-quality miss as a product regression or counting a mocked test as
a live canary. The change preserves the real live journeys and their first
attempts; it does not make the suite green by retrying model answers or hiding
failed observations.

## Current problems

The scheduled Reborn WebUI v2 suite executes 47 cases every three hours. Its
report combines blocking contract cases and nonblocking behavioral cases in one
`passed / total` fraction. A run can therefore be GitHub-green while appearing
incomplete, such as 45/47, even when all 44 blocking contracts passed.

The recent failures also expose four test-design problems:

- The product process runs with a workspace below the repository artifact
  directory. Local tools can traverse into the checkout and inspect the canary
  source, logs, and oracle artifacts. Several model turns did exactly that and
  timed out in a self-referential investigation.
- The workspace-global Slack last-message case seeds a message and immediately
  relies on eventually consistent indexed search. A stale Slack index is
  currently reported as a product answer mismatch.
- Several routine cases require exact synthetic markers or presentation words
  after durable trigger creation has already succeeded. Correct synonyms or a
  reformatted marker can override authoritative side-effect evidence.
- The `Live Canary` workflow also contains deterministic replay, mock-backed
  auth, mock-backed workflow, and local upgrade compatibility lanes. These are
  useful tests, but their placement obscures which jobs actually detect live
  dependency drift.

## Design principles

1. Contract health is authoritative. Behavioral quality and infrastructure
   observations remain visible but use separate denominators and statuses.
2. Live cases must exercise an uncontrolled production-like dependency that a
   hermetic test cannot reproduce: a real model, external provider, OAuth
   consent flow, external side effect, or provider-issued readback.
3. Deterministic contracts remain blocking in ordinary CI. Live LLM tests are
   supplemental and never replace recorded, scripted-provider, loopback, or
   browser contract coverage.
4. The system under test cannot read the harness, expected answers, reports, or
   its own canary logs.
5. Durable or provider-issued evidence outranks final-answer wording for
   side-effecting success. User-facing prose remains an independently reported
   behavioral assertion where it matters.
6. No whole-case model retry can convert a failed first answer into a pass.

## Result model and reporting

Keep the existing case tiers and failure classes, but make them first-class in
aggregation and presentation.

Each structured result is classified as one of:

- `contract_pass` or `contract_failure`
- `behavioral_pass` or `behavioral_warning`
- `infrastructure_inconclusive`
- `precondition_inconclusive`

The reporter renders independent totals. For example:

```text
Contracts: 44/44 passed
Behavioral quality: 1/3 passed, 2 warnings
Infrastructure: 0 inconclusive
```

The lane status is `fail` only when a blocking contract fails. Behavioral
warnings render amber without reducing the contract numerator. Infrastructure
or precondition incidents render inconclusive and never count as a product pass
or failure. The existing combined execution total may remain as secondary
diagnostic information, but it is not the headline health score.

Backward compatibility is preserved for older `results.json` files: missing
tier metadata continues to fail closed as a blocking contract. Existing fields
remain readable; any aggregate fields added to reports are optional.

## Isolated live-agent workspace

The Reborn server receives an explicitly allocated temporary workspace outside
the repository checkout and outside the artifact tree. The harness retains its
logs, traces, screenshots, and results under `artifacts/live-canary`, but that
directory is not the product process working directory or an allowed local-tool
root.

The harness owns the temporary workspace lifetime:

1. Allocate a unique directory through the platform temporary-directory API.
2. Start the Reborn server with that directory as its working directory.
3. Run the selected cases.
4. Stop the process before deleting the workspace.

Tests assert that the selected workspace is not below the repository root or
the output directory. The server startup helper accepts the workspace path
explicitly so tests can verify the boundary without launching the full live
suite.

This isolation is a validity requirement, not a security sandbox. Production
authorization and filesystem mediation continue to enforce their own policies.

## Slack workspace-global freshness case

Split provider freshness from model behavior inside
`qa_10g_slack_last_message_sent_global`:

1. Seed the unique message through the personal Slack token.
2. Poll the same indexed-search surface that the product can use until the
   nonce is visible or the freshness deadline expires.
3. Record `slack_index_latency_ms`, attempts, and the configured freshness SLO.
4. If the message is not indexed by the deadline, return a nonblocking
   infrastructure/inconclusive observation and do not ask the model an
   unanswerable question.
5. Once indexed, ask the model the original workspace-global question exactly
   once. A wrong answer is a behavioral/model-quality warning.

The conversation-scoped QA10G contract remains blocking. It requires
`slack.get_conversation_history`, verifies the seeded nonce, and keeps its
recorded fixture that forbids indexed search.

## Evidence-based routine and integration assertions

Routine creation helpers separate three assertions:

- Contract: the expected capability completed with the required arguments.
- Contract: the durable trigger exists with the required schedule, action, and
  delivery target.
- Behavioral: the final response explains the result clearly.

Synthetic markers remain useful for correlation and external readback, but a
durably verified trigger is not failed merely because the assistant reformats a
creation marker or says `trigger` instead of `routine`. Delivery cases still
require provider-issued success plus independent readback in the intended
destination. Wrong-channel, duplicate, missing, or unsuccessful deliveries
remain blocking failures.

Broad forbidden substrings such as `.md` are replaced with evidence about the
actual capability path and returned artifact type. Error phrases that directly
indicate authentication or authorization failure remain valid negative
assertions.

Slack digest and entity-hygiene checks remain live behavioral observations.
Their deterministic invariants receive hermetic coverage at the capability and
recorded-model seams: display names accompany chaining identifiers, the model
uses the intended lookup sequence, and final synthetic replies contain no raw
Slack identifiers.

## Connection journey consolidation

The suite retains one scheduled chat-driven connection contract per integration
surface: Gmail, Google Calendar, Google Drive, GitHub, Google Sheets, and Slack.
Repeated rows that exist only to restate the same connection journey are removed
from the scheduled selection, not deleted from the case registry.

Cases that need an active extension continue to establish their prerequisite
through a shared setup helper in their own isolated shard. Setup is not reported
as another connection test unless the connection journey itself is the case
under evaluation. If live-code inspection shows that a repeated connect case
also establishes unique state that cannot be reproduced by the shared helper,
that case stays scheduled and the manifest documents the distinction.

## Workflow tier ownership

The `Live Canary` workflow keeps only lanes that exercise live dependencies:

- `public-smoke`
- `persona-rotating`
- `private-oauth`
- `provider-matrix`
- `release-public-full`
- `auth-live-seeded`
- `auth-browser-consent`
- `reborn-webui-v2-live-qa`

The other lanes move without losing coverage:

- `deterministic-replay` is removed from `Live Canary`; the existing
  `replay-gate.yml` remains its CI owner.
- `auth-smoke`, `auth-full`, and `auth-channels` become mock-backed Reborn CI
  jobs. They retain fresh-machine and Playwright coverage.
- `workflow-canary` becomes a hermetic whole-path Reborn CI job. Its mock LLM
  and remapped Telegram, Sheets, Calendar, Gmail, Hacker News, and web-search
  services remain unchanged.
- `upgrade-canary` moves to a dedicated manually dispatched upgrade
  compatibility workflow because it compares release database behavior rather
  than live-provider drift.

The shell dispatcher may keep stable lane names for local developer use, but
operator documentation and GitHub workflow choices identify their actual tier.
The live canary reporter no longer waits for jobs owned by other workflows.

## Persona lane truthfulness

The persona lane remains a valid live-LLM canary even when third-party
integration credentials are absent, but it must report which integrations were
real and which used dummy fallbacks. A persona result must not claim external
provider coverage unless provider-issued evidence was observed.

## Testing strategy

Implementation follows red-green-refactor.

### Reporter tests

- All contracts pass and two behavioral cases fail: contract headline remains
  100%, lane is warning, and behavioral totals show two warnings.
- A blocking contract failure makes the lane fail.
- Infrastructure and precondition results are inconclusive, not warnings or
  product failures.
- Older untyped results fail closed.

### Harness tests

- The live workspace allocator returns a path outside the checkout and output
  tree.
- Server startup uses the explicit isolated workspace.
- Workspace cleanup occurs only after process shutdown.
- Finalized replies with reformatted creation markers can pass when durable
  trigger evidence is correct.
- Missing, mismatched, or duplicate durable evidence still fails.

### Slack tests

- Global search polling records indexing latency when the nonce appears.
- Search timeout produces infrastructure/inconclusive without invoking chat.
- An indexed nonce followed by a wrong model answer is a behavioral warning.
- The scoped history contract continues to require conversation history.
- Digest and entity synthetic fixtures reject raw identifiers in final prose.

### Workflow tests

- Workflow syntax and lane-choice contracts prove non-live jobs no longer live
  in `live-canary.yml`.
- Reborn CI owns the mock auth and workflow jobs.
- The upgrade workflow exposes the previous/current ref inputs and invokes the
  unchanged compatibility runner.
- Canary reporting needs only live jobs.

Targeted Python tests, recorded-behavior Rust tests, workflow validation,
formatting, repository safety checks, and the narrowest affected Clippy/test
commands run before publishing the PR. Live canaries are not required to prove
the deterministic refactor locally, but the PR description includes an exact-
head live validation plan.

## Documentation and operator impact

Update the live-canary README and internal operator documentation to define the
three signal classes, list the live-only lanes, point to the new hermetic and
upgrade workflow owners, and explain why a behavioral warning does not reduce
contract health.

The Slack message becomes more actionable: maintainers can distinguish a code
regression, a live-provider incident, and a model-quality observation without
opening every shard artifact.

## Rollout and rollback

The PR ships as one cohesive change because reporter semantics, case
classification, workspace validity, and workflow ownership must agree.

Rollback is independently safe by layer:

- Reporter changes can be reverted while preserving optional result metadata.
- Workspace isolation can be reverted without changing stored Reborn state.
- The global Slack case can be disabled independently because it is
  nonblocking.
- Moved CI jobs retain the same underlying runner entrypoints and can be moved
  back without changing test implementation.

No persistence schema, production API, authentication policy, or runtime
authorization behavior changes.

## Non-goals

- Making live-model prose deterministic.
- Retrying a failed model answer until it passes.
- Weakening wrong-destination, duplicate-delivery, authorization, or
  provider-readback assertions.
- Removing raw Slack IDs from capability arguments needed for chaining.
- Treating live canaries as PR-blocking substitutes for deterministic tests.
- Refactoring unrelated Reborn product or runtime architecture.
