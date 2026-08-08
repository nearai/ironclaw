// arch-exempt: large_file, tool disclosure migration remains centralized, plan #6175;
// bulk tool_describe (PR #7374) added the describe path and its test block;
// the test block lives in sibling tests.rs; decomposition tracked in plan #7383.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{CapabilityResultWrite, DurablePersistence, LoopCapabilityResultWriter};
use async_trait::async_trait;
use ironclaw_host_api::{
    capability_surface::CapabilitySurfacePolicy,
    ids::{AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName, TenantId, ThreadId},
    resolution::{Resolution, ResolutionBatch},
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
    ActiveSet, CapabilityCatalog, DisclosureCaps, MAX_DESCRIBE_BATCH_SIZE, PromotedSet,
    TOOL_CALL_NAME, TOOL_DESCRIBE_NAME, TOOL_SEARCH_NAME, bridge_tool_definitions,
    canonicalize_json, definition_matches_provider_name, is_bridge_capability_id, is_bridge_name,
    select_active_set,
};
use crate::tool_search::{
    AuthorizedToolSearchIndex, MAX_SEARCH_QUERY_BYTES, definitions_fingerprint,
};

const DISCLOSURE_INPUT_PREFIX: &str = "input:tool-disclosure:";

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

const TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE: &str = "tool_describe requires name";
const TOOL_DESCRIBE_NAMES_MUST_BE_STRINGS_MESSAGE: &str = "tool_describe names must be strings";
const TOOL_DESCRIBE_NAMES_MUST_BE_ARRAY_MESSAGE: &str = "tool_describe names must be an array";
const TOOL_DESCRIBE_NAMES_MUST_BE_NON_EMPTY_MESSAGE: &str = "tool_describe names must not be empty";
const TOOL_DESCRIBE_EITHER_NAME_OR_NAMES_MESSAGE: &str =
    "tool_describe accepts either name or names, not both";
const TOOL_DESCRIBE_BRIDGE_TARGET_MESSAGE: &str = "tool_describe target must not be a bridge";
const TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE: &str = "tool_describe target is unknown";
const TOOL_DESCRIBE_NAME_TOO_LONG_MESSAGE: &str = "tool_describe name is too long";
const TOOL_DESCRIBE_RETURNED_NO_SCHEMAS_MESSAGE: &str = "tool_describe returned no schemas";

/// Longest *requested* bulk `tool_describe` name, in bytes.
///
/// A per-entry failure entry echoes the *requested* spelling so the model can
/// tell which name failed. Without a length bound, `MAX_DESCRIBE_BATCH_SIZE`
/// caps the entry count but not the result bytes — eight junk names of
/// arbitrary length would be reflected verbatim. Over-long bulk names fail
/// their own entry (like any other unresolvable name) with the echo truncated
/// to [`MAX_DESCRIBE_NAME_ECHO`] bytes, so the bound survives the
/// per-entry failure path. The single-name shape never echoes the requested
/// spelling (its failures carry no echo and its success echoes the resolved
/// catalog name), so it is not capped and stays byte-exact. Real provider
/// tool names are validated identifiers far shorter than this, so the cap
/// only ever rejects input that could not have resolved anyway.
const MAX_DESCRIBE_NAME_BYTES: usize = 128;

/// Echo bound for an over-long bulk entry, in bytes: long enough for the
/// model to identify which requested name failed, short enough that the echo
/// cannot itself bloat the result. Applied via `floor_char_boundary` so a
/// multibyte name is never cut mid-character.
const MAX_DESCRIBE_NAME_ECHO: usize = 64;

/// What a `tool_describe` call resolved to.
///
/// The two shapes are kept apart deliberately: `name` keeps the original flat
/// result object (back-compat for every model and transcript that learned it),
/// while `names` returns a `results` array whose entries can fail individually.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DescribeRequest {
    Single(String),
    Bulk(Vec<DescribeName>),
}

/// One requested bulk spelling, pre-classified at parse time.
///
/// The byte cap is enforced while parsing — a spelling over
/// [`MAX_DESCRIBE_NAME_BYTES`] is stored as its truncated echo only, so the
/// transient allocation can never exceed the bound — and the two variants
/// keep the over-long entry's distinct diagnostic while a within-bound name
/// resolves against the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DescribeName {
    /// A spelling within the byte bound, resolved against the catalog.
    Name(String),
    /// A spelling over [`MAX_DESCRIBE_NAME_BYTES`]: fails its own entry with
    /// a truncated echo, never resolved.
    TooLong(String),
}

/// What one `tool_describe` target resolved to.
///
/// `Resolved` carries the catalog entry whose schema is disclosed; `Unknown`
/// is the single per-name failure. Bridge names never reach this point: the
/// single-name path rejects them before the turn-state guard, the bulk loop
/// rejects them inline, and both keep the #5712 opacity boundary — a
/// policy-excluded tool and a nonexistent name collapse to the same `Unknown`
/// outcome.
enum DescribeResolution {
    Unknown,
    Resolved { name: String, entry: Value },
}

/// Parse `tool_describe` arguments into the names to resolve.
///
/// Accepts the original `name` string or the bulk `names` array — either
/// shape alone; both together is a recoverable caller error, because no
/// advertised schema can express a bound across two independent properties.
/// Bounded by [`MAX_DESCRIBE_BATCH_SIZE`] at invoke time as well as in the
/// advertised schema, because a provider that ignores `maxItems` must still
/// get a recoverable failure rather than an unbounded result. Duplicates
/// collapse so a repeated name cannot multiply the result size.
fn parse_tool_describe_names(arguments: &Value) -> Result<DescribeRequest, String> {
    // Null is treated as absent — some callers serialize missing optionals as
    // null, so a null `names` alongside a valid `name` keeps the single-name
    // shape and vice versa. Filter nulls to absence once here; every later
    // check reads the filtered values.
    let name_value = arguments.get("name").filter(|value| !value.is_null());
    let names_value = arguments.get("names").filter(|value| !value.is_null());
    // A `names` key that is present but not an array is a caller error: fail
    // recoverably with a diagnostic that names the actual problem instead of
    // silently coercing to the single-name shape or misreporting
    // "requires name".
    if let Some(names_value) = names_value
        && !names_value.is_array()
    {
        return Err(TOOL_DESCRIBE_NAMES_MUST_BE_ARRAY_MESSAGE.into());
    }
    // `name` and `names` are alternatives, exactly as the advertised schema
    // presents them (both optional, `additionalProperties: false`): sending
    // both is a caller error under either shape, not a third union shape.
    if name_value.is_some() && names_value.is_some() {
        return Err(TOOL_DESCRIBE_EITHER_NAME_OR_NAMES_MESSAGE.into());
    }
    let single_name = name_value.and_then(Value::as_str);
    let Some(values) = names_value.and_then(Value::as_array) else {
        // The pre-bulk single-name shape: the raw requested spelling is passed
        // through untrimmed and uncapped so `name` keeps its exact historical
        // behavior ("target is unknown" for any spelling that does not
        // resolve). Trim, dedup, and the byte bound apply to bulk entries,
        // whose per-entry failures echo the requested spelling.
        let Some(name) = single_name else {
            return Err(TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE.into());
        };
        return Ok(DescribeRequest::Single(name.to_string()));
    };
    // Bulk: the array alone drives the shape. The length bound is applied
    // during the trim+dedup pass, so the model-facing collapse promise holds
    // (nine spellings deduping to eight names are accepted, while a ninth
    // *distinct* name fails recoverably). Dedup borrows from the parsed
    // request and keeps at most one owned string per unique name, so the
    // allocation and result stay bounded regardless of raw array length
    // (iteration is linear in the array, which the ingress payload limit
    // already bounds). The model-visible bound message is derived from
    // [`MAX_DESCRIBE_BATCH_SIZE`] here — no hand-copied constant to drift.
    // Empty-after-trim and null entries are schema-valid or soft-absence
    // but unresolvable, so they fail their own entry like any other unknown
    // name; only non-string items are malformed enough to fail the whole
    // call.
    let mut names: Vec<DescribeName> =
        Vec::with_capacity(MAX_DESCRIBE_BATCH_SIZE.min(values.len()));
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for value in values {
        // Null is soft-absence inside the array too: it becomes the same
        // unresolvable empty entry as an empty string, not a whole-call
        // type error.
        let name = if value.is_null() {
            ""
        } else {
            match value.as_str().map(str::trim) {
                Some(name) => name,
                None => return Err(TOOL_DESCRIBE_NAMES_MUST_BE_STRINGS_MESSAGE.into()),
            }
        };
        if !seen.insert(name) {
            continue; // duplicate spelling; does not count toward the bound
        }
        if names.len() == MAX_DESCRIBE_BATCH_SIZE {
            return Err(format!(
                "tool_describe accepts at most {MAX_DESCRIBE_BATCH_SIZE} names per call"
            ));
        }
        if name.len() > MAX_DESCRIBE_NAME_BYTES {
            // Keep only the bounded echo; the full spelling never needs to
            // outlive the parse pass.
            let echo_bound = name.floor_char_boundary(MAX_DESCRIBE_NAME_ECHO);
            names.push(DescribeName::TooLong(name[..echo_bound].to_string()));
        } else {
            names.push(DescribeName::Name(name.to_string()));
        }
    }
    if names.is_empty() {
        return Err(TOOL_DESCRIBE_NAMES_MUST_BE_NON_EMPTY_MESSAGE.into());
    }
    Ok(DescribeRequest::Bulk(names))
}

fn log_describe_selection(state: &ToolDisclosureTurnState, resolved_name: &str) {
    if let Some(selected_rank) = state.search_ranks.get(resolved_name).copied() {
        debug!(
            target: "ironclaw::reborn::tool_search",
            selected_rank,
            selection_action = "describe",
            "observed deferred-tool selection without logging tool or query metadata"
        );
    }
}

pub struct ToolDisclosureCapabilityDecorator {
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    caps: DisclosureCaps,
}

impl ToolDisclosureCapabilityDecorator {
    pub fn new(result_writer: Arc<dyn LoopCapabilityResultWriter>) -> Self {
        Self {
            result_writer,
            promoted_by_scope: Arc::new(Mutex::new(HashMap::new())),
            caps: DisclosureCaps::default(),
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
        Arc::new(ToolDisclosureCapabilityPort {
            inner,
            run_context: run_context.clone(),
            result_writer: Arc::clone(&self.result_writer),
            promoted_by_scope: Arc::clone(&self.promoted_by_scope),
            caps: self.caps,
            policy,
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
    /// #5712/#5659-w6: the caller's effective policy, resolved once in
    /// `ToolDisclosureCapabilityDecorator::decorate_with_policy` — narrows disclosed
    /// tool_search/tool_describe metadata *and* the tool_search bridge's own
    /// advertised description (the always-on catalog index).
    policy: Arc<CapabilitySurfacePolicy>,
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
        let mut surface = self.inner.visible_capabilities(request).await?;
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
            let target_capability_id = self
                .tool_call_target_inputs
                .lock()
                .map_err(|e| {
                    invalid_invocation(format!("tool_call target store lock is poisoned: {e}"))
                })?
                .get(request.input_ref.as_str())
                .cloned();
            let resolution = self.inner.invoke_capability(request).await?;
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
            if (matches!(&resolution, Resolution::Done(outcome) if outcome.verdict.is_success())
                || resolution.parks())
                && let Some(capability_id) = target_capability_id
            {
                self.promote_target(&capability_id)?;
            }
            return Ok(resolution);
        }
        self.invoke_bridge(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let mut resolutions = Vec::with_capacity(request.invocations.len());
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let resolution = self.invoke_capability(invocation).await?;
            // H1: the batch stops on the first invocation that *parks* — a
            // re-entrant gate as well as a suspension.
            let parks = resolution.parks();
            resolutions.push(resolution);
            if request.stop_on_first_suspension && parks {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension,
        })
    }
}

impl ToolDisclosureCapabilityPort {
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
            let catalog = CapabilityCatalog::new(&authorized_definitions, &[]);
            let search_index = AuthorizedToolSearchIndex::new(authorized_definitions.iter());
            debug!(
                target: "ironclaw::reborn::tool_search",
                authorized_document_count = authorized_definitions.len(),
                index_build_micros = index_started_at.elapsed().as_micros(),
                metadata_fingerprint = fingerprint,
                "rebuilt authorized deferred-tool search index"
            );
            let promoted = self.promoted_for_scope()?;
            let active = select_active_set(&catalog, &promoted, self.caps, &self.policy);
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
                Ok(failed_invalid_input(
                    "tool_call arguments must be a JSON object encoded as a string",
                ))
            }
            BridgeKind::Call => Ok(failed_invalid_input(
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
            return Ok(failed_invalid_input("tool_search requires query"));
        };
        let query = query.trim();
        if query.is_empty() {
            return Ok(failed_invalid_input("tool_search requires query"));
        }
        if query.len() > MAX_SEARCH_QUERY_BYTES {
            return Ok(failed_invalid_input("tool_search query is too long"));
        }
        let limit = bridge
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(10)
            .clamp(1, 50);
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            let search_started_at = std::time::Instant::now();
            let outcome = state.search_index.search(query, limit);
            debug!(
                target: "ironclaw::reborn::tool_search",
                query_class = outcome.query_class.as_str(),
                empty_result = outcome.names.is_empty(),
                returned_count = outcome.names.len(),
                query_latency_micros = search_started_at.elapsed().as_micros(),
                "ranked deferred-tool search without logging raw query or schemas"
            );
            let mut results = Vec::new();
            for (index, name) in outcome.names.into_iter().enumerate() {
                state
                    .search_ranks
                    .insert(name.clone(), index.saturating_add(1));
                state.disclosed_names.insert(name.clone());
                if let Some(result) = state.catalog.search_result(&name) {
                    results.push(json!({
                        "name": result.name,
                        "capability_id": result.capability_id.as_str(),
                        "description": result.description,
                        "required": result.required_params,
                    }));
                }
            }
            json!({
                "query": query,
                "results": results,
            })
        };
        self.completed_bridge_result(request, output, "tool_search returned catalog matches")
            .await
    }

    async fn invoke_tool_describe(
        &self,
        request: &LoopRequest,
        bridge: &BridgeInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        let names = match parse_tool_describe_names(&bridge.arguments) {
            Ok(names) => names,
            Err(message) => return Ok(failed_invalid_input(message)),
        };
        // Bridge-name rejection stays ahead of the turn-state guard, exactly as
        // it was before the bulk shape existed: a bridge target must not be
        // misreported as "tool catalog is unavailable".
        if let DescribeRequest::Single(name) = &names
            && is_bridge_name(name)
        {
            return Ok(failed_invalid_input(TOOL_DESCRIBE_BRIDGE_TARGET_MESSAGE));
        }
        let (output, summary) = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            match self.describe_output(state, &names) {
                Ok(completed) => completed,
                Err(message) => return Ok(failed_invalid_input(message)),
            }
        };
        self.completed_bridge_result(request, output, summary).await
    }

    /// Build the `tool_describe` result for one request shape.
    ///
    /// The single-name shape maps every unresolvable target to a whole-call
    /// recoverable failure (its `Err`); the bulk shape succeeds with per-entry
    /// failures inside the result, except that a batch where every entry
    /// failed is exactly the single-name failure case and keeps its verdict
    /// class (`Err`), so failure-kind tracking and recovery routing see it.
    fn describe_output(
        &self,
        state: &mut ToolDisclosureTurnState,
        names: &DescribeRequest,
    ) -> Result<(Value, &'static str), &'static str> {
        match names {
            DescribeRequest::Single(name) => match self.describe_one(state, name) {
                None => Err(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE),
                Some((_resolved_name, entry)) => Ok((entry, "tool_describe returned schema")),
            },
            DescribeRequest::Bulk(names) => {
                let mut entries = Vec::with_capacity(names.len());
                // Dedup on the *resolved* catalog name, not the requested
                // spelling: two spellings of the same tool (bare encoded
                // name vs dotted capability id) resolve to one entry, so
                // aliases cannot multiply the result size either. Per-entry
                // failures carry no schema and are kept per spelling.
                let mut resolved_seen = BTreeSet::new();
                for name in names {
                    if let Some(entry) = self.describe_bulk_entry(state, name, &mut resolved_seen) {
                        entries.push(entry);
                    }
                }
                // A batch with at least one schema succeeds, with the
                // per-entry failures inside the result. The verdict is
                // derived from the resolved-name set that gates schema entry
                // pushes, not by re-scanning the emitted JSON for a wire key.
                if resolved_seen.is_empty() {
                    return Err(TOOL_DESCRIBE_RETURNED_NO_SCHEMAS_MESSAGE);
                }
                Ok((
                    json!({ "results": entries }),
                    "tool_describe returned schemas",
                ))
            }
        }
    }

    /// Resolve one `tool_describe` target and record it as disclosed.
    ///
    /// Shared by the single-name and bulk paths, so the disclosure
    /// bookkeeping (selection log + resolved catalog name) cannot diverge
    /// between shapes: both record the *resolved* name, so a forgiving
    /// dotted spelling discloses the canonical wire name the model can
    /// actually call. Returns `None` for the opaque unknown outcome.
    fn describe_one(
        &self,
        state: &mut ToolDisclosureTurnState,
        name: &str,
    ) -> Option<(String, Value)> {
        match self.describe_resolution(state, name) {
            DescribeResolution::Unknown => None,
            DescribeResolution::Resolved {
                name: resolved_name,
                entry,
            } => {
                log_describe_selection(state, &resolved_name);
                state.disclosed_names.insert(resolved_name.clone());
                Some((resolved_name, entry))
            }
        }
    }

    /// One entry of a bulk `tool_describe` result.
    ///
    /// Returns `None` when the name resolved to a catalog tool whose schema
    /// was already emitted in this batch (alias dedup). Every per-entry
    /// failure — bridge, over-long, unknown — renders through the same
    /// [`Self::error_entry`] shape, so the opacity-relevant failure surface
    /// is single-sourced.
    fn describe_bulk_entry(
        &self,
        state: &mut ToolDisclosureTurnState,
        name: &DescribeName,
        resolved_seen: &mut BTreeSet<String>,
    ) -> Option<Value> {
        let name = match name {
            // The byte cap was enforced at parse time; only the bounded echo
            // survives, and the distinct too-long diagnostic stays attached.
            DescribeName::TooLong(echo) => {
                return Some(Self::error_entry(echo, TOOL_DESCRIBE_NAME_TOO_LONG_MESSAGE));
            }
            DescribeName::Name(name) => name,
        };
        if is_bridge_name(name) {
            return Some(Self::error_entry(name, TOOL_DESCRIBE_BRIDGE_TARGET_MESSAGE));
        }
        match self.describe_one(state, name) {
            None => Some(Self::error_entry(
                name,
                TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE,
            )),
            Some((resolved_name, entry)) => {
                if resolved_seen.insert(resolved_name) {
                    Some(entry)
                } else {
                    None
                }
            }
        }
    }

    /// The per-entry failure shape: the requested spelling (truncated for
    /// over-long names) plus the recoverable message. The echo is the
    /// caller's own spelling, never a catalog fact, so the #5712 opacity
    /// boundary is untouched.
    fn error_entry(name: &str, message: &'static str) -> Value {
        json!({ "name": name, "error": message })
    }

    /// Resolve one `tool_describe` target to its schema entry, or classify the
    /// per-name failure.
    ///
    /// Shared by the single-name and bulk paths: they differ only in failure
    /// shape (whole-call vs per-entry) and in which spelling they record as
    /// disclosed. The permit check lives here so a policy-excluded tool and a
    /// nonexistent name stay byte-indistinguishable (#5712, #7166 §1).
    fn describe_resolution(
        &self,
        state: &ToolDisclosureTurnState,
        name: &str,
    ) -> DescribeResolution {
        let Some(result) = state.catalog.search_result(name) else {
            return DescribeResolution::Unknown;
        };
        if !self.policy.permits_capability_id(&result.capability_id) {
            return DescribeResolution::Unknown;
        }
        DescribeResolution::Resolved {
            name: result.name.clone(),
            entry: json!({
                "name": result.name,
                "capability_id": result.capability_id.as_str(),
                "description": result.description,
                "required": result.required_params,
                "parameters": result.parameters,
            }),
        }
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
            return Ok(failed_invalid_input("auto-schema requires a target name"));
        };
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            let Some(result) = state.catalog.search_result(name) else {
                return Ok(failed_invalid_input("auto-schema target is unknown"));
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

fn failed_invalid_input(summary: impl Into<String>) -> Resolution {
    let summary = summary.into();
    resolution::failed(
        ironclaw_host_api::result_meta::FailureKind::InputEncode,
        summary.clone(),
        CapabilityFailureDetail::Diagnostic { text: summary },
    )
}

fn invalid_invocation(summary: impl Into<String>) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, summary)
}

#[cfg(test)]
mod tests;
