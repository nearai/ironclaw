# Reborn Self-Learning: Engineering Proposal

Date: 2026-06-24

Audience: product, engineering, security, and runtime reviewers

Status: proposal for team review

## Summary

Reborn needs a general self-learning system, not a one-off feature for learned
skills or learned memory.

The goal is to let the system improve from experience while preserving the
trust boundaries that make Reborn usable in real work: source permissions,
scope, expiration, verification, rollback, and user control.

The proposed design is a cross-cutting learning substrate that can manage
multiple kinds of learned artifacts:

- learned memory;
- learned prompt rules;
- learned skills;
- failure signatures;
- verifier evidence;
- human/admin-gated proposals for higher-risk changes.

Learning must be out-of-band. A user-facing turn may emit learning signals, but
it should not wait for diagnosis, synthesis, verification, curation, or
promotion. Those steps happen asynchronously under budget, priority, and
backpressure controls.

This system should answer one central question:

> Can this specific learning be safely used in this specific future run?

That question cannot be answered by "the model generated a skill" or "the user
approved a note." It requires source provenance, scope checks, TTL, verification,
and promotion policy.

## Memory Types At A Glance

Self-learning is not one memory bucket. The system should distinguish how
different memory classes are surfaced to the model:

- **Always-on profile context:** tiny, stable user or environment facts such as
  timezone, locale, stable communication preferences, and coarse location if
  allowed. This is loaded directly, not searched.
- **High-salience personal memory:** durable user facts or preferences that may
  be useful often, but still require ranking, budget limits, TTL/revalidation,
  and source controls.
- **Flow/skill-scoped memory:** memory loaded only when a workflow, skill,
  capability, project, repo, customer, integration, or other structured context
  is active.
- **On-demand recall:** memory fetched because the current task indicates a need
  for semantic search, hybrid keyword/vector search, or session-history recall.
- **Source-restricted memory:** memory derived from documents, conversations, or
  integrations where the source audience follows the memory. This is
  cross-cutting: source-restricted memory can also be high-salience or
  flow-scoped, but it is omitted when the current run cannot use its source.

These are surfacing policies, not separate storage backends. A memory item can
carry content, source, scope, TTL, salience, trigger hints, and intended
surfacing class, but the host still decides at use time whether that item is
safe and useful to expose.

## Why This Is Needed

The product promise is that the agent should get better over time. If it fails
once, it should be less likely to fail the same way again. If it learns a stable
preference or project convention, the user should not need to repeat it. If a
workflow can be turned into a reusable skill, the system should do that with
minimal user burden.

But self-learning is also a trust boundary. A learned artifact can affect future
prompts, future tool use, future memory retrieval, future network egress, and
future user-visible behavior. If learned artifacts are broad, stale, unverified,
or sourced from content the current conversation should not access, they become
a reliability and security risk.

The design therefore has to optimize for both:

- more automation for safe, low-risk learning;
- explicit review for high-risk or unverifiable learning.

The intended product direction is not "make users approve everything." The
intended direction is:

> Automate everything that the system can prove is safe, scoped, current, and
> helpful. Ask for approval only when the risk is high or the proof is missing.

## Proposed Model

Introduce a general self-learning lifecycle.

The lifecycle is independent of any one output surface and is not in the
latency-critical turn path:

1. A user-facing run observes evidence.
2. The run emits a bounded learning signal.
3. The user-facing turn returns.
4. A background coordinator creates candidate learnings.
5. Each candidate gets source, scope, and TTL.
6. The system classifies the risk and target surface.
7. The system verifies the candidate when verification is possible and budgeted.
8. The candidate is promoted, suppressed, expired, revoked, or sent for review.
9. Future runs use active learnings through memory, prompt context, skills, or
   proposal surfaces.
10. Scheduled curation measures impact and retires stale or harmful learnings.

The learning substrate owns the lifecycle. It does not own every downstream
system.

For example:

- memory facts are routed to the memory system;
- learned prompt rules are routed into prompt/context construction;
- learned skills are routed into the skill system;
- high-risk code/config changes remain proposals;
- fleet/shared learning remains opt-in and later-stage.

## Operational Model

Self-learning should be asynchronous by default.

The user-facing turn should do only the minimum learning work required to emit a
safe signal:

- record that a run completed, failed, or received corrective feedback;
- attach stable references to the relevant evidence;
- avoid storing raw secrets, tool inputs, host paths, or full transcripts inside
  the learning artifact itself;
- enqueue or schedule follow-up work;
- return without waiting for learning to finish.

The background path then performs the expensive or risky steps:

- reconstruct bounded evidence from authorized sources;
- deduplicate repeated failure signatures;
- classify the likely learning type;
- synthesize candidate memory, rule, skill, or proposal artifacts;
- run deterministic checks or budgeted live verification;
- promote only when policy allows;
- request review when the risk tier requires it;
- periodically curate, merge, suppress, expire, or retire stale learnings.

This protects product latency and reliability. If learning is slow, rate-limited,
budget-exhausted, or temporarily broken, the user-facing turn should still work.
The worst normal outcome is that the system learns later, not that the user waits
or the run fails.

Some signals can be prioritized without becoming blocking. For example, explicit
user correction or repeated frustration can move a learning candidate to the
front of the background queue. It still should not let the model self-certify a
fix inside the active user turn.

## Learned Artifact Types

The system should support at least the following artifact classes.

### Learned Memory

Facts, preferences, and context that should help future runs.

Examples:

- "The user is visiting London this week."
- "This workspace uses a two-step release process."
- "This customer prefers concise daily status updates."

Memory is not automatically durable forever. It needs source, scope, and TTL.

### Learned Rules

Prompt-level behavioral corrections or heuristics.

Examples:

- "When a task fails because credentials are unavailable, ask for an integration
  handoff instead of retrying the same call."
- "For this project, prefer existing composition boundaries over adding new
  direct runtime dependencies."

Rules should be compact, scoped, and budgeted. They should not accumulate
without curation.

### Learned Skills

Reusable procedures or capabilities distilled from successful work.

Examples:

- a repeatable workflow for preparing a release note;
- a workspace-specific debugging checklist;
- a procedure for validating a recurring integration.

Skills are higher-impact than simple memory. Some can auto-promote after
verification. Others require review.

### Failure Signatures

Records of recurring failure patterns.

Examples:

- same tool fails for the same missing-auth condition;
- same runtime flow repeatedly produces a malformed request;
- same class of task fails after the same decision point.

Failure signatures let the system recognize "we have seen this before" without
storing raw transcripts or sensitive inputs.

### Verifier Evidence

Proof that a candidate learning helped.

Examples:

- deterministic skill test passed;
- live model re-run no longer reproduces the failure;
- follow-up run completed without the previous failure signature;
- stateful evaluation shows improvement over baseline.

Verifier evidence is what lets the system move from manual approval to
automation.

### Human/Admin-Gated Proposals

Some learnings should not auto-apply.

Examples:

- code changes;
- config changes;
- cross-tenant or global behavior changes;
- new egress behavior;
- broad security or permission changes;
- changes sourced from content whose audience cannot be proven.

These can be suggested, summarized, and queued for review, but they should not
silently activate.

## Source, Scope, And TTL

Every learned artifact needs three things.

### Source

The system must know where the learning came from.

Examples of source categories:

- a particular conversation turn;
- a run outcome;
- a memory document;
- a capability/tool result;
- an external document or integration object.

Source is not just audit metadata. It is the foundation for permission
inheritance and revocation.

### Scope

The system must know where the learning may be used.

Scope can include:

- user;
- workspace;
- project;
- agent;
- conversation;
- document audience;
- integration account;
- organization/tenant.

At use time, the current run must be equal to or narrower than the learning's
allowed scope. If the system cannot prove that, it should omit the learning.

### TTL

The system must know how long the learning is valid.

Examples:

- "user is visiting London" might expire in two weeks;
- "user has two children" may be long-lived but still revocable;
- "this repo uses this release process" may last until the source changes;
- "this bug happened in this runtime version" may expire after the version is
  no longer relevant.

The model can suggest TTL, but the host should clamp it according to policy.

## Permission Inheritance

Learned artifacts must inherit the access limits of their source.

If a memory was extracted from a private document shared with a small group, it
should not be used in a broader conversation that includes people outside that
group.

If a source document is revoked, deleted, or replaced, downstream learned
artifacts must become unusable unless they can be revalidated against an
authorized source.

This means permission checks happen at use time, not only at extraction time.

The system should fail closed:

> If it cannot prove that a learning is allowed in the current context, it does
> not inject or use that learning.

## Promotion Policy

Learned artifacts should move through explicit states.

Suggested conceptual states:

- candidate: observed or synthesized, not used yet;
- pending review: waiting for human/admin decision;
- active: eligible for use;
- suppressed: intentionally not used;
- expired: no longer valid due to TTL;
- revoked: no longer valid due to source/access revocation;
- demoted: previously active, removed due to regression or low confidence.

Promotion should depend on risk and evidence.

Low-risk, same-scope, verifiable learnings can eventually auto-promote.

High-risk or unverifiable learnings should require review.

## Risk Tiers

### Low Risk

Likely auto-promotable after verification:

- scoped memory facts with clear source and TTL;
- narrow prompt rules for a specific project/user;
- skill improvements that pass deterministic checks and do not broaden
  capability access;
- failure signatures used only for detection, not behavior change.

### Medium Risk

May auto-promote only with stronger evidence, or may require review depending on
product posture:

- learned skills that change multi-step workflows;
- rules that affect tool ordering;
- memory facts derived from external documents;
- learnings that affect shared workspace behavior.

### High Risk

Require human/admin approval:

- global or cross-scope behavior changes;
- new network egress behavior;
- code or config changes;
- security-sensitive behavior;
- learnings sourced from restricted or ambiguous audiences;
- changes that grant, expand, or automate capability use.

## Verification Model

Replay is not enough.

Recorded replay proves that a behavior stays consistent against a fixture. It
does not prove that a new prompt rule or memory item actually improves model
behavior, because replay reuses recorded model outputs.

Different learning types need different verification:

- learned skills: deterministic contract execution where possible;
- learned rules: live model re-run or targeted behavioral check;
- learned memory: source validation plus scoped retrieval tests;
- failure signatures: detection accuracy and false-positive checks;
- high-risk proposals: human review plus targeted tests.

Live verification should be budgeted, rate-limited, observable, and off by
default until the policy is explicit.

## Product UX

The user should not experience self-learning as a pile of chores.

The default UX should be:

- quiet improvement for safe, verified, low-risk learnings;
- visible controls for pausing learning and usage;
- clear explanations when something material changes;
- review queues only for high-risk or uncertain changes;
- easy rollback/dismissal;
- honest disclosure when learning sends data to a model provider.

The review UI should not be the primary safety model. It should be the exception
path.

## System Boundaries And Responsibilities

The self-learning system owns:

- artifact lifecycle;
- source provenance;
- scope and permission checks;
- TTL and retention policy;
- risk classification;
- verification records;
- promotion/demotion policy;
- curation and retirement.

The self-learning system does not own:

- the internals of memory storage;
- the internals of skill execution;
- product-specific UI flows;
- raw transcript storage;
- external/fleet sharing by default;
- automatic code/config application.

It coordinates with those systems through stable contracts.

The boundaries look like this at a system level.

### Channel And Ingress Boundary

Channels keep doing what they already do: normalize external input into
conversation turns. They should not decide what the system learns.

The only learning-relevant responsibility at ingress is preserving enough stable
identity and source context for downstream authorization. For example, the
system needs to know which user, workspace, conversation, integration account, or
document audience a signal came from. The request body should not be trusted to
declare that scope.

### Reborn Runtime Boundary

The Reborn runtime remains responsible for the user-facing model/tool loop:
session state, approvals, tool calls, capability routing, model requests, and
turn completion.

Learning does not run inside that hot path. At completion, failure, explicit
user correction, or another meaningful milestone, the runtime emits a bounded
learning signal and returns control to the user. That signal contains stable
references to evidence, not a new pile of raw transcript copied into a learning
store.

This keeps the runtime fast and predictable. If the learning system is backed up
or disabled, normal turns continue.

### Durable Event And Evidence Boundary

The event/run history is the evidence source. Learning artifacts should point
back to durable evidence rather than duplicating sensitive content.

The background learning coordinator can reconstruct just enough context from
authorized sources to diagnose a candidate learning. That reconstruction happens
under the same scope and permission model as future use. If the evidence is no
longer authorized, the candidate cannot be promoted.

### Background Learning Boundary

The learning coordinator is the offline worker for self-improvement. It consumes
learning signals, deduplicates them, classifies the likely learning type,
synthesizes candidates, runs verification, and changes artifact state.

It is not exposed to the model as a normal tool. It has host-owned access to the
safe administrative surfaces it needs, with explicit policy around what it can
write and promote.

The coordinator is also where queueing, priority, budget, backpressure, and
retry behavior live. Repeated failures, explicit corrections, and high-severity
signals can be prioritized. Ordinary low-value signals can batch.

### Memory Boundary

Learned memory flows through the memory service, not around it.

The learning system can propose a memory item, but memory retrieval still owns
document-like behavior, indexing, chunking, and provider-specific storage. At
use time, memory candidates are filtered by source, scope, TTL, revocation, and
conversation audience before they can be included in context.

This means learned memory is not just "text we append somewhere." It is an
authorized, scoped context item with a source and lifecycle.

### Memory Surfacing Classes

Memory storage and memory surfacing are separate design problems. The memory
service can store, index, and retrieve memory, but the core agent loop needs a
host-owned policy for deciding which memory reaches the model for a specific
turn.

The first design step is to make the surfacing classes explicit. These are not
separate storage backends. They are context assembly policies that memory items
can participate in:

- **Always-on profile context:** tiny, stable user/environment facts like
  timezone, locale, stable communication preferences, and maybe coarse location
  if allowed. This should be explicit, curated, and small enough to include
  without search.
- **High-salience personal memory:** durable user facts or preferences that may
  be useful often, but still require ranking, budget limits, TTL/revalidation,
  and source controls.
- **Flow/skill-scoped memory:** memory loaded because a workflow, skill,
  capability, project, repo, customer, integration, or other structured context
  is active. This is trigger-driven, not always injected.
- **On-demand recall:** memory fetched because the current task indicates a need
  for semantic, hybrid, or session-history search. This is the vector-search-like
  path, but it is only one retrieval mode.
- **Source-restricted memory:** memory derived from documents, conversations, or
  integrations where the source audience must follow the memory. This is a
  cross-cutting constraint: even high-salience or flow-scoped memory is omitted
  if the current run cannot use its source.

Learned memory should therefore carry more than content. It should carry source,
scope, TTL, salience, trigger hints, and intended surfacing class. The host still
decides at use time whether that memory is safe and useful to expose.

### Prompt And Context Boundary

Learned rules enter future runs through the prompt/context assembly path, under
strict budget.

The model should see only the compact rule that is relevant to the current run,
not the full source evidence or private provenance details. The host decides
which active rules are applicable before prompt construction. Expired, revoked,
wrong-scope, or over-budget rules are omitted.

This is how a previous failure can influence future behavior without turning
learned context into an unbounded prompt overlay.

### Skill Boundary

Learned skills use the skill lifecycle. The learning coordinator can create or
update a skill candidate, but the skill selector should only consider learned
skills that are active, scoped, verified, and allowed in the current run.

Executable skill fixes can often be verified deterministically. Higher-risk
skills, broad skills, or skills that expand capability usage remain review-gated.

### Capability And Tool Boundary

Learning may observe tool outcomes, but it does not grant tool rights.

If a failure involved a capability, the learning artifact can reference that
capability and its failure category. It cannot bypass approval, auth, sandboxing,
or tool policy in a future run. Future usage still goes through the normal
capability boundary.

### Product And Admin Boundary

Product surfaces expose control and inspection:

- pause learning;
- pause usage of learned artifacts;
- inspect active and pending learnings;
- approve high-risk candidates;
- suppress or delete a learning;
- see why something was learned and where it came from.

The UI should talk through product-facing service contracts, not directly to raw
stores. Product controls are part of the safety model, but they are not the
primary verification mechanism.

### Model Provider Boundary

Some learning operations call a model provider: diagnosis, rule synthesis,
memory extraction, or live behavioral verification.

Those calls happen in the background, with separate budget, rate limits,
observability, and egress disclosure. They do not block the user-facing turn and
they do not silently promote their own output without policy and verification.

## How It Works End To End

### 1. A Turn Emits A Learning Signal

A normal Reborn turn completes, fails, or receives user correction. The runtime
does not try to learn inside the turn. It emits a small signal such as:

- completed run worth summarizing;
- failed run with a stable failure category;
- explicit user correction;
- repeated capability/tool failure;
- user instruction that should be remembered;
- successful repeated workflow that may become a skill.

The signal is scoped to the trusted owner of the run. It points to evidence
instead of copying everything into learning state.

### 2. The Background Coordinator Builds A Candidate

Later, a background worker consumes the signal. It reconstructs bounded evidence
from the event/run history and authorized sources.

The coordinator decides what kind of learning this might be:

- memory: a fact, preference, or project convention;
- rule: a compact behavioral correction for future prompt/context;
- skill: a reusable workflow or executable procedure;
- failure signature: a recurrence detector;
- proposal: a code/config/security/product change that should not auto-apply.

At this point the learning is still a candidate. It is not used in future turns.

### 3. The Candidate Gets Source, Scope, And TTL

Every candidate receives source provenance, use scope, and retention policy.

Source answers "where did this come from?"

Scope answers "where may this be used?"

TTL answers "how long should this remain valid?"

The model can suggest TTL and rationale, but host policy clamps it. The host also
sets the initial scope conservatively. Narrow scope can later widen only with
evidence and policy. Broad scope should not be the default.

### 4. The Candidate Is Verified Or Routed To Review

The verification path depends on the artifact type.

Memory candidates need source validation, scope validation, TTL, and retrieval
checks. A simple preference may not need a live model call.

Rules that change model behavior need stronger evidence. Replay can prove
plumbing, but it cannot prove that a changed prompt alters model behavior, so
important prompt/rule fixes need budgeted live verification.

Skills should use deterministic checks when possible. If the skill executes a
repeatable workflow and passes its contract, it may be eligible for
auto-promotion within the same scope.

High-risk proposals do not auto-promote. They go to review.

### 5. Promotion Makes The Learning Available To Future Runs

Promotion changes a candidate into an active artifact. Active does not mean
global. Active means "eligible if the current run passes use-time checks."

For a future run, the host checks:

- Is this artifact active?
- Has it expired?
- Has the source been revoked or changed?
- Is the current user/workspace/project/conversation inside the allowed scope?
- Is the current conversation audience allowed to see context derived from the
  original source?
- Is there prompt/context budget for this item?
- Has recent evidence shown that this item hurts outcomes?

Only then can it affect memory retrieval, prompt context, or skill selection.

### 6. Curation Keeps Learning From Becoming Noise

Learning quality decays unless it is curated.

Scheduled background work should:

- merge duplicate memories and rules;
- suppress contradicted or low-confidence artifacts;
- expire short-lived context;
- demote artifacts associated with regressions;
- refresh source-derived items when the source changes;
- keep learned prompt context within budget;
- measure whether stateful learning actually improves outcomes.

The goal is not to remember everything. The goal is to remember useful, scoped,
current things.

## Concrete Examples

### Example: Short-Lived User Memory

During a conversation, the user says they are visiting London for two weeks.

The turn finishes normally. The runtime emits a learning signal with a reference
to the message. In the background, the coordinator creates a memory candidate:
the user is visiting London, scoped to that user, sourced from that message, with
a short TTL.

Future travel-related runs for that user can use the memory while it is active.
After the TTL expires, it is omitted automatically. If the source conversation is
deleted or access is revoked, the memory stops being usable.

### Example: Private Document-Derived Memory

A project document says that a customer escalation requires a specific internal
process. The memory extractor proposes a project convention from that document.

The learning stores a source reference to the document and the document's
audience. In a later private project conversation with the same authorized group,
the memory can be retrieved. In a broader conversation that includes someone
outside the document audience, the memory is omitted.

The important point: the source permission follows the learning.

### Example: Repeated Tool Failure Becomes A Rule

The agent repeatedly retries a capability after receiving a missing-auth error.

Each failed run emits a bounded signal. The background coordinator deduplicates
those signals into one failure signature. It synthesizes a narrow rule: when this
capability fails due to missing auth, stop retrying and ask for the right
authorization path.

Because the rule changes model behavior, it needs verification. If a budgeted
live check shows the agent now takes the correct path, the rule can become
active for the relevant scope. Future runs receive the compact rule in prompt
context only when that capability and scope are relevant.

### Example: Successful Workflow Becomes A Skill

The user repeatedly asks the agent to perform the same release-note preparation
workflow. The workflow succeeds several times with similar steps.

The background coordinator proposes a learned skill. The skill starts as a
candidate. If it can be checked deterministically and does not broaden capability
access, it may auto-promote for that workspace. If it sends data to new services,
changes permissions, or has broad effects, it stays pending review.

Once active, the skill selector can use it for matching future tasks in the same
scope. If later evidence shows it causes regressions, the learning system demotes
it.

### Example: Code Or Config Suggestion Stays A Proposal

A failed run suggests that changing runtime configuration or code would prevent
future failures.

The learning system can capture the failure signature and summarize the proposed
fix, but it should not apply the code/config change automatically. That artifact
is routed as a proposal for human review. The system can still use the failure
signature to recognize recurrence while the proposal waits.

## Defaults And Safety Posture

Recommended initial defaults:

- learning substrate available behind explicit enablement;
- no default-on extraction until egress/product posture is approved;
- no automatic promotion without verification;
- source/scope/TTL required for learned artifacts;
- high-risk changes require review;
- learning work is asynchronous and best-effort relative to the user-facing
  turn;
- raw transcripts, tool inputs, secrets, and host paths are not stored in learned
  artifacts.

The long-term goal is more automation, not more approval prompts. But automation
should be earned by evidence.

## Recommendation

The next architectural step should be the general self-learning substrate:

- artifact lifecycle;
- source provenance;
- permission inheritance;
- TTL;
- scoped use-time authorization;
- out-of-band background processing;
- risk-tiered promotion;
- verification;
- retirement.

Once that exists, learned memory, learned rules, and learned skills can all plug
into the same safety model.

That is the deeper fix: not "approve generated skills," but "make the agent
capable of safely learning from experience across every surface where learning
matters."
