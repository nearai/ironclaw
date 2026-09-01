use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{CapabilityResultWrite, DurablePersistence, LoopCapabilityResultWriter};
use async_trait::async_trait;
use futures::future::join_all;
use ironclaw_common::truncate_preview;
use ironclaw_host_api::{
    capability_surface::CapabilitySurfacePolicy,
    ids::{AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName, TenantId, ThreadId},
    model_result_preview::{MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES, MODEL_OBSERVATION_MAX_BYTES},
    resolution::{Resolution, ResolutionBatch},
    result_meta::FailureKind,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityCallCandidate, CapabilityFailureDetail,
    CapabilityInputRef, CapabilityProgress, CapabilitySurfaceVersion, LoopCapabilityPort,
    LoopRequest, LoopRequestBatch, LoopRunContext, ProviderToolCall, ProviderToolCallCapabilityIds,
    ProviderToolCallReplay, ProviderToolDefinition, RegisterProviderToolCallRequest,
    VisibleCapabilityRequest, VisibleCapabilitySurface, resolution,
};
use ironclaw_turns::{CapabilityActivityId, TurnId};
use serde_json::{Value, json};
use tracing::debug;

use crate::tool_disclosure::{
    ActiveSet, CapabilityCatalog, CatalogSearchResult, DisclosureCaps, PromotedSet, TOOL_CALL_NAME,
    TOOL_DESCRIBE_NAME, TOOL_SEARCH_NAME, bridge_tool_definitions, canonicalize_json,
    definition_matches_provider_name, is_bridge_capability_id, is_bridge_name,
    select_active_set_for_mode,
};
use crate::tool_search::{
    AuthorizedToolSearchIndex, MAX_SEARCH_QUERY_BYTES, definitions_fingerprint,
};

const DISCLOSURE_INPUT_PREFIX: &str = "input:tool-disclosure:";

/// Fixed JSON scaffolding a `ModelVisibleToolObservation` embedding a tool_search reply adds
/// beyond the reply's own JSON-string-escaped size: schema_version/status/summary/detail tag/
/// artifacts/trust fields, plus the `result_ref` string carried twice
/// (`crates/app/ironclaw_composition/src/runtime/capability_host/result_preview.rs::result_reference_observation`).
///
/// **Moved here from `ironclaw_host_api::model_result_preview::MODEL_OBSERVATION_WRAPPER_ALLOWANCE_BYTES`
/// (PR #7984 review thread 11).** Before this fix the allowance had to double as an estimate of
/// escaping overhead too — content-dependent, and therefore never exact — because nothing measured
/// the real embedding cost. Now that `wrapped_reply_fits` computes the exact escaped size itself
/// (see below), this constant only needs to cover the truly fixed envelope, so it is no longer a
/// content-dependent fudge factor and no longer belongs to `host_api`'s shared model-result
/// contract: it has exactly one production consumer (this module), so it lives beside that
/// consumer.
const OBSERVATION_ENVELOPE_SCAFFOLDING_BYTES: usize = 512;

/// How many ranks reach the model in one tool_search reply. The single definition used at
/// BOTH the `.take(...)` below AND `invoke_tool_search`'s runtime fallback AND the advertised
/// schema `"default"` in tool_disclosure.rs — keep all three reading this one constant so
/// drift between them is a compile-time impossibility, not a silent mismatch.
pub(crate) const TOOL_SEARCH_INLINE_RESULT_LIMIT: usize = 3;

/// One-line description cap (UTF-8 char boundary). The longest single-line
/// `description = "..."` across every committed first-party extension
/// manifest runs up to 116 B today (independently scanned across all
/// `crates/extensions/packages/*/manifest.toml` tool entries; only a handful
/// exceed 70 B, and the great majority of entries stay <=70 B). This constant
/// guards a future longer one, bounding every compact entry by construction
/// regardless — 116 B is still well under the 163 B worst case below.
///
/// **Byte-accounting note:** `truncate_preview` (`crates/contracts/ironclaw_common/src/util.rs:34`)
/// appends its `"..."` ellipsis AFTER truncating to `max_bytes` rather than reserving room for it,
/// and its `<tool_output>` re-close branch is inert here (a tool/schema description never starts
/// with `<tool_output`). So the true worst case is `COMPACT_DESCRIPTION_MAX_BYTES + 3` = **163 B**,
/// not 160. We pass 160 as the cap; the helper may return up to 163; every byte accounting below
/// (the compact-entry bound and the required/capability_id comparison) uses 163 B, not 160, as the
/// worst case. We do not shrink this constant to 157 to force a round 160 B result — that would be
/// a cosmetic bandaid over the same helper behavior.
///
/// **This cap is also rank 1's real competitor for budget, not a cosmetic detail** (see the Goal
/// section's arithmetic): every byte this constant lets ranks 2-3's descriptions grow by is a byte
/// rank 1's own schema no longer has room for under `wrapped_reply_fits`'s escaped-size check.
/// Raising this constant without re-checking the boundary test above would silently shrink rank 1's
/// headroom.
const COMPACT_DESCRIPTION_MAX_BYTES: usize = 160;

/// Description cap for **rank 1 only** — the rank `TOOL_SEARCH_INVOKE_DIRECTLY_GUIDANCE` tells
/// the model to invoke.
///
/// This catalog encodes routing guidance in description *tails*, and a flat 160 B cap amputates
/// exactly that half. `builtin.http.save`'s description
/// (`crates/kernel/ironclaw_host_runtime/src/first_party_tools/http.rs`, `WEB_SEARCH_PREFERENCE`)
/// runs 559 B and ends "For general web research or retrieving human-facing web pages, prefer an
/// available `web_search` tool" — the sentence that answers "which tool do I use for this?". At
/// 160 B the model sees "...Use this capability for structure" and never learns the answer. A
/// pinchbench trace at a791c14e59 caught the consequence: `task_eu_regulation_research` issued 9
/// searches hunting for a web tool, never saw that sentence, abandoned the tool path and fell back
/// to 322 `builtin.shell` calls (vs 61), scoring 0.00 against 0.89.
///
/// Rank 1 alone gets the larger cap because rank 1 is the entry the model is directed to act on,
/// and because the budget cannot afford it for all three: at 640 B each the reply reaches ~3,720 B
/// and the ladder degrades it, losing rank 1's schema. Asymmetric, deliberately.
const RANK_ONE_DESCRIPTION_MAX_BYTES: usize = 640;

/// Cap on how many `required` parameter names ride in a degraded compact entry
/// (`RequiredParamsShape::Capped`). Untrusted, hosted-MCP tool schemas can declare an
/// arbitrarily large `required` array — #7984 measured 20 names x 40 chars producing a
/// 6,332 B RAW compact reply on its own, over the first-look ceiling outright regardless of
/// escaping. 8 names at the largest realistic name length (`ProviderToolName::MAX_BYTES`,
/// 64 B) is ~576 B for the array across all three ranks combined even before this cap's own
/// escaping accounting, comfortably inside the budget `bounded_search_output`'s later,
/// smaller-count rungs still have to share with everything else in the reply. When even the
/// capped array does not fit, the next rung drops the field entirely
/// (`RequiredParamsShape::Omitted`).
const REQUIRED_PARAMS_COMPACT_CAP: usize = 8;

/// A SUCCESS results[] entry name, not a failure diagnostic (#5712 covers describe/call FAILURE branches only).
const TOOL_SEARCH_INVOKE_DIRECTLY_GUIDANCE: &str = "rank 1 has a complete parameters schema; invoke it directly with the schema above, or via tool_call";
/// Steers the model to `tool_describe` when rank 1's schema is too large to inline here. NOTE:
/// `tool_describe`'s own reply is NOT bounded by this file's first-look-envelope fix — it still
/// rides the generic pager and can itself collapse behind an `omitted` marker for a large enough
/// schema (same defect class as D21 in `docs/internal/tool-discovery-subjective-decisions.md`).
/// Tracked as follow-up, not yet fixed.
const TOOL_SEARCH_DESCRIBE_FOR_SCHEMA_GUIDANCE: &str = "rank 1 matched but its schema is too large to show inline; call tool_describe on it to get the full parameters schema, then invoke it";
const TOOL_SEARCH_NO_MATCH_GUIDANCE: &str = "no deferred tool matches this query; do not search again for it; tell the user the capability is unavailable";
/// #7984: one or more results' `required` parameter list was too large (or too many results
/// together were too large) to show in full — an untrusted, hosted-MCP tool schema can declare
/// an arbitrarily large `required` array. Used by the `RequiredParamsShape::Capped`/`Omitted`
/// rungs and by the fewer-results rung, all of which still return real matches.
const TOOL_SEARCH_TRIMMED_RESULTS_GUIDANCE: &str = "matched results' parameter lists were too large to show in full; call tool_describe on a result to see its complete required parameters, then invoke it";
/// The floor rung: even a single trimmed result did not fit. Returned with an empty `results`
/// array, so unlike every guidance string above, this one must not imply any result is present.
const TOOL_SEARCH_QUERY_TOO_BROAD_GUIDANCE: &str = "no result fit within the reply size limit; narrow the query (e.g. a more specific tool name) and search again";

/// Internal bridge name for an auto-loaded schema (describe-first) response.
///
/// NOT a real provider tool name, so it can never collide with a catalog tool or
/// trip `is_bridge_name`. When the model calls a deferred tool whose schema it
/// has not loaded this turn with arguments that fail pre-dispatch validation, the
/// register path routes the call to this synthetic bridge instead of dispatching
/// blind; `invoke_describe_first` then returns the tool's parameter schema so the
/// model's retry can carry the required fields. See `register_describe_first`.
const DESCRIBE_FIRST_BRIDGE_NAME: &str = "tool_disclosure:auto_schema";

/// Provider tool name of the loop's `capability_info` inspector (mirrors
/// `crate::capability_info::TOOL_NAME`). Inspecting a deferred
/// tool via `capability_info` is treated as intent to use it: the target is
/// disclosed + promoted so it becomes directly callable — the `tool_search` →
/// `capability_info` → direct-call discovery path.
const CAPABILITY_INFO_NAME: &str = "capability_info";

pub struct ToolDisclosureCapabilityDecorator {
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    caps: DisclosureCaps,
    mode: crate::ToolDisclosureMode,
}

impl ToolDisclosureCapabilityDecorator {
    pub fn new(
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        mode: crate::ToolDisclosureMode,
    ) -> Self {
        Self {
            result_writer,
            promoted_by_scope: Arc::new(Mutex::new(HashMap::new())),
            caps: DisclosureCaps::default(),
            mode,
        }
    }

    /// Wrap one run's capability port with disclosure using the exact
    /// policy already resolved by the runner-private profiled factory.
    pub fn decorate_with_policy(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
        policy: Arc<CapabilitySurfacePolicy>,
    ) -> Arc<dyn LoopCapabilityPort> {
        self.decorate_with_policy_and_pins(run_context, inner, policy, Vec::new())
    }

    /// Wrap one run with profile-owned visibility pins. Pins use canonical
    /// capability ids and are only applied after the effective authorized
    /// definitions have been fitted, so they can never grant authority.
    pub fn decorate_with_policy_and_pins(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
        policy: Arc<CapabilitySurfacePolicy>,
        profile_pins: Vec<CapabilityId>,
    ) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(ToolDisclosureCapabilityPort {
            inner,
            run_context: run_context.clone(),
            result_writer: Arc::clone(&self.result_writer),
            promoted_by_scope: Arc::clone(&self.promoted_by_scope),
            caps: self.caps,
            mode: self.mode,
            policy,
            profile_pins,
            turn_state: Mutex::new(None),
            bridge_inputs: Mutex::new(BTreeMap::new()),
            tool_call_target_inputs: Mutex::new(BTreeMap::new()),
        })
    }
}

struct ToolDisclosureCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    caps: DisclosureCaps,
    mode: crate::ToolDisclosureMode,
    /// #5712/#5659-w6: the caller's effective policy, resolved once in
    /// `ToolDisclosureCapabilityDecorator::decorate_with_policy` — narrows disclosed
    /// tool_search/tool_describe metadata *and* the tool_search bridge's own
    /// advertised description (the always-on catalog index).
    policy: Arc<CapabilitySurfacePolicy>,
    /// Reviewed visibility preferences from the run-profile owner. These are
    /// not grants; catalog construction sees only authorized definitions.
    profile_pins: Vec<CapabilityId>,
    turn_state: Mutex<Option<ToolDisclosureTurnState>>,
    bridge_inputs: Mutex<BTreeMap<String, BridgeInvocation>>,
    tool_call_target_inputs: Mutex<BTreeMap<String, CapabilityId>>,
}

#[derive(Debug, Clone)]
struct ToolDisclosureTurnState {
    turn_id: TurnId,
    /// Fingerprint of the effective authorized tool surface and indexed
    /// metadata. The catalog and search index are rebuilt when this changes so tools that become available
    /// mid-turn (an activated extension, a completed OAuth connect) enter the
    /// disclosure catalog and become discoverable/describable/callable — without
    /// it, `tool_describe`/`tool_call` report a just-activated tool as "unknown".
    definitions_fingerprint: u64,
    surface_version: Option<CapabilitySurfaceVersion>,
    catalog: CapabilityCatalog,
    search_index: AuthorizedToolSearchIndex,
    active: ActiveSet,
    disclosed_names: BTreeSet<String>,
    search_ranks: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
struct BridgeInvocation {
    kind: BridgeKind,
    arguments: Value,
}

/// Which synthetic bridge a stored [`BridgeInvocation`] resolves to at invoke
/// time. Replaces discriminating on stashed name strings so the dispatch in
/// [`ToolDisclosureCapabilityDecorator::invoke_bridge`] is exhaustive and
/// legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeKind {
    /// `tool_search` — keyword-rank the deferred catalog.
    Search,
    /// `tool_describe` — return a named tool's parameter schema.
    Describe,
    /// Internal auto-schema (describe-first): return a deferred tool's schema
    /// after a blind call to it failed pre-dispatch validation.
    DescribeFirst,
    /// `tool_call` — invoke a tool by name. Reaching invoke with this kind means
    /// the target could not be resolved (a resolvable target is dispatched
    /// directly and never stored as a bridge invocation), so it always errors
    /// recoverably.
    Call,
}

/// Shared 4-field core: D11's `compact_search_results` and tool_search's own `compact_result_entry` both build from here, then diverge.
fn compact_result_fields(result: &CatalogSearchResult) -> Value {
    json!({"name": result.name, "capability_id": result.capability_id.as_str(), "description": result.description, "required": result.required_params})
}

fn compact_search_results(results: Vec<CatalogSearchResult>) -> Vec<Value> {
    results.iter().map(compact_result_fields).collect()
}

/// One rank's compact entry: D11's core fields plus a truncated description and schema_complete:false. Capped by construction (name <= ProviderToolName::MAX_BYTES 64 B, description <= COMPACT_DESCRIPTION_MAX_BYTES + 3 = 163 B worst case, per the note above — truncate_preview appends "..." after the cap, not within it) — never needs a fit check.
///
/// Truncation uses the EXISTING `ironclaw_common::truncate_preview` (`crates/contracts/ironclaw_common/src/util.rs:34`)
/// rather than a new helper — `ironclaw_common` is already a dependency of `ironclaw_loop_host`
/// (`crates/loop/ironclaw_loop_host/Cargo.toml:31`) and the function is already live production code,
/// used at `crates/domains/ironclaw_llm/src/nearai_chat.rs:768`. Note: `truncate_preview` appends
/// `"..."` on truncation and re-closes a `<tool_output>` tag if truncation cut through one — the
/// tag-closing branch is a no-op for a plain description string, and the `"..."` suffix is a
/// (desirable) behavior change from the earlier no-suffix sketch: it signals to the model that the
/// description was cut. A THIRD copy of this byte-boundary-walk already exists as a private helper —
/// `ironclaw_host_api::dispatch::truncate_at_char_boundary` (`crates/contracts/ironclaw_host_api/src/dispatch.rs:91`,
/// module-private, not part of `dispatch`'s public surface) — do NOT add a fourth; this is exactly the
/// class of duplication `truncate_preview` reuse here avoids repeating.
fn compact_result_entry(result: &CatalogSearchResult) -> Value {
    compact_result_entry_shaped(result, RequiredParamsShape::Full)
}

/// Rank 1's compact entry: identical to `compact_result_entry` except the description keeps its
/// routing tail (`RANK_ONE_DESCRIPTION_MAX_BYTES`). Used for the entry rank-1 guidance points at.
fn rank_one_compact_entry(result: &CatalogSearchResult) -> Value {
    let mut entry = compact_result_entry_shaped(result, RequiredParamsShape::Full);
    entry["description"] = Value::String(truncate_preview(
        &result.description,
        RANK_ONE_DESCRIPTION_MAX_BYTES,
    ));
    entry
}

/// The per-entry `required`-field degradation rungs `bounded_search_output` tries in order.
/// Untrusted, hosted-MCP tool schemas can declare an arbitrarily large `required` array (#7984
/// measured 20 names x 40 chars alone producing a 6,332 B RAW compact reply, over the ceiling
/// outright) — this is the escape hatch that keeps one pathological schema from taking the whole
/// tool_search reply over budget, on every return path, not only the schema-complete one.
#[derive(Debug, Clone, Copy)]
enum RequiredParamsShape {
    /// Every declared required-param name, unmodified.
    Full,
    /// At most `REQUIRED_PARAMS_COMPACT_CAP` names; the rest are dropped silently (the entry
    /// still declares `schema_complete: false`, so the model already knows to call
    /// `tool_describe` for the authoritative full schema).
    Capped,
    /// The `required` field is absent from the entry entirely.
    Omitted,
}

/// One rank's compact entry: D11's core fields plus a truncated description,
/// `schema_complete:false`, and `required` shaped per `shape`. `name`/`description` are capped by
/// construction (name <= `ProviderToolName::MAX_BYTES` 64 B, description <=
/// `COMPACT_DESCRIPTION_MAX_BYTES` + 3 = 163 B worst case, per the note above — `truncate_preview`
/// appends "..." after the cap, not within it); `capability_id`/`required` are NOT bounded by
/// construction (an untrusted hosted-MCP schema controls both), which is exactly why the caller
/// (`bounded_search_output`) always re-measures the candidate it builds from this rather than
/// trusting a per-entry cap alone.
///
/// Truncation uses the EXISTING `ironclaw_common::truncate_preview` (`crates/contracts/ironclaw_common/src/util.rs:34`)
/// rather than a new helper — `ironclaw_common` is already a dependency of `ironclaw_loop_host`
/// (`crates/loop/ironclaw_loop_host/Cargo.toml:31`) and the function is already live production code,
/// used at `crates/domains/ironclaw_llm/src/nearai_chat.rs:768`. Note: `truncate_preview` appends
/// `"..."` on truncation and re-closes a `<tool_output>` tag if truncation cut through one — the
/// tag-closing branch is a no-op for a plain description string, and the `"..."` suffix is a
/// (desirable) behavior change from the earlier no-suffix sketch: it signals to the model that the
/// description was cut. A THIRD copy of this byte-boundary-walk already exists as a private helper —
/// `ironclaw_host_api::dispatch::truncate_at_char_boundary` (`crates/contracts/ironclaw_host_api/src/dispatch.rs:91`,
/// module-private, not part of `dispatch`'s public surface) — do NOT add a fourth; this is exactly the
/// class of duplication `truncate_preview` reuse here avoids repeating.
fn compact_result_entry_shaped(result: &CatalogSearchResult, shape: RequiredParamsShape) -> Value {
    let mut entry = compact_result_fields(result);
    entry["description"] = Value::String(truncate_preview(
        &result.description,
        COMPACT_DESCRIPTION_MAX_BYTES,
    ));
    entry["schema_complete"] = Value::Bool(false);
    match shape {
        RequiredParamsShape::Full => {}
        RequiredParamsShape::Capped => {
            if result.required_params.len() > REQUIRED_PARAMS_COMPACT_CAP {
                entry["required"] = Value::Array(
                    result
                        .required_params
                        .iter()
                        .take(REQUIRED_PARAMS_COMPACT_CAP)
                        .cloned()
                        .map(Value::String)
                        .collect(),
                );
            }
        }
        RequiredParamsShape::Omitted => {
            if let Value::Object(fields) = &mut entry {
                fields.remove("required");
            }
        }
    }
    entry
}

/// Build the JSON reply carrying `guidance`. Every caller passes the exact guidance string the
/// returned value will carry: the value measured here is the value returned — do not reintroduce
/// a probe that measures a different object than it returns.
fn search_reply(query: &str, results: Vec<Value>, guidance: &'static str) -> Value {
    json!({"query": query, "results": results, "guidance": guidance})
}

/// Exact number of bytes `raw` occupies once JSON-string-escaped, quotes excluded. Computed by
/// asking serde_json to perform the escaping (NOT approximated by a per-`"`-or-`\` multiplier):
/// serializing a `Value::String(raw)` performs the exact same escaping
/// `result_reference_observation`'s embedding of the reply into `detail.preview` performs
/// (`crates/app/ironclaw_composition/src/runtime/capability_host/result_preview.rs`), so this
/// measurement cannot drift from the embedding cost it predicts.
fn json_string_escaped_len(raw: &str) -> usize {
    match serde_json::to_string(&Value::String(raw.to_string())) {
        Ok(quoted) => quoted.len().saturating_sub(2), // strip the wrapping `"` .. `"`
        // Serializing a `Value::String` cannot fail in practice (no custom Serialize impl, no
        // writer I/O) — this arm exists only so the function has no panic path. Treat the
        // impossible error as an infinite cost so a candidate is never mistakenly accepted.
        Err(_) => usize::MAX,
    }
}

/// Whether returning `candidate` as the tool_search reply keeps the resulting
/// `ModelVisibleToolObservation.detail.preview` within `MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES`.
///
/// Bounds the ESCAPED size, not the raw size: the reply does not reach the model as raw bytes —
/// `result_reference_observation` re-embeds it as a JSON-escaped string inside `detail.preview`,
/// and escaping cost is content-dependent (~1 extra byte per `"` or `\` in the raw reply, and
/// ordinary JSON is dense with `"`), so a fixed raw-byte budget cannot model it. That gap is the
/// #7984 defect: a reply that fit under the old raw-byte check could still blow the embedded
/// observation's budget once escaped.
///
/// **Two ceilings, each checked against the quantity it actually governs.** They are different
/// limits on different values, and collapsing them into one conservative rule silently degrades
/// replies that would have been delivered intact:
///
/// 1. `raw_len <= MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES` (3,072 B) — `first_look_result_preview`
///    measures the RAW reply to choose verbatim pass-through over the paging path. Fail this and
///    the pager replaces `results` with one `omitted` descriptor (the #7928 defect).
/// 2. `escaped_len + envelope <= MODEL_OBSERVATION_MAX_BYTES` (4,096 B) —
///    `result_reference_observation` re-embeds the reply as a JSON-escaped string inside
///    `detail.preview`. Fail this and it drops `preview` entirely, which reaches the model as the
///    same nothing.
///
/// Escaping is content-dependent (~1 extra byte per `"` or `\`, and ordinary JSON is dense with
/// `"`), so only check 2 can model it — that gap is the review finding this function closes. But
/// check 2's ceiling is 4,096, NOT 3,072: measuring the *wrapped* size against the *raw* ceiling
/// costs ~1 KiB of real headroom. Against the observed first-party corpus that alone degraded 7 of
/// 121 replies out of their rank-1 schema for no delivery reason — every one of them wrapped to
/// ~3.1 KB, comfortably inside 4,096.
fn wrapped_reply_fits(candidate: &Value) -> bool {
    let Ok(serialized) = serde_json::to_vec(candidate) else {
        return false;
    };
    let Ok(raw) = std::str::from_utf8(&serialized) else {
        return false;
    };
    if serialized.len() > MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES {
        return false;
    }
    json_string_escaped_len(raw).saturating_add(OBSERVATION_ENVELOPE_SCAFFOLDING_BYTES)
        <= MODEL_OBSERVATION_MAX_BYTES
}

/// Builds the tool_search reply through an ordered degradation ladder — the first candidate whose
/// ESCAPED size (see `wrapped_reply_fits`) fits the shared first-look ceiling wins, so the reply
/// fits by construction for ANY input, including an adversarial/untrusted hosted-MCP schema:
///
/// a. rank 1 complete: its own full parameter schema, ranks 2-3 compact with `required` full.
/// b. all-compact: no rank carries a schema, `required` still full.
/// c. all-compact with `required` capped (`RequiredParamsShape::Capped`), then omitted entirely
///    (`RequiredParamsShape::Omitted`) if capping alone still does not fit.
/// d. progressively fewer results (3 -> 2 -> 1), `required` omitted.
/// e. the floor: query + guidance + an empty `results` array. Always fits (bounded query, no
///    result content, a static guidance string), so the ladder always terminates.
///
/// Every non-floor rung keeps a real, non-empty `results` array and a guidance string that stays
/// true for what it actually returns — the floor rung is the only one that returns zero results,
/// and the only one using `TOOL_SEARCH_QUERY_TOO_BROAD_GUIDANCE`.
fn bounded_search_output(query: &str, results: Vec<CatalogSearchResult>) -> Value {
    let Some(rank_one) = results.first() else {
        return search_reply(query, Vec::new(), TOOL_SEARCH_NO_MATCH_GUIDANCE);
    };

    // Rung a: rank 1 complete, ranks 2-3 compact with `required` full — the measured object IS
    // the returned object, no probe/clone-and-swap step for the fit check to disagree with.
    //
    // Rank 1 keeps its description tail (`RANK_ONE_DESCRIPTION_MAX_BYTES`): it is the entry both
    // guidance strings point the model at, and this catalog puts "use X instead" routing at the
    // END of descriptions, which a 160 B cap amputates. Ranks 2-3 stay at the tighter cap —
    // affording the larger one for all three overruns the ceiling and costs rank 1 its schema.
    let compact_full: Vec<Value> = results
        .iter()
        .take(TOOL_SEARCH_INLINE_RESULT_LIMIT)
        .enumerate()
        .map(|(rank, result)| {
            if rank == 0 {
                rank_one_compact_entry(result)
            } else {
                compact_result_entry(result)
            }
        })
        .collect();
    let mut complete = compact_full.clone();
    complete[0]["schema_complete"] = Value::Bool(true);
    complete[0]["parameters"] = rank_one.parameters.clone();
    let candidate = search_reply(query, complete, TOOL_SEARCH_INVOKE_DIRECTLY_GUIDANCE);
    if wrapped_reply_fits(&candidate) {
        return candidate;
    }

    // Rung b: all-compact, `required` still full.
    let candidate = search_reply(
        query,
        compact_full,
        TOOL_SEARCH_DESCRIBE_FOR_SCHEMA_GUIDANCE,
    );
    if wrapped_reply_fits(&candidate) {
        return candidate;
    }

    // Rung c (capped): all-compact, `required` capped to REQUIRED_PARAMS_COMPACT_CAP names.
    let compact_capped: Vec<Value> = results
        .iter()
        .take(TOOL_SEARCH_INLINE_RESULT_LIMIT)
        .map(|result| compact_result_entry_shaped(result, RequiredParamsShape::Capped))
        .collect();
    let candidate = search_reply(query, compact_capped, TOOL_SEARCH_TRIMMED_RESULTS_GUIDANCE);
    if wrapped_reply_fits(&candidate) {
        return candidate;
    }

    // Rung c (omitted): all-compact, `required` dropped entirely — the smallest per-entry shape.
    let compact_omitted: Vec<Value> = results
        .iter()
        .take(TOOL_SEARCH_INLINE_RESULT_LIMIT)
        .map(|result| compact_result_entry_shaped(result, RequiredParamsShape::Omitted))
        .collect();
    let candidate = search_reply(
        query,
        compact_omitted.clone(),
        TOOL_SEARCH_TRIMMED_RESULTS_GUIDANCE,
    );
    if wrapped_reply_fits(&candidate) {
        return candidate;
    }

    // Rung d: progressively fewer of the smallest-shape entries (2, then 1).
    for keep in (1..TOOL_SEARCH_INLINE_RESULT_LIMIT).rev() {
        let subset: Vec<Value> = compact_omitted.iter().take(keep).cloned().collect();
        let candidate = search_reply(query, subset, TOOL_SEARCH_TRIMMED_RESULTS_GUIDANCE);
        if wrapped_reply_fits(&candidate) {
            return candidate;
        }
    }

    // Rung e: the floor. Always fits — bounded query, no result content, static guidance.
    search_reply(query, Vec::new(), TOOL_SEARCH_QUERY_TOO_BROAD_GUIDANCE)
}

impl BridgeKind {
    /// Map a stored bridge name to its kind. Returns `None` for a name that is
    /// not one of the known bridges.
    fn from_provider_name(name: &str) -> Option<Self> {
        match name {
            TOOL_SEARCH_NAME => Some(Self::Search),
            TOOL_DESCRIBE_NAME => Some(Self::Describe),
            DESCRIBE_FIRST_BRIDGE_NAME => Some(Self::DescribeFirst),
            TOOL_CALL_NAME => Some(Self::Call),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedToolTarget {
    definition: ProviderToolDefinition,
    target_call: ProviderToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PromotionScopeKey {
    tenant_id: TenantId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    thread_id: ThreadId,
}

impl PromotionScopeKey {
    fn from_run_context(run_context: &LoopRunContext) -> Self {
        let scope = &run_context.scope;
        Self {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for ToolDisclosureCapabilityPort {
    fn requires_ordered_batch_invocation(&self, invocations: &[LoopRequest]) -> bool {
        self.inner.requires_ordered_batch_invocation(invocations)
    }

    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let state = self.turn_state()?;
        let Some(state) = state.as_ref() else {
            return Ok(Vec::new());
        };
        let (effective_catalog_count, effective_catalog_schema_tokens) =
            state.catalog.effective_metrics(&self.policy);
        // Live token savings = how much of the full (authorized) tool surface we
        // avoided advertising this turn. Lets a benchmark/live run report the
        // real reduction directly from one log line (the fixture benchmark can't,
        // since its names are decoupled from the real core).
        let reduction_pct = if effective_catalog_schema_tokens > 0 {
            100.0
                * (1.0
                    - (f64::from(state.active.advertised_tokens)
                        / f64::from(effective_catalog_schema_tokens)))
        } else {
            0.0
        };
        debug!(
            target: "ironclaw::reborn::context_shadow",
            effective_catalog_count,
            effective_catalog_schema_tokens,
            advertised_tool_count = state.active.definitions.len(),
            advertised_tool_schema_tokens = state.active.advertised_tokens,
            deferred = state.active.deferred,
            reduction_pct,
            "reborn live tool disclosure surface"
        );
        Ok(state.active.definitions.clone())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(tool_call)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure resolving direct deferred provider tool call"
                );
                // Resolve to the catalog's capability id directly. This is the
                // resolvability gate for the gateway pre-check; it must NOT depend
                // on the inner port being able to re-resolve the (unadvertised)
                // provider name, which it cannot for a deferred tool. The real
                // effective_capability_ids / approval expansion are applied later
                // by validate/register, which dispatch the synthesized target call.
                return Ok(ProviderToolCallCapabilityIds::single(
                    target.definition.capability_id,
                ));
            }
            return self.inner.provider_tool_call_capability_ids(tool_call);
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(tool_call)?
        {
            return Ok(ProviderToolCallCapabilityIds {
                provider_capability_id: target.definition.capability_id.clone(),
                effective_capability_ids: vec![target.definition.capability_id],
            });
        }
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(invalid_invocation("bridge tool definition is unavailable"));
        };
        Ok(ProviderToolCallCapabilityIds::single(
            definition.capability_id,
        ))
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(tool_call)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure validating direct deferred provider tool call"
                );
                return self.inner.validate_provider_tool_call(&target.target_call);
            }
            return self.inner.validate_provider_tool_call(tool_call);
        }
        if matches!(
            tool_call.name.as_str(),
            TOOL_SEARCH_NAME | TOOL_DESCRIBE_NAME
        ) {
            return Ok(());
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(tool_call)?
        {
            // A resolved target that fails inner validation must NOT abort the
            // whole provider response — the gateway discards the entire response
            // on a validation error, which turns a recoverable bad-arguments
            // `tool_call` into a run-borking failure. Probe validation for an
            // early diagnostic, but always return Ok: registration falls back to
            // the bridge path on failure, surfacing a recoverable invalid_input
            // at invoke time that the model can correct and retry.
            //
            // NOTE: this makes `validate_provider_tool_call` no longer mean "this
            // will pass" for the bridge path — the real gate is `register`. That
            // muddied contract is a workaround for the gateway's discard-the-whole-
            // response-on-validate-error behavior; the honest fix is upstream (fail
            // only the offending call, not the whole response). Tracked in the
            // context-management design doc under "validate contract"; remove this
            // probe-and-swallow once the gateway stops discarding the response.
            if let Err(error) = self.inner.validate_provider_tool_call(&target.target_call) {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    target = target.definition.name.as_str(),
                    error_kind = ?error.kind,
                    "tool_call target failed inner validation; deferring to recoverable bridge failure"
                );
            }
            return Ok(());
        }
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let RegisterProviderToolCallRequest {
            tool_call,
            activity_id,
        } = request;
        // Inspecting a deferred tool via `capability_info` promotes it for direct
        // use next turn (search → capability_info → direct call). Runs before
        // dispatch so the promotion lands even though the call itself delegates
        // to the inner port below.
        self.note_capability_info_target(&tool_call)?;
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(&tool_call)? {
                // Preserve the model's emitted wire name in the replay when it is
                // already valid (the common `__`-encoded case) so the replayed
                // assistant tool call mirrors what the model generated. Only when
                // the model called the deferred tool by a non-wire-safe form —
                // most often the dotted catalog capability_id like
                // `google-calendar.list_events` — fall back to the resolved
                // definition's canonical name; recording a dotted name fails
                // `validate_provider_tool_name` and borks the run on transcript
                // write.
                let replay_tool_name =
                    replay_provider_tool_name(&tool_call.name, &target.definition.name);
                debug!(
                    tool_name = tool_call.name.as_str(),
                    replay_tool_name = replay_tool_name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure registering direct deferred provider tool call"
                );
                // Describe-first (see `should_describe_first`): a deferred tool
                // called by name with arguments that fail pre-dispatch validation
                // gets its schema instead of a blind dispatch, one-shot per
                // undisclosed tool.
                if self.should_describe_first(&target)? {
                    debug!(
                        tool_name = tool_call.name.as_str(),
                        capability_id = target.definition.capability_id.as_str(),
                        "deferred direct call failed pre-dispatch validation before its schema was disclosed; returning schema (describe-first)"
                    );
                    return self.register_describe_first(
                        &tool_call,
                        replay_tool_name,
                        target.definition.name.as_str(),
                    );
                }
                let mut candidate = self
                    .inner
                    .register_provider_tool_call(register_request(target.target_call, activity_id))
                    .await?;
                candidate.provider_replay = Some(provider_replay_for(&tool_call, replay_tool_name));
                self.record_promotable_input(
                    candidate.input_ref.as_str(),
                    candidate.capability_id.clone(),
                )?;
                return Ok(candidate);
            }
            return self
                .inner
                .register_provider_tool_call(register_request(tool_call, activity_id))
                .await;
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(&tool_call)?
        {
            // The model invoked the `tool_call` bridge itself (a valid wire
            // name); the replay reflects that actual call, not the target.
            let bridge_provider_tool_name = tool_call.name.clone();
            // Describe-first (see `should_describe_first`): same as the
            // direct-deferred path above, but for a tool reached via the bridge.
            if self.should_describe_first(&target)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    target = target.definition.name.as_str(),
                    "tool_call to an undisclosed tool failed pre-dispatch validation; returning schema (describe-first)"
                );
                return self.register_describe_first(
                    &tool_call,
                    bridge_provider_tool_name,
                    target.definition.name.as_str(),
                );
            }
            match self
                .inner
                .register_provider_tool_call(register_request(target.target_call, activity_id))
                .await
            {
                Ok(mut candidate) => {
                    candidate.provider_replay =
                        Some(provider_replay_for(&tool_call, bridge_provider_tool_name));
                    self.record_promotable_input(
                        candidate.input_ref.as_str(),
                        candidate.capability_id.clone(),
                    )?;
                    return Ok(candidate);
                }
                Err(error) => {
                    // The resolved target could not be registered (e.g. malformed
                    // arguments for a deferred tool). Fall back to the bridge path
                    // so the model receives a recoverable invalid_input failure at
                    // invoke time instead of the whole run aborting.
                    debug!(
                        tool_name = tool_call.name.as_str(),
                        error_kind = ?error.kind,
                        "tool_call target registration failed; falling back to recoverable bridge failure"
                    );
                    return self.register_bridge_call(tool_call);
                }
            }
        }
        self.register_bridge_call(tool_call)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        let mut surface = Box::pin(self.inner.visible_capabilities(request)).await?;
        // The inner surface is the full reachable authorized catalog *before* we
        // narrow the advertised `descriptors` below. Capture it as the call-time
        // "callable" view so the model-visible capability filter authorizes
        // bridge / forgiving-direct calls to catalog tools the model legitimately
        // reaches this turn but that aren't advertised. Without this, a resumed
        // run whose discovered tools dropped off the advertised surface has its
        // retry hard-rejected as "outside the model-visible capability view".
        let callable_capability_ids: Vec<CapabilityId> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let mut state = self.refresh_turn_state(&surface)?;
        let Some(state) = state.as_mut() else {
            surface.callable_capability_ids = Some(callable_capability_ids);
            return Ok(surface);
        };
        let active_or_disclosed_descriptors = state
            .catalog
            .active_or_disclosed_descriptors(&state.active, &state.disclosed_names);
        let active_or_disclosed_ids: BTreeSet<CapabilityId> = active_or_disclosed_descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let bridge_descriptors: Vec<_> = active_or_disclosed_descriptors
            .into_iter()
            .filter(|descriptor| is_bridge_capability_id(&descriptor.capability_id))
            .collect();
        surface.descriptors.retain(|descriptor| {
            active_or_disclosed_ids.contains(&descriptor.capability_id)
                && !is_bridge_capability_id(&descriptor.capability_id)
        });
        let mut advertised_ids: BTreeSet<CapabilityId> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        for descriptor in bridge_descriptors {
            if advertised_ids.insert(descriptor.capability_id.clone()) {
                surface.descriptors.push(descriptor);
            }
        }
        // Callable = the full reachable catalog (captured above) UNION the tools
        // actually advertised this turn UNION every bridge capability. Deferred
        // surfaces advertise all three bridges, and describe-first routing can
        // also synthesize a response through `tool_describe`'s capability id.
        // Keeping every bridge callable authorizes both paths at the executor
        // visibility gate.
        let mut callable: BTreeSet<CapabilityId> = callable_capability_ids.into_iter().collect();
        callable.extend(
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone()),
        );
        callable.extend(
            bridge_tool_definitions()
                .into_iter()
                .map(|definition| definition.capability_id),
        );
        surface.callable_capability_ids = Some(callable.into_iter().collect());
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        if !is_bridge_capability_id(&request.capability_id) {
            let target_capability_id =
                self.target_capability_id_for_input_ref(request.input_ref.as_str())?;
            // Chain-boxing: each port delegation is boxed so the stacked
            // decorator chain never compiles into a single oversized poll
            // frame (see reborn_integration_model_recovery stack-overflow).
            let resolution = Box::pin(self.inner.invoke_capability(request)).await?;
            self.promote_target_after_resolution(target_capability_id, &resolution)?;
            return Ok(resolution);
        }
        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        Box::pin(self.invoke_bridge(request)).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        // The executor clears this flag only for a model-emitted parallel
        // batch. Preserve invocation order in the returned vector while
        // allowing the read-only bridge futures to overlap.
        if !request.stop_on_first_suspension {
            let resolutions = join_all(
                request
                    .invocations
                    .into_iter()
                    .map(|invocation| self.invoke_capability(invocation)),
            )
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
            return Ok(ResolutionBatch {
                resolutions,
                stopped_on_suspension: false,
            });
        }

        // Preserve batch-specific behavior (for example coalesced subagent
        // gates) when every invocation belongs to the inner port.
        if request
            .invocations
            .iter()
            .all(|invocation| !is_bridge_capability_id(&invocation.capability_id))
        {
            // Chain-boxing: each port delegation is boxed so the stacked
            // decorator chain never compiles into a single oversized poll
            // frame (see reborn_integration_model_recovery stack-overflow).
            return Box::pin(self.invoke_inner_batch_preserving_promotions(request)).await;
        }

        let mut resolutions = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            // Chain-boxing: each port delegation is boxed so the stacked
            // decorator chain never compiles into a single oversized poll
            // frame (see reborn_integration_model_recovery stack-overflow).
            let resolution = Box::pin(self.invoke_capability(invocation)).await?;
            let parks = resolution.parks();
            resolutions.push(resolution);
            if parks {
                return Ok(ResolutionBatch {
                    resolutions,
                    stopped_on_suspension: true,
                });
            }
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension: false,
        })
    }
}

impl ToolDisclosureCapabilityPort {
    fn target_capability_id_for_input_ref(
        &self,
        input_ref: &str,
    ) -> Result<Option<CapabilityId>, AgentLoopHostError> {
        self.tool_call_target_inputs
            .lock()
            .map_err(|e| {
                invalid_invocation(format!("tool_call target store lock is poisoned: {e}"))
            })
            .map(|targets| targets.get(input_ref).cloned())
    }

    fn promote_target_after_resolution(
        &self,
        target_capability_id: Option<CapabilityId>,
        resolution: &Resolution,
    ) -> Result<(), AgentLoopHostError> {
        // Promote on a completed dispatch OR a gate/park (approval/auth/
        // resource or parked work). A tool the model dispatched that paused for
        // a user action is just as "earned" as a completed one, and it MUST
        // stay visible across the Blocked/Suspended resume: otherwise the
        // per-turn disclosed set resets, the tool drops off the model-visible
        // surface, and the model's retry is hard-rejected by the visible-surface
        // filter ("outside the model-visible capability view") — discarding the
        // whole response and borking the run. A hard *failure* (a Done with a
        // recoverable-failure verdict) still does NOT promote (the model may
        // abandon it), so this does not drift toward advertising every
        // discovered tool — only ones the model actually invoked. `parks()` is
        // the gate+suspension predicate (the loop enum's old `is_suspension()`
        // also lumped gates in).
        if (matches!(resolution, Resolution::Done(outcome) if outcome.verdict.is_success())
            || resolution.parks())
            && let Some(capability_id) = target_capability_id
        {
            self.promote_target(&capability_id)?;
        }
        Ok(())
    }

    async fn invoke_inner_batch_preserving_promotions(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let target_capability_ids = request
            .invocations
            .iter()
            .map(|invocation| {
                self.target_capability_id_for_input_ref(invocation.input_ref.as_str())
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Chain-boxing: each port delegation is boxed so the stacked
        // decorator chain never compiles into a single oversized poll
        // frame (see reborn_integration_model_recovery stack-overflow).
        let batch = Box::pin(self.inner.invoke_capability_batch(request)).await?;
        for (resolution, target_capability_id) in
            batch.resolutions.iter().zip(target_capability_ids)
        {
            self.promote_target_after_resolution(target_capability_id, resolution)?;
        }
        Ok(batch)
    }

    fn turn_state(
        &self,
    ) -> Result<MutexGuard<'_, Option<ToolDisclosureTurnState>>, AgentLoopHostError> {
        let mut guard = self.lock_turn_state()?;
        let stale_turn = guard
            .as_ref()
            .is_some_and(|state| state.turn_id != self.run_context.turn_id);
        if stale_turn {
            *guard = None;
        }
        Ok(guard)
    }

    fn refresh_turn_state(
        &self,
        surface: &VisibleCapabilitySurface,
    ) -> Result<MutexGuard<'_, Option<ToolDisclosureTurnState>>, AgentLoopHostError> {
        let guard = self.lock_turn_state()?;
        let current_surface = guard.as_ref().is_some_and(|state| {
            state.turn_id == self.run_context.turn_id
                && state.surface_version.as_ref() == Some(&surface.version)
        });
        if current_surface {
            return Ok(guard);
        }
        drop(guard);
        let authorized_capability_ids = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        self.rebuild_turn_state(surface.version.clone(), authorized_capability_ids)
    }

    fn rebuild_turn_state(
        &self,
        surface_version: CapabilitySurfaceVersion,
        authorized_capability_ids: BTreeSet<CapabilityId>,
    ) -> Result<MutexGuard<'_, Option<ToolDisclosureTurnState>>, AgentLoopHostError> {
        // The visible-surface version commits to full descriptor metadata,
        // including schemas. Bridge calls reuse the state until the owning
        // visible-capability refresh reports a new version. Definition retrieval
        // and canonical schema hashing therefore happen only on a real refresh,
        // and remain outside the state critical section.
        let definitions = self.inner.tool_definitions()?;
        let authorized_definitions: Vec<_> = definitions
            .into_iter()
            .filter(|definition| authorized_capability_ids.contains(&definition.capability_id))
            .collect();
        let fingerprint = definitions_fingerprint(&authorized_definitions);
        let mut guard = self.lock_turn_state()?;
        // Fit and cache retrieval only over the effective authorized corpus.
        // Denied schemas therefore cannot affect IDF, ordering, counts, cache
        // invalidation, or search-index construction work.
        let same_turn = guard
            .as_ref()
            .map(|state| state.turn_id == self.run_context.turn_id)
            .unwrap_or(false);
        let rebuild = guard
            .as_ref()
            .map(|state| {
                state.turn_id != self.run_context.turn_id
                    || state.surface_version.as_ref() != Some(&surface_version)
                    || state.definitions_fingerprint != fingerprint
            })
            .unwrap_or(true);
        if rebuild {
            let index_started_at = std::time::Instant::now();
            let effective_pins = if self.mode.includes_profile_pins() {
                self.profile_pins.as_slice()
            } else {
                &[]
            };
            let catalog = CapabilityCatalog::new(&authorized_definitions, effective_pins);
            let search_index = AuthorizedToolSearchIndex::new(authorized_definitions.iter());
            debug!(
                target: "ironclaw::reborn::tool_search",
                authorized_document_count = authorized_definitions.len(),
                index_build_micros = index_started_at.elapsed().as_micros(),
                metadata_fingerprint = fingerprint,
                "rebuilt authorized deferred-tool search index"
            );
            let promoted = self.promoted_for_scope()?;
            let active =
                select_active_set_for_mode(&catalog, &promoted, self.caps, &self.policy, self.mode);
            // Preserve disclosure progress across a same-turn refresh (a tool the
            // model already described stays disclosed); a genuine turn change
            // starts fresh.
            let (disclosed_names, search_ranks) = guard
                .take()
                .filter(|_| same_turn)
                .map(|state| (state.disclosed_names, state.search_ranks))
                .unwrap_or((BTreeSet::new(), BTreeMap::new()));
            *guard = Some(ToolDisclosureTurnState {
                turn_id: self.run_context.turn_id,
                definitions_fingerprint: fingerprint,
                surface_version: Some(surface_version),
                catalog,
                search_index,
                active,
                disclosed_names,
                search_ranks,
            });
        }
        Ok(guard)
    }

    fn lock_turn_state(
        &self,
    ) -> Result<MutexGuard<'_, Option<ToolDisclosureTurnState>>, AgentLoopHostError> {
        self.turn_state.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure turn state lock is poisoned: {e}"),
            )
        })
    }

    fn promoted_for_scope(&self) -> Result<PromotedSet, AgentLoopHostError> {
        let key = PromotionScopeKey::from_run_context(&self.run_context);
        let guard = self.promoted_by_scope.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure promoted set lock is poisoned: {e}"),
            )
        })?;
        Ok(guard.get(&key).cloned().unwrap_or_default())
    }

    fn promote_target(&self, capability_id: &CapabilityId) -> Result<(), AgentLoopHostError> {
        let target = {
            let guard = self.turn_state()?;
            let Some(state) = guard.as_ref() else {
                return Ok(());
            };
            state
                .catalog
                .definition_by_capability_id(capability_id)
                .map(|definition| {
                    let name = definition.name.to_string();
                    let selected_rank = state.search_ranks.get(&name).copied();
                    (name, selected_rank)
                })
        };
        let Some((name, selected_rank)) = target else {
            return Ok(());
        };
        if let Some(selected_rank) = selected_rank {
            debug!(
                target: "ironclaw::reborn::tool_search",
                selected_rank,
                selection_action = "invoke",
                "observed deferred-tool selection without logging tool or query metadata"
            );
        }
        let key = PromotionScopeKey::from_run_context(&self.run_context);
        let mut guard = self.promoted_by_scope.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure promoted set lock is poisoned: {e}"),
            )
        })?;
        guard.entry(key).or_default().push(name);
        Ok(())
    }

    /// When the model inspects a deferred tool via `capability_info`, treat it as
    /// intent to use that tool: disclose it this turn and promote it for the scope
    /// so it becomes advertised with its full schema and directly callable next
    /// turn — the `tool_search` → `capability_info` → direct-call flow. This is the
    /// same disclose+promote a `tool_describe` / successful `tool_call` already
    /// does, wired onto the `capability_info` path so that path can stand alone.
    /// No-op for a non-`capability_info` call or a target not in the catalog.
    fn note_capability_info_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if tool_call.name.as_str() != CAPABILITY_INFO_NAME {
            return Ok(());
        }
        let Some(target_name) = tool_call
            .arguments
            .get("name")
            .or_else(|| tool_call.arguments.get("capability_id"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };
        let capability_id = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(());
            };
            // `capability_info`'s `name` may be the provider tool name (dotted or
            // encoded) or the canonical capability id — resolve either.
            let resolved = state
                .catalog
                .search_result(target_name)
                .map(|result| (result.name, result.capability_id))
                .or_else(|| {
                    CapabilityId::new(target_name)
                        .ok()
                        .and_then(|capability_id| {
                            state
                                .catalog
                                .definition_by_capability_id(&capability_id)
                                .map(|definition| {
                                    (
                                        definition.name.to_string(),
                                        definition.capability_id.clone(),
                                    )
                                })
                        })
                });
            let Some((name, capability_id)) = resolved else {
                return Ok(());
            };
            state.disclosed_names.insert(name);
            capability_id
        };
        // Field named `tool`, not `target`: a first-argument `target = …` in a
        // tracing macro is the field form of the metadata-target syntax (#7146),
        // so a field that genuinely means "the tool this call targeted" has to
        // be spelled differently or it reads as a mis-typed target.
        debug!(
            tool = target_name,
            capability_id = capability_id.as_str(),
            "capability_info inspected a deferred tool; disclosing + promoting it for direct use"
        );
        self.promote_target(&capability_id)
    }

    fn record_promotable_input(
        &self,
        input_ref: &str,
        capability_id: CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        self.tool_call_target_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("tool target store lock is poisoned: {e}")))?
            .insert(input_ref.to_string(), capability_id);
        Ok(())
    }

    /// Whether the model has loaded this tool's schema this turn (via
    /// `tool_search` / `tool_describe` / a prior describe-first). Gates
    /// describe-first so it fires at most once per undisclosed tool: once the
    /// schema is in context, a still-invalid call dispatches and fails through the
    /// normal path the no-progress detector can count.
    fn is_disclosed(&self, name: &str) -> Result<bool, AgentLoopHostError> {
        let guard = self.turn_state()?;
        Ok(guard
            .as_ref()
            .map(|state| state.disclosed_names.contains(name))
            .unwrap_or(false))
    }

    fn register_bridge_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(invalid_invocation("bridge tool definition is unavailable"));
        };
        let digest_input = provider_call_digest_input(
            &tool_call.id,
            tool_call.name.as_str(),
            &tool_call.arguments,
        );
        let digest = ironclaw_host_api::approval::sha256_digest_token(digest_input.as_bytes());
        let input_ref = CapabilityInputRef::new(format!("{DISCLOSURE_INPUT_PREFIX}{digest}"))
            .map_err(|e| {
                invalid_invocation(format!("bridge input ref could not be represented: {e}"))
            })?;
        self.bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .insert(
                input_ref.as_str().to_string(),
                BridgeInvocation {
                    kind: BridgeKind::from_provider_name(tool_call.name.as_str()).ok_or_else(
                        || invalid_invocation("bridge tool definition is unavailable"),
                    )?,
                    arguments: tool_call.arguments.clone(),
                },
            );
        let surface_version = self.current_surface_version()?;
        Ok(CapabilityCallCandidate {
            activity_id: CapabilityActivityId::new(),
            surface_version,
            capability_id: definition.capability_id,
            input_ref,
            effective_capability_ids: Vec::new(),
            provider_replay: Some(provider_replay_for(&tool_call, tool_call.name.clone())),
        })
    }

    /// Whether a resolved deferred call should be answered with its schema
    /// (describe-first) rather than dispatched blind.
    ///
    /// This is the blind-call regression tool disclosure introduces:
    /// pre-disclosure the full schema was always in context so the model filled
    /// required fields; with schemas deferred the model calls by name alone,
    /// omitting required arguments and looping on the opaque validation error.
    /// True when the target's schema has NOT been disclosed this turn AND its
    /// arguments fail pre-dispatch validation. Once disclosed, a still-invalid
    /// retry dispatches and fails normally, so the no-progress detector still
    /// observes the repeated failure. Well-formed blind calls return false and
    /// dispatch directly, adding no round-trip on correct calls.
    fn should_describe_first(
        &self,
        target: &ResolvedToolTarget,
    ) -> Result<bool, AgentLoopHostError> {
        if self.is_disclosed(target.definition.name.as_str())? {
            return Ok(false);
        }
        // Resolution failure: the inner port can't even resolve the call.
        if self
            .inner
            .validate_provider_tool_call(&target.target_call)
            .is_err()
        {
            return Ok(true);
        }
        // Input-schema failure: the call resolves, but its arguments don't satisfy
        // the tool's parameter schema. Pre-disclosure the full schema was always
        // in context so the model formatted the call; deferred, it calls the tool
        // blind and a nested-shape error (e.g. a `schedule` `oneOf`) hands back no
        // schema to recover from, so a weak model guesses the shape and spirals.
        // Probe the arguments against the catalog schema and describe-first on a
        // mismatch so the model's retry carries the real schema + examples.
        Ok(!arguments_satisfy_schema(
            &target.target_call.arguments,
            &target.definition.parameters,
        ))
    }

    /// Register a deferred call whose arguments failed pre-dispatch validation as
    /// an auto-schema (describe-first) bridge response rather than a blind
    /// dispatch. `invoke_describe_first` returns the tool's parameter schema and
    /// marks it disclosed, so the model's retry carries the required fields.
    ///
    /// The candidate borrows the `tool_describe` bridge capability id so
    /// `invoke_capability` routes it to `invoke_bridge`; the stored
    /// `BridgeInvocation` name (`DESCRIBE_FIRST_BRIDGE_NAME`) distinguishes it from
    /// a genuine `tool_describe`. The replay mirrors the model's actual call —
    /// `replay_tool_name` is the wire-safe name the caller already resolved (the
    /// bridge name, or the canonical definition name for a dotted direct call).
    fn register_describe_first(
        &self,
        tool_call: &ProviderToolCall,
        replay_tool_name: ProviderToolName,
        target_name: &str,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name.as_str() == TOOL_DESCRIBE_NAME)
        else {
            return Err(invalid_invocation(
                "tool_describe bridge definition is unavailable",
            ));
        };
        // Distinct digest input so an auto-schema input never collides with a
        // genuine bridge input for the same provider call id.
        let digest_input = provider_call_digest_input(
            &format!("{}:auto-schema", tool_call.id),
            target_name,
            &tool_call.arguments,
        );
        let digest = ironclaw_host_api::approval::sha256_digest_token(digest_input.as_bytes());
        let input_ref = CapabilityInputRef::new(format!("{DISCLOSURE_INPUT_PREFIX}{digest}"))
            .map_err(|e| {
                invalid_invocation(format!(
                    "auto-schema input ref could not be represented: {e}"
                ))
            })?;
        self.bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .insert(
                input_ref.as_str().to_string(),
                BridgeInvocation {
                    kind: BridgeKind::DescribeFirst,
                    arguments: json!({ "name": target_name }),
                },
            );
        let surface_version = self.current_surface_version()?;
        Ok(CapabilityCallCandidate {
            activity_id: CapabilityActivityId::new(),
            surface_version,
            capability_id: definition.capability_id,
            input_ref,
            effective_capability_ids: Vec::new(),
            provider_replay: Some(provider_replay_for(tool_call, replay_tool_name)),
        })
    }

    fn current_surface_version(&self) -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
        let guard = self.turn_state()?;
        guard
            .as_ref()
            .and_then(|state| state.surface_version.clone())
            .ok_or_else(|| invalid_invocation("capability surface is unavailable"))
    }

    async fn invoke_bridge(&self, request: LoopRequest) -> Result<Resolution, AgentLoopHostError> {
        let bridge = self
            .bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .get(request.input_ref.as_str())
            .cloned()
            .ok_or_else(|| invalid_invocation("bridge input is unavailable"))?;
        match bridge.kind {
            BridgeKind::Search => self.invoke_tool_search(&request, &bridge).await,
            BridgeKind::Describe => self.invoke_tool_describe(&request, &bridge).await,
            BridgeKind::DescribeFirst => self.invoke_describe_first(&request, &bridge).await,
            BridgeKind::Call if decode_tool_call_arguments(&bridge.arguments).is_none() => {
                Ok(failed_recoverable(
                    FailureKind::InputEncode,
                    "tool_call arguments must be a JSON object encoded as a string",
                ))
            }
            BridgeKind::Call => Ok(failed_recoverable(
                FailureKind::UnknownCapability,
                "tool_call target is not a known tool; use tool_search to find the correct tool name",
            )),
        }
    }

    async fn invoke_tool_search(
        &self,
        request: &LoopRequest,
        bridge: &BridgeInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        let Some(query) = bridge.arguments.get("query").and_then(Value::as_str) else {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "tool_search requires query",
            ));
        };
        let query = query.trim();
        if query.is_empty() {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "tool_search requires query",
            ));
        }
        if query.len() > MAX_SEARCH_QUERY_BYTES {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "tool_search query is too long",
            ));
        }
        let limit = bridge
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(TOOL_SEARCH_INLINE_RESULT_LIMIT)
            .clamp(1, 50);
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_recoverable(
                    FailureKind::Unavailable,
                    "tool catalog is unavailable",
                ));
            };
            // In complete-signature mode the reply carries at most
            // TOOL_SEARCH_INLINE_RESULT_LIMIT results regardless of the caller's `limit` (see
            // `bounded_search_output`'s own `.take(...)`), and `AuthorizedToolSearchIndex::search`
            // scores the WHOLE corpus before truncating to `limit` — so narrowing `limit` here
            // cannot change which ranks come first, only how many the index bothers to
            // canonicalize (`state.catalog.search_result`) below. Capping avoids materializing and
            // immediately discarding ranks 4-50 on every complete-signature search. The Compact
            // control arm (unbounded by design, see below) must keep using the caller's full
            // `limit`.
            let limit_for_search = if self.mode.includes_complete_signatures() {
                limit.min(TOOL_SEARCH_INLINE_RESULT_LIMIT)
            } else {
                limit
            };
            let search_started_at = std::time::Instant::now();
            let outcome = state.search_index.search(query, limit_for_search);
            debug!(
                target: "ironclaw::reborn::tool_search",
                query_class = outcome.query_class.as_str(),
                empty_result = outcome.names.is_empty(),
                returned_count = outcome.names.len(),
                query_latency_micros = search_started_at.elapsed().as_micros(),
                "ranked deferred-tool search without logging raw query or schemas"
            );
            let mut ranked_results = Vec::new();
            for (index, name) in outcome.names.into_iter().enumerate() {
                state
                    .search_ranks
                    .insert(name.clone(), index.saturating_add(1));
                if let Some(result) = state.catalog.search_result(&name) {
                    ranked_results.push(result);
                }
            }
            let output = if self.mode.includes_complete_signatures() {
                bounded_search_output(query, ranked_results)
            } else {
                // ToolDisclosureMode::Compact is a deliberately unbounded measurement control
                // arm (tool_disclosure_mode.rs), reachable only via REBORN_TOOL_DISCLOSURE=compact
                // (production default is Namespaces). compact_search_results maps every ranked
                // result through compact_result_fields with no description truncation, no
                // `.take(...)`, and no byte-budget check, on purpose: truncating here would
                // change the control arm's wire bytes and invalidate the A/B comparison it exists
                // to run. The bounded, first-look-envelope construction above applies only to the
                // production arms.
                json!({
                    "query": query,
                    "results": compact_search_results(ranked_results),
                })
            };
            if let Some(results) = output.get("results").and_then(Value::as_array) {
                for result in results {
                    if result.get("schema_complete").and_then(Value::as_bool) == Some(true)
                        && let Some(name) = result.get("name").and_then(Value::as_str)
                    {
                        state.disclosed_names.insert(name.to_string());
                    }
                }
            }
            output
        };
        self.completed_bridge_result(request, output, "tool_search returned catalog matches")
            .await
    }

    async fn invoke_tool_describe(
        &self,
        request: &LoopRequest,
        bridge: &BridgeInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        let Some(name) = bridge.arguments.get("name").and_then(Value::as_str) else {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "tool_describe requires name",
            ));
        };
        if is_bridge_name(name) {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "tool_describe target must not be a bridge",
            ));
        }
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_recoverable(
                    FailureKind::Unavailable,
                    "tool catalog is unavailable",
                ));
            };
            let Some(result) = state.catalog.search_result(name) else {
                return Ok(failed_recoverable(
                    FailureKind::UnknownCapability,
                    "tool_describe target is unknown; use tool_search to find the correct tool name",
                ));
            };
            // #5712: same message as a truly unknown name — a narrowed profile
            // must not learn that a non-allowlisted tool exists.
            if !self.policy.permits_capability_id(&result.capability_id) {
                return Ok(failed_recoverable(
                    FailureKind::UnknownCapability,
                    "tool_describe target is unknown; use tool_search to find the correct tool name",
                ));
            }
            if let Some(selected_rank) = state.search_ranks.get(&result.name).copied() {
                debug!(
                    target: "ironclaw::reborn::tool_search",
                    selected_rank,
                    selection_action = "describe",
                    "observed deferred-tool selection without logging tool or query metadata"
                );
            }
            state.disclosed_names.insert(name.to_string());
            json!({
                "name": result.name,
                "capability_id": result.capability_id.as_str(),
                "description": result.description,
                "required": result.required_params,
                "parameters": result.parameters,
            })
        };
        self.completed_bridge_result(request, output, "tool_describe returned schema")
            .await
    }

    /// Invoke an auto-schema (describe-first) bridge: return the target tool's
    /// parameter schema and mark it disclosed, so the model's retry carries the
    /// required fields. Mirrors `invoke_tool_describe` but its note tells the
    /// model the schema was loaded automatically because its call did not match —
    /// the schema is rendered exactly when the model needs it, restoring the
    /// pre-disclosure guarantee for the one call that got it wrong.
    async fn invoke_describe_first(
        &self,
        request: &LoopRequest,
        bridge: &BridgeInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        let Some(name) = bridge.arguments.get("name").and_then(Value::as_str) else {
            return Ok(failed_recoverable(
                FailureKind::InputEncode,
                "auto-schema requires a target name",
            ));
        };
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_recoverable(
                    FailureKind::Unavailable,
                    "tool catalog is unavailable",
                ));
            };
            let Some(result) = state.catalog.search_result(name) else {
                return Ok(failed_recoverable(
                    FailureKind::UnknownCapability,
                    "auto-schema target is unknown",
                ));
            };
            state.disclosed_names.insert(result.name.clone());
            json!({
                "status": "schema_loaded",
                "note": "Your previous arguments did not match this tool's schema (its schema had not been loaded yet). Here is the parameter schema — call the tool again with the required arguments.",
                "name": result.name,
                "capability_id": result.capability_id.as_str(),
                "description": result.description,
                "required": result.required_params,
                "parameters": result.parameters,
            })
        };
        self.completed_bridge_result(request, output, "auto-loaded tool schema before invocation")
            .await
    }

    async fn completed_bridge_result(
        &self,
        request: &LoopRequest,
        output: Value,
        safe_summary: &'static str,
    ) -> Result<Resolution, AgentLoopHostError> {
        let write = self
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &self.run_context,
                input_ref: &request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        Ok(resolution::completed(
            write.result_ref,
            safe_summary.to_string(),
            CapabilityProgress::MadeProgress,
            false,
            write.byte_len,
            write.output_digest,
            write.model_observation,
        ))
    }

    fn target_call(
        &self,
        tool_call: &ProviderToolCall,
        target: &ProviderToolDefinition,
        arguments: Value,
    ) -> ProviderToolCall {
        let digest_input =
            provider_call_digest_input(&tool_call.id, target.name.as_str(), &arguments);
        let target_id = ironclaw_host_api::approval::sha256_digest_token(digest_input.as_bytes());
        ProviderToolCall {
            provider_id: tool_call.provider_id.clone(),
            provider_model_id: tool_call.provider_model_id.clone(),
            turn_id: tool_call.turn_id.clone(),
            id: format!("{}:{target_id}", tool_call.id),
            name: target.name.clone(),
            arguments,
            response_reasoning: tool_call.response_reasoning.clone(),
            reasoning: tool_call.reasoning.clone(),
            signature: tool_call.signature.clone(),
        }
    }

    fn allowed_tool_call_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ResolvedToolTarget>, AgentLoopHostError> {
        let Some(name) = tool_call.arguments.get("name").and_then(Value::as_str) else {
            return Ok(None);
        };
        if is_bridge_name(name) {
            return Ok(None);
        }
        let Some(arguments) = decode_tool_call_arguments(&tool_call.arguments) else {
            return Ok(None);
        };
        let guard = self.turn_state()?;
        let Some(state) = guard.as_ref() else {
            return Ok(None);
        };
        // Forgiving resolution: resolve any allowlisted tool the catalog knows
        // by name, regardless of whether it has been advertised or discovered
        // this turn. A catalog-known but non-allowlisted target must follow the
        // same recoverable bridge path as a nonexistent target; otherwise this
        // synthetic bridge becomes an existence oracle for targets excluded
        // from the filtered base surface.
        // A *direct* call to an undisclosed tool already resolves via
        // `direct_deferred_target`, so the `tool_call` bridge must not be
        // stricter than the direct path. Requiring prior disclosure here was a
        // dead end: a model that calls `tool_call` before `tool_search`/
        // `tool_describe` got a generic "invalid_input" with no recovery hint and
        // looped until the run died. Resolving forgivingly lets the call dispatch
        // and surface the tool's *real* schema error (with repairs) — which the
        // model can act on — and earns promotion on success via the register
        // path's `record_promotable_input`. Safety/approval/auth gates still run
        // at dispatch, so this is a token-economy boundary, not a security one.
        let Some(definition) = self.catalog_target(state, name) else {
            return Ok(None);
        };
        if !self.policy.permits_capability_id(&definition.capability_id) {
            return Ok(None);
        }
        let target_call = self.target_call(tool_call, &definition, arguments);
        Ok(Some(ResolvedToolTarget {
            definition,
            target_call,
        }))
    }

    fn direct_deferred_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ResolvedToolTarget>, AgentLoopHostError> {
        if is_bridge_name(tool_call.name.as_str()) {
            return Ok(None);
        }
        let guard = self.turn_state()?;
        let Some(state) = guard.as_ref() else {
            debug!(
                tool_name = tool_call.name.as_str(),
                "reborn tool disclosure direct-deferred miss: no turn state"
            );
            return Ok(None);
        };
        let Some(definition) = self.catalog_target(state, tool_call.name.as_str()) else {
            // DIAGNOSTIC (temporary): the model called a non-bridge tool that the
            // catalog could not resolve by name. Sample catalog names + capability
            // ids that share the called tool's provider prefix, so a name-form
            // mismatch (dotted vs `__`-encoded) is visible vs. genuinely absent.
            let prefix: String = tool_call
                .name
                .as_str()
                .chars()
                .take_while(|c| *c != '_' && *c != '.' && *c != '-')
                .collect();
            let sample: Vec<String> = state
                .catalog
                .definitions()
                .filter(|definition| {
                    definition.name.as_str().starts_with(&prefix)
                        || definition.capability_id.as_str().starts_with(&prefix)
                })
                .map(|definition| {
                    format!("{}|{}", definition.name, definition.capability_id.as_str())
                })
                .take(8)
                .collect();
            debug!(
                tool_name = tool_call.name.as_str(),
                catalog_len = state.catalog.len(),
                prefix = prefix.as_str(),
                prefix_matches = ?sample,
                "reborn tool disclosure direct-deferred miss: not found in catalog by name"
            );
            return Ok(None);
        };
        let active = state
            .active
            .definitions
            .iter()
            .any(|candidate| candidate.name == tool_call.name);
        if active {
            // Normal path: the tool is advertised, so the inner port dispatches
            // it directly. Not a forgiving-path case.
            Ok(None)
        } else {
            let target_call = self.target_call(tool_call, &definition, tool_call.arguments.clone());
            Ok(Some(ResolvedToolTarget {
                definition,
                target_call,
            }))
        }
    }

    fn catalog_target(
        &self,
        state: &ToolDisclosureTurnState,
        provider_name: &str,
    ) -> Option<ProviderToolDefinition> {
        state
            .catalog
            .definition_by_name(provider_name)
            .or_else(|| {
                state
                    .catalog
                    .definitions()
                    .find(|definition| definition_matches_provider_name(definition, provider_name))
            })
            .cloned()
    }
}

/// Whether `arguments` satisfy `schema`, used only as a describe-first *assist*
/// (never as a gate).
///
/// Conservative by design: if the schema can't be compiled — an unresolved
/// `$ref`, a dialect `jsonschema` rejects — we return `true` ("satisfied") so the
/// call dispatches normally and the real capability-input validator remains the
/// single source of truth. This probe only decides whether to hand the model the
/// schema early; a false negative would merely block that assist, never a call.
fn arguments_satisfy_schema(arguments: &Value, schema: &Value) -> bool {
    match jsonschema::validator_for(schema) {
        Ok(validator) => validator.is_valid(arguments),
        Err(_) => true,
    }
}

/// Choose the wire name to record in a forgiving direct-deferred replay.
///
/// Preserve the model's emitted name when it is already a valid provider tool
/// name (the common `__`-encoded case) so the replayed assistant tool call
/// faithfully mirrors what the model generated. Only when the model called the
/// deferred tool by a non-wire-safe form — most often the dotted catalog
/// `capability_id` such as `google-calendar.list_events` — fall back to the
/// resolved definition's canonical name, which is always wire-safe. Recording a
/// dotted name fails `validate_provider_tool_name` and borks the run on the
/// assistant transcript / provider-error result-ref write.
fn replay_provider_tool_name(
    called_name: &ProviderToolName,
    definition_name: &ProviderToolName,
) -> ProviderToolName {
    if ironclaw_safety::validate_provider_tool_name(called_name.as_str()).is_ok() {
        called_name.clone()
    } else {
        definition_name.clone()
    }
}

/// Build the provider-call replay metadata recorded with a capability candidate.
///
/// `provider_tool_name` is the wire name the replay (and any provider-error
/// result reference) serializes into the transcript. It MUST be a canonical
/// provider tool name (`[A-Za-z0-9_-]`) because `validate_provider_tool_name`
/// rejects anything else and a failed transcript write borks the whole run. On
/// the forgiving direct-deferred path the model may have called a deferred tool
/// by its dotted catalog `capability_id` (e.g. `google-calendar.list_events`);
/// callers there must pass the resolved definition's `name` (the `__`-encoded
/// wire name), NOT the raw `tool_call.name`. Bridge/normal paths pass the
/// already-valid `tool_call.name`.
fn provider_replay_for(
    tool_call: &ProviderToolCall,
    provider_tool_name: ProviderToolName,
) -> ProviderToolCallReplay {
    ProviderToolCallReplay {
        provider_id: tool_call.provider_id.clone(),
        provider_model_id: tool_call.provider_model_id.clone(),
        provider_turn_id: tool_call.turn_id.clone().unwrap_or_default(),
        provider_call_id: tool_call.id.clone(),
        provider_tool_name,
        arguments: tool_call.arguments.clone(),
        response_reasoning: tool_call.response_reasoning.clone(),
        reasoning: tool_call.reasoning.clone(),
        signature: tool_call.signature.clone(),
    }
}

/// Build the inner-port registration request for a synthesized target call,
/// preserving any activity identity the gateway bound to the bridge call so the
/// inner port registers the dispatched target under the same id.
fn register_request(
    tool_call: ProviderToolCall,
    activity_id: Option<CapabilityActivityId>,
) -> RegisterProviderToolCallRequest {
    match activity_id {
        Some(activity_id) => RegisterProviderToolCallRequest::for_activity(tool_call, activity_id),
        None => RegisterProviderToolCallRequest::new(tool_call),
    }
}

fn provider_call_digest_input(provider_call_id: &str, name: &str, arguments: &Value) -> String {
    json!({
        "provider_call_id": provider_call_id,
        "name": name,
        "arguments": canonicalize_json(arguments),
    })
    .to_string()
}

/// Decode the provider-safe `tool_call.arguments` string while continuing to
/// accept the original object form from recorded replays and in-flight callers.
fn decode_tool_call_arguments(bridge_arguments: &Value) -> Option<Value> {
    match bridge_arguments.get("arguments") {
        None => Some(json!({})),
        Some(Value::String(encoded)) => serde_json::from_str::<Value>(encoded)
            .ok()
            .filter(Value::is_object),
        Some(value @ Value::Object(_)) => Some(value.clone()),
        Some(_) => None,
    }
}

fn failed_recoverable(kind: FailureKind, summary: &'static str) -> Resolution {
    resolution::failed(
        kind,
        summary.to_string(),
        CapabilityFailureDetail::Diagnostic {
            text: summary.to_string(),
        },
    )
}

fn invalid_invocation(summary: impl Into<String>) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_satisfy_schema_gates_describe_first_on_nested_shape() {
        // trigger_create's `schedule` must be an object (oneOf cron/once); a weak
        // model that calls it deferred often sends a bare cron string. That must
        // read as "does not satisfy" so describe-first hands over the schema.
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "schedule": {
                    "oneOf": [
                        {"type": "object", "properties": {"kind": {"const": "cron"}}, "required": ["kind", "expression"]},
                        {"type": "object", "properties": {"kind": {"const": "once"}}, "required": ["kind", "at"]}
                    ]
                }
            },
            "required": ["name", "schedule"]
        });
        assert!(
            !arguments_satisfy_schema(&json!({"name": "r", "schedule": "*/30 * * * *"}), &schema),
            "a bare-string schedule must fail the object oneOf → describe-first"
        );
        assert!(
            arguments_satisfy_schema(
                &json!({"name": "r", "schedule": {"kind": "cron", "expression": "*/30 * * * *"}}),
                &schema
            ),
            "the correct object shape must satisfy the schema → dispatch directly"
        );
        // Unresolved $ref / uncompilable schema is treated as satisfied (assist,
        // never a gate): the real capability validator stays authoritative.
        assert!(arguments_satisfy_schema(
            &json!({"anything": true}),
            &json!({"$ref": "https://example.com/not-resolvable.json"})
        ));
    }

    use crate::CapabilityWriteResult;
    use ironclaw_host_api::{
        ids::{AgentId, ProjectId, TenantId, ThreadId},
        resolution::ToolVerdict,
        result_meta::FailureKind,
    };
    use ironclaw_loop_contracts::{
        CapabilityDescriptorView, InMemoryRunProfileResolver, ResolvedRunProfile,
        RunProfileResolutionRequest, RunProfileResolver,
    };
    use ironclaw_turns::{LoopResultRef, TurnRunId, TurnScope};

    struct SpyPort {
        definitions: Vec<ProviderToolDefinition>,
        surface_version: CapabilitySurfaceVersion,
        registered_calls: Mutex<Vec<ProviderToolCall>>,
        invocations: Mutex<Vec<LoopRequest>>,
    }

    #[cfg(any(test, feature = "test-support"))]
    struct MutableDefinitionsPort {
        definitions: Mutex<Vec<ProviderToolDefinition>>,
        surface_version: Mutex<CapabilitySurfaceVersion>,
        visible_capability_ids: Option<BTreeSet<CapabilityId>>,
        tool_definition_reads: std::sync::atomic::AtomicUsize,
    }

    #[cfg(any(test, feature = "test-support"))]
    #[async_trait]
    impl LoopCapabilityPort for MutableDefinitionsPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            self.tool_definition_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(self
                .definitions
                .lock()
                .expect("mutable definitions lock")
                .clone())
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            let definitions = self
                .definitions
                .lock()
                .expect("mutable definitions lock")
                .clone();
            Ok(VisibleCapabilitySurface {
                version: self
                    .surface_version
                    .lock()
                    .expect("mutable surface-version lock")
                    .clone(),
                descriptors: definitions
                    .into_iter()
                    .filter(|definition| {
                        self.visible_capability_ids
                            .as_ref()
                            .is_none_or(|visible| visible.contains(&definition.capability_id))
                    })
                    .map(|definition| CapabilityDescriptorView {
                        capability_id: definition.capability_id,
                        provider: None,
                        runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                        safe_name: definition.name.to_string(),
                        safe_description: definition.description,
                        description_trust: definition.description_trust,
                        parameters_schema: definition.parameters,
                    })
                    .collect(),
                callable_capability_ids: None,
            })
        }

        async fn invoke_capability(
            &self,
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            unreachable!("turn-state rebuild test does not dispatch")
        }

        async fn invoke_capability_batch(
            &self,
            _request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            unreachable!("turn-state rebuild test does not dispatch")
        }
    }

    #[async_trait]
    impl LoopCapabilityPort for SpyPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self.definitions.clone())
        }

        fn provider_tool_call_capability_ids(
            &self,
            tool_call: &ProviderToolCall,
        ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
            let definition = self
                .definitions
                .iter()
                .find(|definition| definition.name == tool_call.name)
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "provider tool call is outside the visible capability surface",
                    )
                })?;
            Ok(ProviderToolCallCapabilityIds::single(
                definition.capability_id.clone(),
            ))
        }

        fn validate_provider_tool_call(
            &self,
            tool_call: &ProviderToolCall,
        ) -> Result<(), AgentLoopHostError> {
            // Sentinel: lets a test drive the describe-first path by failing
            // pre-dispatch validation for a resolved target (mirrors the
            // `register_explodes` register-failure sentinel above).
            if tool_call
                .arguments
                .get("__force_invalid")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Err(invalid_invocation(
                    "spy validation rejects forced-invalid input",
                ));
            }
            self.provider_tool_call_capability_ids(tool_call)
                .map(|_| ())
        }

        async fn register_provider_tool_call(
            &self,
            request: RegisterProviderToolCallRequest,
        ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
            let RegisterProviderToolCallRequest {
                tool_call,
                activity_id,
            } = request;
            // Sentinel: lets tests drive the gateway's "register failed" arm.
            if tool_call.name.as_str() == "register_explodes" {
                return Err(invalid_invocation("spy register explodes"));
            }
            self.validate_provider_tool_call(&tool_call)?;
            self.registered_calls
                .lock()
                .expect("registered calls lock")
                .push(tool_call.clone());
            let definition = self
                .definitions
                .iter()
                .find(|definition| definition.name == tool_call.name)
                .expect("test target definition")
                .clone();
            Ok(CapabilityCallCandidate {
                activity_id: activity_id.unwrap_or_else(CapabilityActivityId::new),
                surface_version: self.surface_version.clone(),
                capability_id: definition.capability_id,
                input_ref: input_ref(format!("input:{}", tool_call.name)),
                effective_capability_ids: Vec::new(),
                provider_replay: Some(provider_replay_for(&tool_call, tool_call.name.clone())),
            })
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                callable_capability_ids: None,
                version: self.surface_version.clone(),
                descriptors: self
                    .definitions
                    .iter()
                    .map(|definition| CapabilityDescriptorView {
                        capability_id: definition.capability_id.clone(),
                        provider: None,
                        runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                        safe_name: definition.name.to_string(),
                        safe_description: definition.description.clone(),
                        description_trust: definition.description_trust,
                        parameters_schema: definition.parameters.clone(),
                    })
                    .collect(),
            })
        }

        async fn invoke_capability(
            &self,
            request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            // Sentinel: lets a test drive a gate (approval) suspension outcome.
            let suspends = request.capability_id.as_str() == "fixture.suspends";
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            if suspends {
                Ok(resolution::approval_required(
                    ironclaw_turns::LoopGateRef::new("gate:test").expect("valid gate ref"),
                    "approval needed".to_string(),
                    None,
                )
                .resolution)
            } else {
                Ok(resolution::completed(
                    LoopResultRef::new("result:target").expect("valid result ref"),
                    "target completed".to_string(),
                    CapabilityProgress::MadeProgress,
                    false,
                    2,
                    None,
                    None,
                ))
            }
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            let mut resolutions = Vec::new();
            for invocation in request.invocations {
                resolutions.push(self.invoke_capability(invocation).await?);
            }
            Ok(ResolutionBatch {
                resolutions,
                stopped_on_suspension: false,
            })
        }
    }

    #[tokio::test]
    async fn same_turn_schema_refresh_rebuilds_index_and_preserves_progress() {
        let inner = Arc::new(MutableDefinitionsPort {
            definitions: Mutex::new(vec![provider_definition(
                "fixture.lookup",
                "fixture__lookup",
                "Lookup records",
            )]),
            surface_version: Mutex::new(
                CapabilitySurfaceVersion::new("surface:initial")
                    .expect("valid initial surface version"),
            ),
            visible_capability_ids: None,
            tool_definition_reads: std::sync::atomic::AtomicUsize::new(0),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("initial surface refresh");
        let original_fingerprint = {
            let mut guard = port.lock_turn_state().expect("initial turn state");
            let state = guard.as_mut().expect("initial state exists");
            state.disclosed_names.insert("fixture__lookup".to_string());
            state.search_ranks.insert("fixture__lookup".to_string(), 2);
            state.definitions_fingerprint
        };
        let reads_after_initial_refresh = inner
            .tool_definition_reads
            .load(std::sync::atomic::Ordering::Relaxed);
        drop(port.turn_state().expect("cached turn state"));
        drop(port.turn_state().expect("cached turn state again"));
        assert_eq!(
            inner
                .tool_definition_reads
                .load(std::sync::atomic::Ordering::Relaxed),
            reads_after_initial_refresh,
            "unchanged same-turn bridge calls must not refetch or rehash definitions"
        );
        inner.definitions.lock().expect("mutable definitions lock")[0].parameters = json!({
            "type": "object",
            "properties": {"timezone": {"type": "string"}},
            "required": ["timezone"],
            "additionalProperties": false
        });
        *inner
            .surface_version
            .lock()
            .expect("mutable surface-version lock") =
            CapabilitySurfaceVersion::new("surface:refreshed")
                .expect("valid refreshed surface version");

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("same-turn surface refresh");
        let guard = port.lock_turn_state().expect("refreshed turn state");
        let state = guard.as_ref().expect("refreshed state exists");
        assert_ne!(state.definitions_fingerprint, original_fingerprint);
        assert_eq!(
            state.search_index.search("timezone", 1).names,
            vec!["fixture__lookup"]
        );
        assert!(state.disclosed_names.contains("fixture__lookup"));
        assert_eq!(state.search_ranks["fixture__lookup"], 2);
    }

    #[tokio::test]
    async fn complete_policy_qualified_surface_limits_disclosure_catalog_and_search_index() {
        let visible_ids: BTreeSet<_> = (0..6)
            .map(|index| {
                CapabilityId::new(format!("fixture.visible_{index}"))
                    .expect("valid visible capability id")
            })
            .collect();
        let mut definitions: Vec<_> = visible_ids
            .iter()
            .enumerate()
            .map(|(index, capability_id)| {
                provider_definition(
                    capability_id.as_str(),
                    &format!("visible_tool_{index}"),
                    "Visible operation",
                )
            })
            .collect();
        definitions.push(provider_definition(
            "fixture.policy_excluded",
            "policy_excluded_tool",
            "Forbidden runtime effect approval vocabulary",
        ));
        let inner = Arc::new(MutableDefinitionsPort {
            definitions: Mutex::new(definitions),
            surface_version: Mutex::new(
                CapabilitySurfaceVersion::new("surface:complete-policy")
                    .expect("valid surface version"),
            ),
            visible_capability_ids: Some(visible_ids),
            tool_definition_reads: std::sync::atomic::AtomicUsize::new(0),
        });
        let port = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("complete policy-qualified surface builds turn state");

        let guard = port.lock_turn_state().expect("turn state lock");
        let state = guard.as_ref().expect("turn state exists");
        assert!(
            state
                .catalog
                .definition_by_capability_id(
                    &CapabilityId::new("fixture.policy_excluded")
                        .expect("valid excluded capability id")
                )
                .is_none(),
            "a capability excluded by non-ID policy dimensions must not enter the disclosure catalog"
        );
        assert!(
            state
                .search_index
                .search("forbidden runtime effect approval vocabulary", 5)
                .names
                .is_empty(),
            "excluded capability metadata must not affect or appear in deferred-tool search"
        );
    }

    struct TestWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for TestWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            let result_digest = ironclaw_host_api::approval::sha256_digest_token(
                write.input_ref.as_str().as_bytes(),
            )
            .replace(':', ".");
            Ok(CapabilityWriteResult::without_output_digest(
                LoopResultRef::new(format!("result:{result_digest}")).expect("valid result ref"),
                write.output.to_string().len() as u64,
            ))
        }
    }

    struct BarrierWriter {
        barrier: Arc<tokio::sync::Barrier>,
    }

    #[derive(Default)]
    struct BatchOnlyPort {
        batches: Mutex<Vec<LoopRequestBatch>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for BatchOnlyPort {
        fn requires_ordered_batch_invocation(&self, _invocations: &[LoopRequest]) -> bool {
            true
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface:ordered-disclosure")
                    .expect("surface version"),
                descriptors: Vec::new(),
                callable_capability_ids: None,
            })
        }

        async fn invoke_capability(
            &self,
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "ordered test port must be entered through its batch contract",
            ))
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            let resolutions = request
                .invocations
                .iter()
                .map(|invocation| {
                    resolution::completed(
                        LoopResultRef::new(format!(
                            "result:{}",
                            invocation.capability_id.as_str().replace('.', "-")
                        ))
                        .expect("valid result ref"),
                        "ordered target completed".to_string(),
                        CapabilityProgress::MadeProgress,
                        false,
                        0,
                        None,
                        None,
                    )
                })
                .collect();
            self.batches.lock().expect("batches lock").push(request);
            Ok(ResolutionBatch {
                resolutions,
                stopped_on_suspension: false,
            })
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for BarrierWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            self.barrier.wait().await;
            TestWriter.write_capability_result(write).await
        }
    }

    #[derive(Default)]
    struct CapturingWriter {
        outputs: Mutex<Vec<Value>>,
    }

    #[tokio::test]
    async fn parallel_discovery_batch_overlaps_bridge_invocations() {
        let definitions = (0..6)
            .map(|index| {
                provider_definition(
                    &format!("fixture.lookup_{index}"),
                    &format!("fixture__lookup_{index}"),
                    "Lookup records",
                )
            })
            .collect();
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:parallel-discovery")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port_with_writer(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(BarrierWriter {
                barrier: Arc::new(tokio::sync::Barrier::new(2)),
            }),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface initializes the disclosure catalog");

        let search = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "lookup", "limit": 2}),
            )))
            .await
            .expect("search registers");
        let describe = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_DESCRIBE_NAME,
                json!({"name": "fixture__lookup_5"}),
            )))
            .await
            .expect("describe registers");
        let requests = [search, describe]
            .into_iter()
            .map(|candidate| LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .collect();

        let batch = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            port.invoke_capability_batch(LoopRequestBatch {
                invocations: requests,
                stop_on_first_suspension: false,
            }),
        )
        .await
        .expect("parallel discovery calls must reach the writer concurrently")
        .expect("parallel discovery batch succeeds");

        assert_eq!(batch.resolutions.len(), 2);
        assert!(!batch.stopped_on_suspension);
    }

    #[tokio::test]
    async fn ordered_non_bridge_batch_reaches_inner_batch_contract() {
        let inner = Arc::new(BatchOnlyPort::default());
        let port = disclosure_port(
            inner.clone() as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        let surface_version =
            CapabilitySurfaceVersion::new("surface:ordered-disclosure").expect("surface version");
        let request = |capability_id: &str| LoopRequest {
            activity_id: Default::default(),
            surface_version: surface_version.clone(),
            capability_id: CapabilityId::new(capability_id).expect("capability id"),
            input_ref: input_ref(format!("input:{capability_id}")),
            approval_resume: None,
            auth_resume: None,
        };

        let batch = port
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![request("fixture.first"), request("fixture.second")],
                stop_on_first_suspension: true,
            })
            .await
            .expect("ordered batch reaches inner batch contract");

        assert_eq!(batch.resolutions.len(), 2);
        let batches = inner.batches.lock().expect("batches lock");
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0]
                .invocations
                .iter()
                .map(|invocation| invocation.capability_id.as_str())
                .collect::<Vec<_>>(),
            vec!["fixture.first", "fixture.second"]
        );
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for CapturingWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            self.outputs
                .lock()
                .expect("captured outputs lock")
                .push(write.output.clone());
            TestWriter.write_capability_result(write).await
        }
    }

    #[tokio::test]
    async fn visible_surface_preserves_verified_catalog_description_provenance() {
        let mut definition = provider_definition(
            "fixture.read_file",
            "read_file",
            "Verified catalog description",
        );
        definition.description_trust =
            ironclaw_host_api::capability::CapabilityDescriptionTrust::VerifiedCatalog;
        let inner = Arc::new(SpyPort {
            definitions: vec![definition],
            surface_version: CapabilitySurfaceVersion::new("surface:verified-catalog")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let descriptor = surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.safe_name == "read_file")
            .expect("core capability remains visible");

        assert_eq!(
            descriptor.description_trust,
            ironclaw_host_api::capability::CapabilityDescriptionTrust::VerifiedCatalog
        );
    }

    #[tokio::test]
    async fn search_discloses_tool_call_dispatches_target_and_promotes_next_turn() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let writer = Arc::new(CapturingWriter::default());
        let port = disclosure_port_with_writer(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
            Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == "hidden_tool"),
            "deferred tool should not be model-visible before discovery"
        );
        let advertised = port.tool_definitions().expect("tool definitions");
        for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
            assert!(
                advertised
                    .iter()
                    .any(|definition| definition.name.as_str() == bridge),
                "deferred surfaces advertise the complete discovery protocol: missing {bridge}"
            );
        }
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool")
        );

        // Forgiving `tool_call` resolution of an undisclosed catalog tool is
        // covered by `tool_call_resolves_undisclosed_catalog_target_forgivingly`.
        // This test focuses on the search -> disclose -> dispatch -> promote flow.

        let search = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            )))
            .await
            .expect("search registers");
        let search_outcome = port
            .invoke_capability(LoopRequest {
                activity_id: search.activity_id,
                surface_version: search.surface_version,
                capability_id: search.capability_id,
                input_ref: search.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("search invokes");
        assert!(matches!(search_outcome, Resolution::Done(ref o) if o.verdict.is_success()));
        {
            let outputs = writer.outputs.lock().expect("captured outputs lock");
            assert_eq!(
                outputs.len(),
                1,
                "complete search signature must not require a describe result"
            );
            let hidden_result = outputs[0]["results"]
                .as_array()
                .expect("search results")
                .iter()
                .find(|result| result["name"] == "hidden_tool")
                .expect("hidden result");
            assert_eq!(hidden_result["schema_complete"], true);
            assert_eq!(hidden_result["parameters"]["required"], json!(["path"]));
        }

        let disclosed_surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface after search");
        assert!(
            disclosed_surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == "hidden_tool"),
            "same-turn search should disclose target to the executor surface"
        );

        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": r#"{"path":"demo"}"#}),
            )))
            .await
            .expect("disclosed tool_call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            TOOL_CALL_NAME
        );
        let batch = port
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![LoopRequest {
                    activity_id: target.activity_id,
                    surface_version: target.surface_version,
                    capability_id: target.capability_id,
                    input_ref: target.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                }],
                stop_on_first_suspension: true,
            })
            .await
            .expect("target batch invokes");
        assert!(matches!(
            batch.resolutions.as_slice(),
            [Resolution::Done(o)] if o.verdict.is_success()
        ));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .map(|call| (call.name.as_str(), &call.arguments)),
            Some(("hidden_tool", &json!({"path": "demo"}))),
            "the provider-safe string must decode before inner registration"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "successful deferred tool_call should promote the target on the next turn"
        );
    }

    #[tokio::test]
    async fn incomplete_search_signature_falls_back_to_describe_before_dispatch() {
        let mut hidden = provider_definition(
            "fixture.hidden",
            "hidden_tool",
            "Hidden workspace operation",
        );
        hidden.parameters = json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "x".repeat(9 * 1024)}},
            "required": ["path"]
        });
        let mut definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            hidden,
        ];
        definitions.extend((1..=4).map(|index| {
            provider_definition(
                &format!("fixture.extra_{index}"),
                &format!("extra_tool_{index}"),
                "Extra operation",
            )
        }));
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let writer = Arc::new(CapturingWriter::default());
        let port = disclosure_port_with_writer(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
            Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let search = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 1}),
            )))
            .await
            .expect("search registers");
        port.invoke_capability(LoopRequest {
            activity_id: search.activity_id,
            surface_version: search.surface_version,
            capability_id: search.capability_id,
            input_ref: search.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("search invokes");

        {
            let outputs = writer.outputs.lock().expect("captured outputs lock");
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0]["results"][0]["schema_complete"], false);
            assert!(outputs[0]["results"][0].get("parameters").is_none());
        }

        let describe_first = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": "{}"}),
            )))
            .await
            .expect("invalid undisclosed call registers describe-first");
        port.invoke_capability(LoopRequest {
            activity_id: describe_first.activity_id,
            surface_version: describe_first.surface_version,
            capability_id: describe_first.capability_id,
            input_ref: describe_first.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("describe-first invokes");
        {
            let outputs = writer.outputs.lock().expect("captured outputs lock");
            assert_eq!(outputs.len(), 2, "incomplete signature auto-loads once");
            assert_eq!(outputs[1]["status"], "schema_loaded");
            assert_eq!(outputs[1]["name"], "hidden_tool");
            assert!(outputs[1].get("parameters").is_some());
        }
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "invalid call must not dispatch before its schema is visible"
        );
    }

    #[tokio::test]
    async fn direct_deferred_catalog_tool_dispatches_target_and_promotes_next_turn() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred"
        );

        let direct_call = provider_call("hidden_tool", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("direct deferred call resolves through inner");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "fixture.hidden"
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("direct deferred call validates through inner");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "hidden_tool"
        );
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "hidden_tool"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "successful direct deferred call should promote the target on the next turn"
        );
    }

    #[tokio::test]
    async fn capability_info_on_deferred_tool_promotes_it_for_direct_use_next_turn() {
        // Firat's discovery flow: tool_search (names) -> capability_info (loads +
        // promotes) -> direct call. Inspecting a deferred tool via capability_info
        // must disclose it this turn and promote it for the next, so it becomes
        // directly callable — without this the model inspects a tool it can never
        // reach and loops.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !port
                .tool_definitions()
                .expect("tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred"
        );

        // The model inspects the deferred tool by its canonical capability id.
        let inspect = provider_call("capability_info", json!({"name": "fixture.hidden"}));
        port.note_capability_info_target(&inspect)
            .expect("capability_info promotes the inspected target");

        // This turn the inspected tool is disclosed onto the callable surface
        // (visible_capabilities descriptors), so a call to it is authorized.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface after inspect");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id.as_str() == "fixture.hidden"),
            "capability_info discloses the inspected tool onto the surface this turn"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        assert!(
            next_turn
                .tool_definitions()
                .expect("next tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "capability_info promotes the inspected tool for the next turn"
        );
    }

    #[tokio::test]
    async fn undisclosed_invalid_deferred_call_returns_schema_instead_of_dispatching() {
        // The failure tool disclosure introduced: the model calls a deferred tool
        // whose schema it has not loaded, with arguments that fail validation (a
        // required field — e.g. an id — it does not have). Pre-disclosure the
        // schema was always in context; now it is deferred, so the model calls
        // blind and loops on the opaque schema error. Describe-first returns the
        // schema as a recoverable completion WITHOUT dispatching the target blind,
        // so the model's retry can be well-formed.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("describe-first registers");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "an undisclosed invalid call must route to a schema (bridge) response, not the target"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "describe-first must NOT register/dispatch the target on the inner port"
        );

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("describe-first invokes");
        assert!(
            matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
            "describe-first returns the schema as a recoverable completion"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "describe-first must NOT invoke the target on the inner port"
        );
    }

    #[tokio::test]
    async fn well_formed_blind_deferred_call_dispatches_without_describe_first() {
        // Describe-first must not tax correct calls: a blind call whose arguments
        // pass validation dispatches straight to the target (no wasted round-trip),
        // matching the zero-round-trip pre-disclosure behavior.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("valid blind call registers");
        assert_eq!(
            candidate.capability_id.as_str(),
            "fixture.hidden",
            "a well-formed blind call dispatches the target directly, not describe-first"
        );
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target registered")
                .name
                .as_str(),
            "hidden_tool"
        );
    }

    #[tokio::test]
    async fn describe_first_is_one_shot_so_repeated_failures_still_reach_dispatch() {
        // Backstop-safety: describe-first fires at most once per undisclosed tool.
        // After the schema is disclosed, a still-invalid call must dispatch (and
        // fail) through the normal path rather than returning a schema again —
        // otherwise a wedged model would receive an endless stream of
        // "made progress" schema responses and the no-progress detector would
        // never observe the repeated failure.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        // First invalid blind call -> describe-first (schema bridge), discloses.
        let first = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("first registers");
        assert!(
            is_bridge_capability_id(&first.capability_id),
            "first undisclosed invalid call is describe-first"
        );
        port.invoke_capability(LoopRequest {
            activity_id: first.activity_id,
            surface_version: first.surface_version,
            capability_id: first.capability_id,
            input_ref: first.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("first invokes (discloses schema)");

        // Second still-invalid call -> now disclosed, so it no longer intercepts:
        // it dispatches, the inner port rejects it, and a recoverable Failed
        // outcome (countable by the no-progress detector) surfaces — NOT a schema.
        let second = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("second registers via recoverable fallback");
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: second.activity_id,
                surface_version: second.surface_version,
                capability_id: second.capability_id,
                input_ref: second.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("second invokes");
        assert!(
            matches!(outcome, Resolution::Done(ref o) if matches!(o.verdict, ToolVerdict::RecoverableFailure { .. })),
            "after disclosure a still-invalid call surfaces a Failed outcome the no-progress detector can count, not another schema"
        );
    }

    #[tokio::test]
    async fn direct_deferred_encoded_wire_name_records_canonical_wire_name_in_replay() {
        // Regression: a weak model calls a deferred provider tool by its canonical
        // `__`-encoded wire name (e.g. `google-calendar__list_events`, which
        // `tool_search`/`tool_describe` surface) before it is advertised. The
        // forgiving direct-deferred path resolves that, and the recorded provider
        // replay (consumed by the assistant transcript and any provider-error
        // result ref) MUST carry the canonical wire name so it serializes without
        // tripping `validate_provider_tool_name`. (The dotted capability_id form a
        // model might otherwise copy can no longer reach this port: `ProviderToolName`
        // excludes dots, so the gateway rejects such a call before it lands here.)
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "google-calendar.list_events",
                "google-calendar__list_events",
                "List Google Calendar events",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "google-calendar__list_events"),
            "deferred Google Calendar tool starts hidden"
        );

        // The model calls the deferred tool by its `__`-encoded wire name before
        // it is advertised.
        let deferred_call = provider_call("google-calendar__list_events", json!({"path": "demo"}));
        port.provider_tool_call_capability_ids(&deferred_call)
            .expect("deferred wire name resolves through forgiving path");
        port.validate_provider_tool_call(&deferred_call)
            .expect("deferred wire name validates through forgiving path");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(deferred_call))
            .await
            .expect("deferred wire name registers as target");

        let replay = candidate.provider_replay.as_ref().expect("provider replay");
        assert_eq!(
            replay.provider_tool_name.as_str(),
            "google-calendar__list_events",
            "replay records the canonical wire name"
        );
        // The recorded name must serialize into the transcript without error.
        ironclaw_safety::validate_provider_tool_name(replay.provider_tool_name.as_str())
            .expect("recorded provider tool name is wire-safe");
    }

    #[tokio::test]
    async fn gate_suspended_target_is_promoted_so_it_survives_the_resume() {
        // Regression: a tool the model dispatched that paused on an approval/auth
        // gate must stay model-visible across the resume, exactly like a completed
        // dispatch. Otherwise the per-turn disclosed set resets on resume, the tool
        // drops off the surface, and the model's retry is hard-rejected by the
        // visible-surface filter ("outside the model-visible capability view") —
        // discarding the response and borking the run. Only *invoked* tools promote
        // (completed or gate-suspended), so a mere search/describe still does not,
        // and the advertised surface does not balloon toward "all tools".
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.suspends", "suspends_tool", "Needs approval"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !port
                .tool_definitions()
                .expect("tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "suspends_tool"),
            "suspends_tool starts deferred"
        );

        // Direct-deferred call -> resolves -> dispatch -> APPROVAL suspension.
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                "suspends_tool",
                json!({"path": "demo"}),
            )))
            .await
            .expect("direct deferred call registers as target");
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(
            outcome.parks(),
            "the gate must park the call (a re-entrant Blocked gate), not complete it"
        );

        // The resume is a fresh decorator instance (new turn state) sharing the
        // promoted store, exactly like the live BlockedApproval resume. The
        // gate-blocked tool must still be advertised.
        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        assert!(
            next_turn
                .tool_definitions()
                .expect("next tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "suspends_tool"),
            "a gate-suspended tool must be promoted so it survives the resume"
        );
    }

    #[tokio::test]
    async fn callable_set_includes_advertised_bridges_so_the_visible_filter_keeps_them() {
        // Regression: callable_capability_ids was derived only from the inner
        // catalog, which excludes the synthesized bridges. The outer model-visible
        // filter is seeded from callable and strips any advertised tool not in it —
        // so the bridges (tool_search / tool_describe / tool_call) vanished from the
        // model's tool list and it could no longer discover anything ("tool_search
        // is not available"). Callable must be a superset of everything advertised
        // this turn AND still include the deferred long tail.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            advertised
                .iter()
                .any(|d| d.name.as_str() == TOOL_SEARCH_NAME),
            "fixture must be in deferred mode so the bridges are advertised"
        );
        let callable: std::collections::HashSet<_> = surface
            .callable_capability_ids
            .as_ref()
            .expect("disclosure narrows the surface, so callable set is populated")
            .iter()
            .cloned()
            .collect();
        // Every advertised tool — bridges included — must be authorizable, or the
        // visible-surface filter strips it from the model's tool list.
        for descriptor in &surface.descriptors {
            assert!(
                callable.contains(&descriptor.capability_id),
                "advertised tool {} missing from callable; the visible filter would strip it",
                descriptor.capability_id.as_str()
            );
        }
        // The deferred long tail stays callable (the original purpose of callable).
        assert!(
            callable.iter().any(|id| id.as_str() == "fixture.hidden"),
            "deferred catalog tool must remain callable"
        );
    }

    #[tokio::test]
    async fn direct_provider_encoded_builtin_dispatches_and_promotes() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("builtin.echo", "echo", "Echo the input"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "echo"),
            "echo starts deferred"
        );

        let direct_call = provider_call("builtin__echo", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("provider-encoded direct deferred call resolves");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "builtin.echo"
        );
        assert_eq!(
            capability_ids
                .effective_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["builtin.echo"]
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("provider-encoded direct deferred call validates against resolved target");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("provider-encoded direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "builtin.echo");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "builtin__echo"
        );
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "echo",
            "inner registration must receive the catalog target name"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "builtin.echo"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "echo"),
            "successful provider-encoded direct deferred call should promote the target next turn"
        );
    }

    #[tokio::test]
    async fn direct_provider_encoded_non_builtin_extension_tool_dispatches_and_promotes() {
        // Generality guard: the forgiving direct-deferred path must resolve ANY
        // deferred tool by its provider-encoded wire name, not just `builtin__*`.
        // Production sets `ProviderToolDefinition.name` to the encoded wire name
        // (`capability.provider_tool_name`, see capability_port surface_snapshot)
        // for every provider, and the catalog matches it by exact name. This
        // fixture mirrors that for a NON-builtin extension tool
        // (`gmail.send_message` -> wire `gmail__send_message`), so the resolution
        // cannot lean on the builtin-specific `strip_prefix("builtin__")` leniency
        // — if it did, this tool would fail "unresolved unadvertised" exactly like
        // the long tail of extension/MCP tools would in production.
        let definitions = vec![
            provider_definition("builtin.read_file", "builtin__read_file", "Read a file"),
            provider_definition(
                "gmail.send_message",
                "gmail__send_message",
                "Send an email via Gmail",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "gmail__send_message"),
            "gmail__send_message starts deferred"
        );

        let direct_call = provider_call("gmail__send_message", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("provider-encoded non-builtin direct deferred call resolves");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "gmail.send_message"
        );
        assert_eq!(
            capability_ids
                .effective_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["gmail.send_message"]
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("provider-encoded non-builtin direct deferred call validates against target");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("provider-encoded non-builtin direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "gmail.send_message");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "gmail__send_message"
        );
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "gmail__send_message",
            "inner registration must receive the catalog target name"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "gmail.send_message"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "gmail__send_message"),
            "successful non-builtin direct deferred call should promote the target next turn"
        );
    }

    #[tokio::test]
    async fn tool_call_resolves_undisclosed_catalog_target_forgivingly() {
        // Regression: a model (often a strong one) may invoke a catalog tool via
        // the `tool_call` bridge WITHOUT first discovering it through
        // tool_search/tool_describe. The bridge used to reject that with a generic
        // `invalid_input` ("unknown or not disclosed") carrying no recovery hint,
        // so the model looped on the same dead-end call until the run died. The
        // bridge must be no stricter than a direct call: an undisclosed catalog
        // tool resolves and dispatches to the target.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred (never discovered this turn)"
        );

        // tool_call the deferred tool WITHOUT any prior tool_search/tool_describe.
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("undisclosed tool_call resolves forgivingly");
        assert_eq!(
            candidate.capability_id.as_str(),
            "fixture.hidden",
            "undisclosed tool_call must resolve to the catalog target, not the bridge"
        );

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target dispatches");
        assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "hidden_tool",
            "the inner port must receive the unwrapped target call"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );
    }

    #[tokio::test]
    async fn tool_call_target_registration_failure_falls_back_to_recoverable_bridge_failure() {
        // Regression: the forgiving tool_call path resolves a deferred target, but
        // if the inner port then rejects it (e.g. malformed arguments), that must
        // surface as a RECOVERABLE invalid_input the model can retry — NOT a hard
        // error, which the gateway turns into a run-borking discard of the whole
        // provider response. (Observed live with gpt-5.5: repeated tool_call
        // validation rejections, run ending Failed / driver_protocol_violation.)
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.explodes", "register_explodes", "Register fails"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let bridge_call = provider_call(
            TOOL_CALL_NAME,
            json!({"name": "register_explodes", "arguments": {"path": "demo"}}),
        );
        // Validation must NOT hard-fail — that would abort the whole response.
        port.validate_provider_tool_call(&bridge_call)
            .expect("bridge validate downgrades a target failure to recoverable");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(bridge_call))
            .await
            .expect("bridge register falls back instead of erroring");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "a target that cannot register must fall back to the bridge path"
        );

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles the fallback");
        // T5 (#5712/#7892): a target that fails registration falls back to the
        // same "not a known tool" bridge branch as a genuinely unknown target,
        // now carrying FailureKind::UnknownCapability so the model's structured
        // hint agrees with the free text (both say "not a known tool").
        assert!(
            matches!(
                outcome,
                Resolution::Done(ref o)
                    if matches!(
                        o.verdict,
                        ToolVerdict::RecoverableFailure {
                            error_kind: FailureKind::UnknownCapability,
                            ..
                        }
                    )
            ),
            "fallback must be a recoverable UnknownCapability failure, not run death"
        );
    }

    #[tokio::test]
    async fn tool_call_targeting_a_bridge_is_rejected_without_dispatch() {
        // Recursion guard: tool_call(name = a bridge) must NOT re-enter the
        // bridge or dispatch anything — it is a model-recoverable failure.
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        // Build the surface first, as the real loop always does before a call.
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": TOOL_SEARCH_NAME, "arguments": {}}),
            )))
            .await
            .expect("recursive tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "recursive tool_call must stay on the bridge path, never resolve to a target"
        );
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles recursion");
        // T5 (#5712/#7892): recursing into a bridge name hits the same "not a
        // known tool" branch as any other unresolvable tool_call target, now
        // FailureKind::UnknownCapability.
        assert!(
            matches!(
                outcome,
                Resolution::Done(ref o)
                    if matches!(
                        o.verdict,
                        ToolVerdict::RecoverableFailure {
                            error_kind: FailureKind::UnknownCapability,
                            ..
                        }
                    )
            ),
            "recursive tool_call must be a recoverable UnknownCapability failure, not run death"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "recursion must not register any target call on the inner port"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "recursion must not dispatch to the inner port"
        );
    }

    #[tokio::test]
    async fn advertised_tool_describe_errors_are_recoverable_without_dispatch() {
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        for (arguments, expected, expected_kind) in [
            (
                json!({}),
                "tool_describe requires name",
                FailureKind::InputEncode,
            ),
            (
                json!({"name": 42}),
                "tool_describe requires name",
                FailureKind::InputEncode,
            ),
            (
                json!({"name": TOOL_SEARCH_NAME}),
                "tool_describe target must not be a bridge",
                FailureKind::InputEncode,
            ),
            (
                json!({"name": "does_not_exist"}),
                "tool_describe target is unknown; use tool_search to find the correct tool name",
                FailureKind::UnknownCapability,
            ),
        ] {
            let candidate =
                port.register_provider_tool_call(RegisterProviderToolCallRequest::new(
                    provider_call(TOOL_DESCRIBE_NAME, arguments),
                ))
                .await
                .expect("tool_describe registers on the bridge path");
            let outcome = port
                .invoke_capability(LoopRequest {
                    activity_id: candidate.activity_id,
                    surface_version: candidate.surface_version,
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("tool_describe returns a recoverable result");
            assert!(
                matches!(
                    outcome,
                    Resolution::Done(ref output)
                        if matches!(
                            &output.verdict,
                            ToolVerdict::RecoverableFailure { diagnostic, .. }
                                if diagnostic.model_visible_text() == Some(expected)
                        )
                ),
                "unexpected tool_describe outcome for {expected}: {outcome:?}"
            );
            assert!(
                matches!(
                    outcome,
                    Resolution::Done(ref output)
                        if matches!(
                            &output.verdict,
                            ToolVerdict::RecoverableFailure { error_kind, .. }
                                if *error_kind == expected_kind
                        )
                ),
                "unexpected FailureKind for {expected}: {outcome:?}"
            );
        }

        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "invalid tool_describe calls must not register an inner capability"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "invalid tool_describe calls must not dispatch an inner capability"
        );
    }

    #[tokio::test]
    async fn tool_call_targeting_unknown_tool_is_rejected_without_dispatch() {
        // Unknown-target guard: tool_call(name = not in catalog) must be a
        // model-recoverable failure and must not dispatch to the inner port.
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        // Build the surface first, as the real loop always does before a call.
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "does_not_exist", "arguments": {}}),
            )))
            .await
            .expect("unknown-target tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "unknown-target tool_call must stay on the bridge path"
        );
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles unknown target");
        // T5 (#5712/#7892): an unresolvable tool_call target carries
        // FailureKind::UnknownCapability so the structured hint agrees with
        // the free text ("not a known tool"), instead of steering the model
        // to fix arguments on a target that was never a real tool.
        assert!(
            matches!(
                outcome,
                Resolution::Done(ref o)
                    if matches!(
                        o.verdict,
                        ToolVerdict::RecoverableFailure {
                            error_kind: FailureKind::UnknownCapability,
                            ..
                        }
                    )
            ),
            "unknown-target tool_call must be a recoverable UnknownCapability failure"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "unknown target must not register any call on the inner port"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "unknown target must not dispatch to the inner port"
        );
    }

    #[tokio::test]
    async fn promotions_are_scoped_by_full_turn_scope_not_thread_only() {
        let inner = Arc::new(SpyPort {
            definitions: vec![
                provider_definition("fixture.read_file", "read_file", "Read a file"),
                provider_definition(
                    "fixture.hidden",
                    "hidden_tool",
                    "Hidden workspace operation",
                ),
                provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
                provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
                provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
                provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
            ],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let tenant_a_first_turn = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context_for(
                "tenant-a",
                "agent-tool-disclosure",
                "project-tool-disclosure",
                "shared-thread",
                TurnId::new(),
            )
            .await,
            Arc::clone(&promoted_by_scope),
        );
        tenant_a_first_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");
        let search = tenant_a_first_turn
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            )))
            .await
            .expect("search registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(LoopRequest {
                    activity_id: search.activity_id,
                    surface_version: search.surface_version,
                    capability_id: search.capability_id,
                    input_ref: search.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("search invokes"),
            Resolution::Done(o) if o.verdict.is_success()
        ));
        let target = tenant_a_first_turn
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("target registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(LoopRequest {
                    activity_id: target.activity_id,
                    surface_version: target.surface_version,
                    capability_id: target.capability_id,
                    input_ref: target.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("target invokes"),
            Resolution::Done(o) if o.verdict.is_success()
        ));

        let tenant_b_next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context_for(
                "tenant-b",
                "agent-tool-disclosure",
                "project-tool-disclosure",
                "shared-thread",
                TurnId::new(),
            )
            .await,
            promoted_by_scope,
        );
        tenant_b_next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("tenant B surface builds");
        let tenant_b_advertised = tenant_b_next_turn
            .tool_definitions()
            .expect("tenant B tool definitions");
        assert!(
            !tenant_b_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "promotion from tenant A must not leak to tenant B with the same thread id"
        );
    }

    #[tokio::test]
    async fn tool_search_rejects_missing_non_string_or_blank_query() {
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        for arguments in [
            json!({}),
            json!({"query": 42}),
            json!({"query": ""}),
            json!({"query": "   "}),
            json!({"query": "x".repeat(MAX_SEARCH_QUERY_BYTES.saturating_add(1))}),
        ] {
            let candidate =
                port.register_provider_tool_call(RegisterProviderToolCallRequest::new(
                    provider_call(TOOL_SEARCH_NAME, arguments),
                ))
                .await
                .expect("tool_search registers");
            let outcome = port
                .invoke_capability(LoopRequest {
                    activity_id: candidate.activity_id,
                    surface_version: candidate.surface_version,
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("tool_search invokes");
            assert!(matches!(
                outcome,
                Resolution::Done(ref o)
                    if matches!(
                        o.verdict,
                        ToolVerdict::RecoverableFailure {
                            error_kind: FailureKind::InputEncode,
                            ..
                        }
                    )
            ));
        }
    }

    #[test]
    fn bounded_search_output_fits_or_degrades_to_compact() {
        let small_result = search_result_with_schema("alpha", json!({"type": "object"}));
        let small_output = bounded_search_output("alpha", vec![small_result.clone()]);
        assert_eq!(
            small_output["results"][0]["schema_complete"],
            Value::Bool(true)
        );
        assert_eq!(
            small_output["results"][0]["parameters"],
            small_result.parameters
        );
        assert_eq!(
            small_output["guidance"],
            TOOL_SEARCH_INVOKE_DIRECTLY_GUIDANCE
        );

        let large_result = search_result_with_schema("alpha", json!({"value": "x".repeat(4096)}));
        let large_output = bounded_search_output("alpha", vec![large_result]);
        assert_eq!(
            large_output["results"][0]["schema_complete"],
            Value::Bool(false)
        );
        assert!(large_output["results"][0].get("parameters").is_none());
        assert_eq!(
            large_output["guidance"],
            TOOL_SEARCH_DESCRIBE_FOR_SCHEMA_GUIDANCE
        );
    }

    /// Test-only now: production's fit check moved from a fixed raw-byte comparison
    /// (`serialized_len_within` against a byte ceiling) to the exact escaped-size check
    /// `wrapped_reply_fits` performs (see #7984). Kept here as a focused unit test of the
    /// bounded-writer byte-counting mechanics themselves.
    struct BoundedCountingWriter {
        bytes_written: usize,
        limit: usize,
    }

    impl std::io::Write for BoundedCountingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let Some(next) = self.bytes_written.checked_add(buffer.len()) else {
                return Err(std::io::Error::other(
                    "serialized schema exceeds byte limit",
                ));
            };
            if next > self.limit {
                return Err(std::io::Error::other(
                    "serialized schema exceeds byte limit",
                ));
            }
            self.bytes_written = next;
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn serialized_len_within(value: &Value, limit: usize) -> Option<usize> {
        let mut writer = BoundedCountingWriter {
            bytes_written: 0,
            limit,
        };
        serde_json::to_writer(&mut writer, value)
            .ok()
            .map(|()| writer.bytes_written)
    }

    #[test]
    fn bounded_schema_measurement_stops_at_the_limit() {
        let schema = json!({"value": "x".repeat(1_000_000)});

        assert_eq!(
            serialized_len_within(&json!({"type": "object"}), 64),
            Some(17)
        );
        assert_eq!(serialized_len_within(&schema, 8 * 1024), None);
    }

    #[test]
    fn compact_control_arm_omits_signature_contract_fields() {
        let results = compact_search_results(vec![search_result_with_schema(
            "alpha",
            json!({"type": "object"}),
        )]);

        assert!(results[0].get("parameters").is_none());
        assert!(results[0].get("schema_complete").is_none());
        assert_eq!(results[0]["name"], "alpha");
    }

    #[test]
    fn bounded_search_output_only_rank_one_can_be_schema_complete() {
        let first = search_result_with_schema("first", json!({"type": "object"}));
        let second = search_result_with_schema("second", json!({"type": "string"}));
        let output = bounded_search_output("query", vec![first, second]);

        assert_eq!(output["results"][0]["name"], "first");
        assert_eq!(output["results"][0]["schema_complete"], Value::Bool(true));
        assert_eq!(output["results"][1]["name"], "second");
        assert_eq!(output["results"][1]["schema_complete"], Value::Bool(false));
    }

    #[test]
    fn bounded_search_output_is_deterministic_for_empty_and_multiple_results() {
        let empty_output = bounded_search_output("query", Vec::new());
        assert_eq!(empty_output["results"], json!([]));
        assert_eq!(empty_output["guidance"], TOOL_SEARCH_NO_MATCH_GUIDANCE);
        assert!(
            serde_json::to_vec(&empty_output).expect("serializes").len()
                <= MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES
        );

        let first = search_result_with_schema("first", json!({"type": "object"}));
        let second = search_result_with_schema("second", json!({"type": "string"}));
        let inputs = vec![first, second];
        assert_eq!(
            serde_json::to_vec(&bounded_search_output("query", inputs.clone()))
                .expect("first result serializes"),
            serde_json::to_vec(&bounded_search_output("query", inputs))
                .expect("second result serializes")
        );
    }

    /// Proves the `.take(TOOL_SEARCH_INLINE_RESULT_LIMIT)` cap in `bounded_search_output` actually
    /// drops a 4th+ ranked result rather than merely happening to line up with every existing
    /// test's fixture count: every other `bounded_search_output_*` test above passes at most
    /// `TOOL_SEARCH_INLINE_RESULT_LIMIT` results, so none of them can fail if the cap were removed
    /// or widened. This one passes 5 small (well under budget) results and asserts both that the
    /// array is truncated to exactly the limit AND that survivors are ranks 1-3 in order, not an
    /// arbitrary subset -- and that no panic occurs when more results are supplied than the limit.
    #[test]
    fn bounded_search_output_drops_results_beyond_the_inline_limit() {
        let results: Vec<_> = (1..=5)
            .map(|rank| {
                search_result_with_schema(&format!("rank{rank}"), json!({"type": "object"}))
            })
            .collect();

        let output = bounded_search_output("query", results);

        let names: Vec<&str> = output["results"]
            .as_array()
            .expect("results is an array")
            .iter()
            .map(|entry| entry["name"].as_str().expect("name is a string"))
            .collect();

        assert_eq!(
            names.len(),
            TOOL_SEARCH_INLINE_RESULT_LIMIT,
            "a 4th+ ranked result must be dropped, not carried through"
        );
        assert_eq!(
            names,
            vec!["rank1", "rank2", "rank3"],
            "survivors must be exactly the first three ranks, in rank order"
        );
    }

    /// A `parameters` schema whose compact-serialized size is controlled precisely: a fixed shape
    /// plus one `description` field padded to add exactly `pad_len` unescaped ASCII bytes.
    fn schema_of_pad_len(pad_len: usize) -> Value {
        json!({
            "type": "object",
            "properties": {"value": {"type": "string"}, "padding": {"type": "string"}},
            "description": "x".repeat(pad_len),
            "required": ["value"],
        })
    }

    #[test]
    fn bounded_search_output_promotes_rank_one_exactly_at_the_shared_budget_boundary() {
        let rank_two = search_result_with_schema("rank_two", json!({"type": "object"}));
        let rank_three = search_result_with_schema("rank_three", json!({"type": "object"}));

        // Whether rank 1's own complete-schema candidate (rung a) fits at a given pad_len.
        // Padding is unescaped ASCII, so it grows both the raw AND the JSON-string-escaped size
        // byte-for-byte -- `fits_at` is therefore monotonic decreasing in pad_len, so binary
        // search finds the EXACT byte the escaped-size check flips at, deriving the boundary
        // from the real (fixed) check rather than assuming raw bytes == embedded bytes (the
        // assumption #7984's escaping defect broke).
        let fits_at = |pad_len: usize| -> bool {
            let rank_one = search_result_with_schema("rank_one", schema_of_pad_len(pad_len));
            let output = bounded_search_output(
                "query",
                vec![rank_one, rank_two.clone(), rank_three.clone()],
            );
            output["results"][0]["schema_complete"] == Value::Bool(true)
        };

        assert!(
            fits_at(0),
            "pad_len=0 baseline must itself fit, or the derived boundary below is meaningless"
        );
        let mut lower = 0usize; // known to fit
        let mut upper = 8192usize; // must not fit -- checked next
        assert!(
            !fits_at(upper),
            "search upper bound must itself not fit; widen it if this assertion fires"
        );
        while upper - lower > 1 {
            let mid = lower + (upper - lower) / 2;
            if fits_at(mid) {
                lower = mid;
            } else {
                upper = mid;
            }
        }
        // `lower` = the largest pad_len that still fits ("just under"); `upper` = `lower + 1`,
        // the smallest pad_len that does not ("just over").

        let just_under = search_result_with_schema("rank_one", schema_of_pad_len(lower));
        let under_output = bounded_search_output(
            "query",
            vec![just_under.clone(), rank_two.clone(), rank_three.clone()],
        );
        assert_eq!(
            under_output["results"][0]["schema_complete"],
            Value::Bool(true)
        );
        assert_eq!(
            under_output["results"][0]["parameters"], just_under.parameters,
            "complete rank 1 is untruncated (D5)"
        );
        assert_eq!(
            under_output["guidance"],
            TOOL_SEARCH_INVOKE_DIRECTLY_GUIDANCE
        );
        assert!(
            serde_json::to_vec(&under_output).expect("serializes").len()
                <= MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES
        );

        let just_over = search_result_with_schema("rank_one", schema_of_pad_len(upper));
        let over_output = bounded_search_output("query", vec![just_over, rank_two, rank_three]);
        assert_eq!(
            over_output["results"][0]["schema_complete"],
            Value::Bool(false)
        );
        assert!(
            over_output["results"][0].get("parameters").is_none(),
            "compact rank 1 carries no partial schema (D5)"
        );
        assert_eq!(
            over_output["guidance"],
            TOOL_SEARCH_DESCRIBE_FOR_SCHEMA_GUIDANCE
        );
        // The compact-fallback path gets its own byte assertion — every return path is measured.
        assert!(
            serde_json::to_vec(&over_output).expect("serializes").len()
                <= MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES,
            "compact-fallback reply must itself stay under the first-look ceiling"
        );
    }

    /// One compact entry at the practical worst-case field sizes derived in the Goal section's
    /// "Compact-path worst case" table: name at ProviderToolName::MAX_BYTES (64 B, enforced by
    /// construction), capability_id at the largest real capability id observed across every
    /// extension manifest (39 B -- NOT enforced by construction), description at
    /// COMPACT_DESCRIPTION_MAX_BYTES's true 163 B worst case (enforced by truncate_preview),
    /// required at the largest real required-params array observed across every committed
    /// extension schema (6 params, google-sheets/format_cells.input.v1.json).
    fn worst_case_compact_result() -> crate::tool_disclosure::CatalogSearchResult {
        crate::tool_disclosure::CatalogSearchResult {
            name: "n".repeat(64),
            capability_id: CapabilityId::new(format!("fixture.{}", "c".repeat(31)))
                .expect("valid capability id"), // "fixture." (8) + 31 B -> 39 B total, matching the observed real max
            // truncate_preview (util.rs:34-38) is a strict `<=`: `if s.len() <= max_bytes { return
            // s.to_string(); }`. At exactly 160 B (== COMPACT_DESCRIPTION_MAX_BYTES) that condition
            // is TRUE, so the string is returned UNCHANGED -- no "..." appended, field stays 160 B,
            // not 163 B. Truncation only actually fires, and only then does the unreserved 3-byte
            // "..." land, once the input exceeds 160 B. 200 is comfortably past that boundary so the
            // intent (exercise the true 163 B worst case) is unmistakable, not an off-by-one away
            // from silently drifting back onto it.
            description: "d".repeat(200),
            required_params: vec!["r".repeat(10); 6], // 6 entries, matches the observed real worst case
            parameters: json!({"type": "object"}),
        }
    }

    /// Proves the compact fallback path (rank 1's OWN schema forced not to fit, so every rank rides
    /// compact at once) stays under the shared first-look ceiling at the practical worst-case field
    /// sizes computed in the Goal section, not just the small fixtures the other tests use.
    #[test]
    fn bounded_search_output_compact_worst_case_fits_the_first_look_ceiling() {
        let query = "q".repeat(MAX_SEARCH_QUERY_BYTES); // 1_024 B, the real cap (tool_search.rs:12)
        let results: Vec<_> = (0..TOOL_SEARCH_INLINE_RESULT_LIMIT)
            .map(|n| {
                let mut r = worst_case_compact_result();
                if n == 0 {
                    // Force rank 1's OWN schema to miss the budget so every rank, including rank 1,
                    // rides compact -- the actual fallback branch this test exists to pin.
                    r.parameters = schema_of_pad_len(4096);
                }
                r
            })
            .collect();
        let output = bounded_search_output(&query, results);
        assert_eq!(
            output["results"][0]["schema_complete"],
            Value::Bool(false),
            "test setup must exercise the compact-fallback branch, not the schema-complete one"
        );
        // Rank 1 keeps its routing tail: the fixture's 200 B description is under
        // RANK_ONE_DESCRIPTION_MAX_BYTES, so it rides untruncated. This is the half of the rule
        // that matters for tool selection -- this catalog puts "use X instead" at the END of a
        // description, and truncating rank 1 to 160 B amputates exactly that.
        assert_eq!(
            output["results"][0]["description"]
                .as_str()
                .expect("description is a string")
                .len(),
            200,
            "rank 1 must keep its FULL description (under RANK_ONE_DESCRIPTION_MAX_BYTES) -- this \
             catalog puts routing guidance at the END of a description, so truncating rank 1 \
             amputates the sentence that answers which tool to use"
        );
        // Ranks 2-3 stay on the tighter cap and DO cross truncate_preview's <= boundary
        // (util.rs:34-38): 200 B in, 160 B cap, +3 B ellipsis = 163 B out. Affording rank 1's
        // larger cap for all three ranks overruns the ceiling and costs rank 1 its schema -- the
        // asymmetry is deliberate; if this drifts, re-check the budget arithmetic, not this line.
        assert_eq!(
            output["results"][1]["description"]
                .as_str()
                .expect("description is a string")
                .len(),
            163,
            "ranks 2-3 stay capped at COMPACT_DESCRIPTION_MAX_BYTES + 3"
        );
        let bytes = serde_json::to_vec(&output).expect("reply serializes").len();
        assert!(
            bytes <= MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES,
            "worst-case compact-fallback reply was {bytes} B, over the {MODEL_FIRST_LOOK_PREVIEW_MAX_BYTES} B \
             first-look ceiling -- see the Goal section's compact-path worst-case arithmetic (computed \
             at 2,482 B for this exact construction; if this fails, that arithmetic and the corpus \
             maxima it's based on need re-deriving, not this assertion loosening)"
        );
    }

    fn search_result_with_schema(
        name: &str,
        parameters: Value,
    ) -> crate::tool_disclosure::CatalogSearchResult {
        crate::tool_disclosure::CatalogSearchResult {
            name: name.to_string(),
            capability_id: CapabilityId::new(format!("fixture.{name}"))
                .expect("valid capability id"),
            description: format!("{name} description"),
            required_params: vec!["value".to_string()],
            parameters,
        }
    }

    /// #7984 defect 2: escaping cost is content-dependent, so a fixed raw-byte budget cannot
    /// model it. Build a result whose `required` names and `description` are saturated with `"`
    /// and `\` -- each occurrence costs an EXTRA byte once JSON-string-escaped into
    /// `detail.preview`, on top of the escaping the raw candidate's own JSON serialization
    /// already performs on them (a `"`/`\` inside a JSON string value is escaped once building
    /// the raw reply, then escaped AGAIN embedding that raw reply as a string) -- so a handful of
    /// these characters costs far more than their raw byte count suggests.
    fn escape_heavy_required_name(seed: usize) -> String {
        format!(
            "\"param_{seed}\"_with\\backslash_and_\"quotes\"_{}",
            "\\\"".repeat(6)
        )
    }

    #[test]
    fn bounded_search_output_degrades_for_escape_heavy_content() {
        let mut rank_one = search_result_with_schema("rank_one", json!({"type": "object"}));
        rank_one.description = format!(
            "\"escape\\heavy\" description with lots of \\\"quoted\\\" segments: {}",
            "\\\"".repeat(40)
        );
        rank_one.required_params = (0..10).map(escape_heavy_required_name).collect();
        let mut rank_two = search_result_with_schema("rank_two", json!({"type": "object"}));
        rank_two.description = rank_one.description.clone();
        rank_two.required_params = rank_one.required_params.clone();
        let mut rank_three = search_result_with_schema("rank_three", json!({"type": "object"}));
        rank_three.description = rank_one.description.clone();
        rank_three.required_params = rank_one.required_params.clone();

        let output = bounded_search_output("query", vec![rank_one, rank_two, rank_three]);

        // A real, non-empty reply -- not a panic, and not the empty floor rung.
        let results = output["results"]
            .as_array()
            .expect("results is an array")
            .clone();
        assert!(
            !results.is_empty(),
            "escape-heavy content must still return real matches, not the empty floor"
        );
        // The SAME escaped-size check production enforces -- proves the reply fits by
        // construction, not merely that this test forgot to check.
        assert!(
            wrapped_reply_fits(&output),
            "escape-heavy reply must fit the escaped-size budget: {}",
            serde_json::to_string(&output).expect("serializes")
        );
    }

    #[test]
    fn bounded_search_output_degrades_for_wide_required_array() {
        // #7984 defect 1: an untrusted/dynamic (e.g. hosted-MCP) tool schema can declare an
        // arbitrarily large `required` array. 20 names x 40 chars alone produces a 6,332 B RAW
        // compact reply -- over the 3,072 B first-look ceiling outright, regardless of escaping,
        // so the compact fallback (which previously had NO runtime check at all) must itself
        // degrade.
        let wide_required: Vec<String> = (0..20)
            .map(|index: usize| format!("required_param_name_{index:02}_{}", "p".repeat(17)))
            .collect();
        assert_eq!(
            wide_required[0].len(),
            40,
            "fixture must actually be 40 B per name, matching the Goal section's worst case"
        );
        let mut rank_one = search_result_with_schema("rank_one", json!({"type": "object"}));
        rank_one.required_params = wide_required.clone();
        let mut rank_two = search_result_with_schema("rank_two", json!({"type": "object"}));
        rank_two.required_params = wide_required.clone();
        let mut rank_three = search_result_with_schema("rank_three", json!({"type": "object"}));
        rank_three.required_params = wide_required;

        let output = bounded_search_output("query", vec![rank_one, rank_two, rank_three]);

        let results = output["results"]
            .as_array()
            .expect("results is an array")
            .clone();
        assert!(
            !results.is_empty(),
            "a wide required array must still return real matches, not the empty floor"
        );
        assert!(
            wrapped_reply_fits(&output),
            "wide-required-array reply must fit the escaped-size budget: {}",
            serde_json::to_string(&output).expect("serializes")
        );
        // The degradation actually fired: the 20-name array did not ride through unbounded.
        let required_len = results[0]["required"]
            .as_array()
            .map(|array| array.len())
            .unwrap_or(0);
        assert!(
            required_len < 20,
            "required array must have been capped or omitted, not passed through unbounded \
             (got {required_len} entries)"
        );
    }

    fn disclosure_port(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    ) -> ToolDisclosureCapabilityPort {
        disclosure_port_with_writer(inner, run_context, promoted_by_scope, Arc::new(TestWriter))
    }

    fn disclosure_port_with_writer(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> ToolDisclosureCapabilityPort {
        ToolDisclosureCapabilityPort {
            inner,
            run_context,
            result_writer,
            promoted_by_scope,
            caps: DisclosureCaps {
                max_tokens: u32::MAX,
                max_tools: 5,
                ctx_limit: None,
            },
            mode: crate::ToolDisclosureMode::Bridged,
            // Unnarrowed — unit tests here exercise disclosure mechanics, not
            // profile narrowing (that's the integration tier).
            policy: Arc::new(CapabilitySurfacePolicy::allow_all()),
            profile_pins: Vec::new(),
            turn_state: Mutex::new(None),
            bridge_inputs: Mutex::new(BTreeMap::new()),
            tool_call_target_inputs: Mutex::new(BTreeMap::new()),
        }
    }

    async fn run_context(turn_id: TurnId) -> LoopRunContext {
        run_context_for(
            "tenant-tool-disclosure",
            "agent-tool-disclosure",
            "project-tool-disclosure",
            "thread-tool-disclosure",
            turn_id,
        )
        .await
    }

    async fn run_context_for(
        tenant: &str,
        agent: &str,
        project: &str,
        thread: &str,
        turn_id: TurnId,
    ) -> LoopRunContext {
        let tenant_id = TenantId::new(tenant).expect("valid tenant");
        let agent_id = AgentId::new(agent).expect("valid agent");
        let project_id = ProjectId::new(project).expect("valid project");
        let thread_id = ThreadId::new(thread).expect("valid thread");
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved: ResolvedRunProfile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("run profile resolves");
        LoopRunContext::new(turn_scope, turn_id, TurnRunId::new(), resolved)
    }

    fn provider_definition(
        capability_id: &str,
        name: &str,
        description: &str,
    ) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
            name: ProviderToolName::new(name).expect("valid provider tool name"),
            description: description.to_string(),
            description_trust: Default::default(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn provider_call(name: &str, arguments: Value) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("provider-turn".to_string()),
            id: format!("call-{name}"),
            name: ProviderToolName::new(name).expect("valid provider tool name"),
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn input_ref(value: impl Into<String>) -> CapabilityInputRef {
        CapabilityInputRef::new(value.into()).expect("valid input ref")
    }
}
// arch-exempt: large_file, tool disclosure migration remains centralized, plan #6175
