# Tool discovery implementation decisions

This log records subjective calls made while implementing issue #7405 so they
can be reviewed independently from the code. It is intentionally explicit
about alternatives and should be updated whenever implementation evidence
changes one of these choices.

## D1 — Namespace identity comes from `CapabilityId`

**Decision:** Treat the first segment of the typed, extension-prefixed
`CapabilityId` as the authoritative discovery namespace. Never derive a
namespace from the provider-facing tool name or description.

**Why:** The `CapabilityId` contract already requires
`<extension>.<capability>[.<sub>...]`, and extension registration constructs
capability IDs from the package/extension identity. Provider names are encoded
for model APIs and are not an ownership boundary.

**Alternative considered:** Add a second namespace field to every provider tool
definition. Rejected for this issue because it duplicates an invariant already
carried by `CapabilityId` and would require migration across every producer.

**Review trigger:** Revisit if a valid capability can be owned by an extension
different from the first capability-ID segment.

## D2 — Explicit search remains relevance-first

**Decision:** Do not diversity-rerank explicit `tool_search(query=...)`
results. Namespace fairness applies only to the passive always-visible catalog
preview.

**Why:** BM25F is already quality-gated, and changing explicit ranking would
confound retrieval quality with presentation changes.

**Alternative considered:** Interleave explicit results by namespace. Rejected
because it can move the best matching tool below weaker cross-namespace
results.

## D3 — Passive preview uses namespace-first round-robin allocation

**Decision:** Render every authorized discoverable namespace and count before
allocating optional representative tool names. Allocate representatives in
stable namespace rounds. Reviewed pins are stronger than representatives: when
authorized, their complete definitions are directly visible before the bridge
preview is assembled.

**Why:** This prevents alphabetical tool-name order and large integrations from
starving later namespaces while keeping serialization deterministic.

**Alternative considered:** Allocate bytes proportionally to namespace size.
Rejected because large integrations would again dominate awareness.

## D4 — Pins are visibility preferences, never grants

**Decision:** Pins are supplied by the profile/policy owner as canonical
capability IDs, then intersected with the effective authorized catalog. Unknown,
denied, and duplicate pins have no effect.

**Why:** Disclosure must not infer product policy or widen authority. Canonical
IDs also avoid provider-name encoding ambiguity.

**Alternative considered:** Automatically pin frequently called tools.
Rejected because popularity is not a reviewed safety or product-policy signal,
especially for side-effecting tools.

## D5 — Complete-signature byte budgets

**Decision:** Keep the initial 8 KiB per-schema and 24 KiB total canonical JSON
budgets while collecting the end-to-end comparison. Schemas are complete or
omitted, never truncated.

**Why:** The per-schema ceiling guarantees one result cannot consume the whole
response, while the total ceiling bounds context growth. The benchmark must
decide whether the total should be reduced or biased more strongly toward rank
1; intuition alone is insufficient.

**Alternative considered:** Return only rank 1's schema. Kept as a benchmark
candidate rather than selected as the production rule because ambiguous tasks
may benefit from several complete candidates.

## D6 — Reviewable implementation, benchmarkable stack tip

**Decision:** Keep #7409 as the retrieval baseline and implement the complete
behavior plus all experimental/control arms on the #7410 stack tip.

**Why:** The benchmark needs all arms in one executable tree, while the clean
baseline remains independently reviewable.

**Alternative considered:** Squash the baseline and implementation into one
large PR. Rejected because it makes retrieval-fixture changes harder to review
without improving benchmark validity.

## D7 — Initial production pins are read-only and profile-specific

**Decision:** Keep product-specific pins in profile-owned configuration rather
than generic runner code. The initial reviewed benchmark configuration uses
`gmail.list_messages`, `google-calendar.list_events`, and `github.search_code`
for `interactive_tools`; mission and subagent profiles use
`github.search_issues_pull_requests` and `github.get_file_content`. Scheduled
trigger and unknown profiles receive no pins. Shipping defaults remain empty
until an operator supplies this reviewed map.

**Why:** These are compact, common read workflows that help the model gather
context before acting. Restricting the first reviewed set to read-only tools
avoids making an external write disproportionately salient. The run-profile
owner supplies canonical capability IDs through
`REBORN_TOOL_DISCLOSURE_PROFILE_PINS`; missing or unauthorized extensions remain
no-ops. Keeping concrete extension policy out of the generic runner also
preserves the extension-specificity architecture boundary.

**Alternative considered:** Pin write actions such as email send, calendar
create, and GitHub issue mutation because they are common. Rejected for the
initial rollout because usage frequency alone is not enough reason to elevate a
side-effecting tool.

**Review trigger:** Replace or extend these pins when end-to-end task evidence
shows a different small set materially improves completion without raising
invalid-call or approval churn.

## D8 — Namespace overflow is explicit, not lossy by accident

**Decision:** Namespace/count rows consume the passive-preview budget before
representative tool names. If pathological namespace cardinality or identifier
length exceeds the safe-description ceiling, stop at the last complete row and
say that additional authorized namespaces exist; never emit a partial name.

**Why:** The 4,096-byte safety ceiling makes universal physical representation
impossible for arbitrary identifiers. This policy guarantees complete,
deterministic entries and makes overflow visible. The benchmark catalogs use 20
namespaces, for which every namespace fits.

**Alternative considered:** Hash or truncate namespace identifiers to force all
of them into the preview. Rejected because the model could not reliably search
for the real integration and collisions would make counts misleading.

## D9 — Benchmark arms share one production binary

**Decision:** Extend `REBORN_TOOL_DISCLOSURE` with explicit `compact`,
`signatures`, and `namespaces` comparison modes. Together with `off` and the
default `bridged` mode, these select all five issue arms from the same build.

**Why:** Comparing separate historical branches would mix code drift with the
interaction being measured. One binary keeps authorization, dispatch, model
route, and instrumentation constant; only presentation/signature/pin behavior
changes. Unknown values continue to fail closed to `off`.

**Alternative considered:** Keep the arms test-only. Rejected because that
would prevent Railway or another deployed environment from gathering real
provider token and latency data.

## D10 — Deterministic gates and live measurements stay distinct

**Decision:** CI gates deterministic retrieval, protocol-turn, namespace,
authorization-leakage, and stable-surface properties. Input/cached token counts
and cold/warm end-to-end latency are populated only by a real configured model
run; missing provider measurements remain explicitly unmeasured.

**Why:** Local serialization bytes and wall-clock unit-test timings are useful
diagnostics but are not model tokenization or network latency. Reporting them as
such would create precise-looking but invalid benchmark evidence.

**Alternative considered:** Estimate tokens as bytes divided by four and commit
local timings. Rejected because provider tokenizers, caching, network, and model
behavior dominate the requested end-to-end measurements.

## D11 — The legacy control keeps its absent completeness marker

**Decision:** The `compact` control arm preserves the historical search-result
shape, which has neither `parameters` nor `schema_complete`. The shared model
protocol explicitly treats an absent marker as describe-required. Signature
arms return the explicit marker and may skip describe only when it is `true`.

**Why:** Adding a marker to the control would change the payload under test,
while leaving the prompt ambiguous could accidentally remove the historical
round trip. This keeps the wire-level control intact and makes the intended
interaction deterministic at the instruction layer.

**Alternative considered:** Add a second compact-only system prompt. Rejected
because the one-sentence compatibility rule expresses the distinction without
duplicating a multi-line prompt asset or widening composition APIs.

## D12 — Representative action summaries use exact tool names

**Decision:** After namespace/count rows, the bounded action summary consists
of exact provider-visible tool names selected in fair rounds rather than prose
copied from tool descriptions.

**Why:** Exact names are actionable search terms and already pass the provider
tool-name validation boundary. Free-form extension descriptions can contain
sensitive-content markers and could make the entire bridge description fail
safe-summary validation; sanitizing or rewriting them would add a second,
potentially misleading retrieval vocabulary.

**Alternative considered:** Include truncated natural-language descriptions.
Rejected because byte truncation can split meaning, and arbitrary descriptions
are neither guaranteed safe for the always-visible surface nor stable enough
for cache comparisons.
