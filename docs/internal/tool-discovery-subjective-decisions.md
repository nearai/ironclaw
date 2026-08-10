# Tool discovery implementation decisions

This log records subjective calls made while implementing issue #7405 so they
can be reviewed independently from the code. It is intentionally explicit
about alternatives and should be updated whenever implementation evidence
changes one of these choices.

## D1 — Namespace identity comes from `CapabilityId`, with semantic first-party groups

**Decision:** Derive discovery namespaces only from the typed `CapabilityId`.
For extension-owned tools, keep the first segment as the namespace (`github`,
`gmail`, and so on). For the platform-owned `builtin.*` and `ironclaw.*`
families, map stable capability-id families to intent-oriented groups such as
`coding`, `memory`, `scheduling`, `skills`, and `observability`. Never derive a
namespace from the provider-facing tool name or description.

**Why:** The `CapabilityId` contract already requires
`<extension>.<capability>[.<sub>...]`, and extension registration constructs
capability IDs from the package/extension identity. That identity is useful for
external products, but the platform owners `builtin` and `ironclaw` collapse
unrelated intents and provide almost no routing signal. Provider names are
encoded for model APIs and are not an ownership boundary.

**Alternative considered:** Add a second namespace field to every provider tool
definition. Rejected because the current first-party taxonomy is deterministic
from stable IDs, while a new cross-crate field would require every producer,
adapter, wrapper, and test double to migrate. Revisit when extensions need to
override their extension-id namespace.

**Review trigger:** Revisit if extensions need multiple model-facing groups or a
valid capability can be owned by an extension different from the first
capability-ID segment.

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

## D13 — Stop the full live matrix on a required-scenario arm failure

**Decision:** Screen every arm with the exact canonical-ID scenario before the
full live matrix. If an arm deterministically fails that minimum path, fix it
before spending the remaining provider calls.

**Evidence:** On 2026-08-10 at commit `7a4cafea`, the 100-tool screen completed
in `off` and pinned `bridged`. `compact`, `signatures`, and unpinned
`namespaces` ended after their first `tool_call` or `tool_search` without
invoking the target; repeating with the exact canonical query produced the
same result. The preliminary scale screen compared only the two working paths.
Both completed at 500 and 1,000 tools, but the single cold samples had high
provider variance: `off` took 86.7s and 41.6s, while `bridged` took 11.3s and
75.0s. These are diagnostics, not a winner claim.

**Why:** A 420-observation result would not rescue a path that fails a required
scenario by construction, and averaging that failure into an overall score
would hide the actual merge blocker.

**Alternative considered:** Complete the matrix and rank the two working arms.
Rejected until the blocked arms pass the smoke case; one cold observation per
size is not enough to distinguish provider variance from disclosure latency.

**Resolution:** The screen exposed the model gateway's unavailable-capability
guard comparing an explicitly requested canonical ID only with the advertised
subset. It suppressed both the policy-filtered discovery call and the later
exact deferred invocation. The guard now allows `tool_search`/`tool_describe`
and the exact requested target to reach the normal capability policy while it
continues suppressing unrelated substitute calls. A fresh live `signatures`
screen then completed search, signature return, and the correct MCP invocation.

## D14 — Default to namespace summaries without pins

**Decision:** Make `namespaces` the production default. Keep `bridged` available
as an explicit opt-in for deployments whose profile pins have been validated
against representative end-to-end workflows.

**Evidence:** The stopped-on-request live run at commit `753f091b2c` produced
268 completed observations with the production server, NearAI
`deepseek-ai/DeepSeek-V4-Flash`, 20 hosted MCP namespaces, and deterministic
100- and 500-tool catalogs. At both completed sizes, `namespaces` achieved
28/28 task completions with zero unauthorized calls. Its median end-to-end
latency was 12.8s at 100 tools and 11.4s at 500 tools. The pinned `bridged` arm
also achieved 28/28 at 100 tools with a 7.0s median, but at 500 tools its first
cross-namespace run called the pinned `google_calendar__list_events` tool
instead of the required `google_calendar__create_event`, then claimed the event
was created. A later repetition also failed. The completed comparable 500-tool
single-tool subset was 16/16, but the interrupted four-repetition workflow
group was not written to the observation JSONL.

**Why:** Pins can substantially reduce latency for a known compact workflow,
but a nearby high-value primitive can attract the model away from the required
action. Namespace summaries retained perfect observed completion at both
catalog sizes without paying this correctness risk or a standing pinned-schema
budget. The 1,000-tool tier was not run because the user requested a decision
from the evidence already collected; this is therefore the best-supported
default, not a claim of exhaustive scale validation.

**Alternative considered:** Keep `bridged` as the default because it was the
fastest perfect arm at 100 tools. Rejected because the 500-tool trace showed a
specific pin-induced wrong-tool call and false success claim. Latency does not
override the task-completion gate. Removing only the calendar pin was also
rejected as overfitting this seven-task fixture; pins remain an explicit,
reviewable deployment choice.

## D15 — Replace first-party ownership buckets with semantic namespaces

**Decision:** Keep extension IDs as discovery namespaces for external tools,
but group platform-owned capability-ID families by user intent. The initial
closed taxonomy is `agents`, `coding`, `data`, `extensions`, `memory`,
`messaging`, `observability`, `scheduling`, `settings`, `skills`, and `web`,
with `system` as the deterministic fallback for uncategorized first-party IDs.

**Evidence:** Exact-head Railway QA for the namespace default exposed only
`builtin (22)` and `ironclaw (2)` before this change. Those labels correctly
described ownership but did not tell the model whether the deferred tools were
for scheduling, skill management, observability, or another task family. A
focused 100-tool live run after the change completed six unaffected task
classes with zero unauthorized calls. The upload task found the correct tool
but stopped because the benchmark had named a nonexistent workspace file; once
the prompt supplied deterministic inline content, the rerun completed too.

**Why:** Namespace summaries spend permanent prompt bytes and therefore must
provide routing information. A closed mapping over stable capability IDs is
cache-stable, deterministic, reviewable, and cannot be manipulated through
provider-facing names or descriptions. Coding tools are included in the
taxonomy but appear in the on-demand summary only when they are actually
deferred; directly advertised core tools are not counted as searchable.

**Alternative considered:** Add a model-facing namespace field to the shared
provider-tool contract. Rejected for this PR because first-party intent is
already encoded in stable capability-ID families, while a new field would
force a cross-workspace producer and test-double migration. This becomes the
preferred follow-up if extensions need multiple semantic groups or explicit
namespace overrides.

## D16 — Live benchmark tasks own every required input

**Decision:** Make the upload scenario provide deterministic file content in
the prompt rather than depend on a pre-existing workspace fixture.

**Why:** The benchmark scores whether the model discovers and invokes the
correct tool. Asking it to upload an absent file instead measures whether it
fabricates content or correctly requests missing input. The observed model
found the exact Google Drive upload tool and its complete `mime_type` schema,
then reasonably refused to invent `report.csv`; scoring that as retrieval
failure was invalid.

**Alternative considered:** Create `report.csv` in the temporary workspace.
Rejected because inline content keeps the task self-contained across server
homes and avoids adding filesystem setup as another benchmark dependency.

## D17 — Score end-to-end execution, not tool-name presence

**Decision:** A benchmark observation completes only when the expected tools are
called in order with the task's required arguments. No-match and denied-tool
cases inspect every model-attempted call in the trace, including wrapped
`tool_call` targets. Latency starts at the task request and stops at the first
correct tool call, not the first arbitrary call.

**Why:** Tool-name presence can award success to an invocation that uploads the
wrong content, reads instead of creates, or performs a workflow backwards. A
hard-coded zero leakage counter cannot validate the authorization claim. These
are task-completion and safety properties, not retrieval-only metrics.

**Alternative considered:** Keep name-only scoring and manually inspect failed
traces. Rejected because it produces precise-looking aggregate completion and
leakage numbers that the harness did not actually measure.

## D18 — Benchmark namespaces must carry the semantics under test

**Decision:** The synthetic catalog uses 20 stable semantic MCP integration IDs
(`github`, `gmail`, `google-calendar`, and so on). Fixed relevance-corpus tools
stay with their semantic owner; generated distractors fill the smallest bucket
deterministically. Completed observations are appended and synced individually
and resume by stable observation ID.

**Why:** Randomly round-robining real tools through anonymous packages measures
fair allocation across arbitrary buckets, not whether namespace summaries help
the model route to a relevant integration. Per-group buffering also discarded
valid observations when a later repetition was interrupted.

**Alternative considered:** Keep anonymous packages because they are perfectly
balanced. Rejected because balance cannot compensate for invalid namespace
meaning. The fixed corpus may create a small size imbalance at low tool counts;
generated tools still distribute fairly without moving relevant tools away from
their owner.

## D19 — Keep profile pins typed and explicit at construction

**Decision:** Parse profile-pin configuration into validated
`CapabilitySurfaceProfileId` keys, preserve parse-error causes in diagnostics,
and require every disclosure decorator constructor to receive its mode.

**Why:** Raw string keys postpone identity validation, while a constructor that
silently selects `bridged` can activate pins when a caller forgets a follow-up
builder call. The explicit mode makes the context-cost decision visible at every
construction site.

**Alternative considered:** Retain the fluent `.with_mode(...)` override and
document the default. Rejected because the unsafe intermediate value remains a
valid production object and documentation cannot enforce the call sequence.

## D20 — Keep `namespaces` as the default after corrected 100-tool comparison

**Decision:** Keep unpinned `namespaces` as the production default and retain
`bridged` as an explicit, deployment-reviewed optimization.

**Evidence:** Benchmark artifact identity
`tool-discovery-v2-1a674e7-nearai-deepseek-v4-flash-seed7405-100tools-56obs`
records 56 exact-head observations at commit `1a674e7724`. It used the NearAI
route with `deepseek-ai/DeepSeek-V4-Flash`, temperature `0.0`, catalog generator
`tool-search-scale-v2`, seed `7405`, two arms, seven task classes, and per-arm/task
repetitions labeled one cold followed by three warm against the 100-tool semantic
catalog. `namespaces` completed 26/28
(92.9%) with a 21.6s overall median; `bridged` completed 24/28 (85.7%) with a
14.6s median. Both had zero forbidden-tool attempts. The simple calendar and
upload tasks were materially faster with pins, but the two-step Gmail-to-calendar
workflow completed 2/4 under `namespaces` and 0/4 under `bridged`. One bridged
workflow also invoked the pinned calendar-list tool before attempting creation.
Several unrelated task/arm groups hit the provider's roughly 187s tail, so the
latency difference is directional rather than a clean causal estimate.

**Why:** Pins buy latency by spending standing context and steering attention
toward selected primitives. That is valuable for a known narrow workflow, but
the stricter argument-and-order scorer confirms the same end-to-end correctness
risk seen in the earlier 500-tool exploratory run. The default should optimize
for task completion; deployments may opt into pins after validating their own
workflow mix.

**Alternative considered:** Select `bridged` because its overall median was
7.0s lower. Rejected because the completion gate failed and the median mixes
very different provider-tail behavior across task classes. A latency win does
not compensate for a lower end-to-end success rate.
