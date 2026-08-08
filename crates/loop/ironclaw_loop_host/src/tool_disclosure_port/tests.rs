// arch-exempt: large_file, tool disclosure migration remains centralized, test block split from mod.rs, plan #7383
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
    CapabilityDescriptorView, ConcurrencyHint, InMemoryRunProfileResolver, ResolvedRunProfile,
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
                    concurrency_hint: ConcurrencyHint::SafeForParallel,
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
        // `register_explodes` register-failure sentinel in
        // `register_provider_tool_call` below).
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
                    concurrency_hint: ConcurrencyHint::SafeForParallel,
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

/// Shared test-double plumbing: mint the deterministic result ref and
/// byte length both writer doubles report, so a writer double never
/// forks a second copy of the digest computation.
fn test_write_result(write: CapabilityResultWrite<'_>) -> CapabilityWriteResult {
    let result_digest =
        ironclaw_host_api::approval::sha256_digest_token(write.input_ref.as_str().as_bytes())
            .replace(':', ".");
    CapabilityWriteResult::without_output_digest(
        LoopResultRef::new(format!("result:{result_digest}")).expect("valid result ref"),
        write.output.to_string().len() as u64,
    )
}

#[async_trait]
impl LoopCapabilityResultWriter for TestWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        Ok(test_write_result(write))
    }
}

/// Captures the JSON payload each bridge invocation writes, so bulk
/// `tool_describe` tests can assert on the actual model-visible result
/// (per-entry schemas and per-entry failures), not just the verdict.
#[derive(Default)]
struct RecordingWriter {
    outputs: Mutex<Vec<Value>>,
}

#[async_trait]
impl LoopCapabilityResultWriter for RecordingWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        self.outputs
            .lock()
            .expect("recorded outputs lock")
            .push(write.output.clone());
        Ok(test_write_result(write))
    }
}

async fn invoke_bridge_call(
    port: &ToolDisclosureCapabilityPort,
    bridge_name: &str,
    arguments: Value,
) -> Resolution {
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
            bridge_name,
            arguments,
        )))
        .await
        .expect("bridge call registers");
    port.invoke_capability(LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: candidate.surface_version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    })
    .await
    .expect("bridge call invokes")
}

fn bulk_describe_fixture_definitions() -> Vec<ProviderToolDefinition> {
    vec![
        provider_definition("fixture.read_file", "read_file", "Read a file"),
        provider_definition("fixture.alpha", "alpha_tool", "Alpha operation"),
        provider_definition("fixture.beta", "beta_tool", "Beta operation"),
        provider_definition("fixture.gamma", "gamma_tool", "Gamma operation"),
        provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
        provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
    ]
}

fn spy_port_with(definitions: Vec<ProviderToolDefinition>) -> Arc<SpyPort> {
    Arc::new(SpyPort {
        definitions,
        surface_version: CapabilitySurfaceVersion::new("surface:test")
            .expect("valid surface version"),
        registered_calls: Mutex::new(Vec::new()),
        invocations: Mutex::new(Vec::new()),
    })
}

fn recorded_output(writer: &RecordingWriter) -> Value {
    writer
        .outputs
        .lock()
        .expect("recorded outputs lock")
        .last()
        .cloned()
        .expect("bridge wrote a result")
}

/// A bounded `names` array collapses a whole `tool_search` candidate list
/// into one round-trip (see [`MAX_DESCRIBE_BATCH_SIZE`] for the production-trace
/// motivation): every requested schema must come back in a single result.
#[tokio::test]
async fn tool_describe_bulk_returns_every_requested_schema_in_one_result() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["alpha_tool", "beta_tool", "gamma_tool"]}),
    )
    .await;

    assert!(
        matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
        "a bulk describe of known tools succeeds: {outcome:?}"
    );
    let output = recorded_output(&writer);
    let results = output["results"]
        .as_array()
        .expect("bulk describe returns a results array");
    assert_eq!(results.len(), 3, "one entry per requested name");
    for (entry, expected) in results
        .iter()
        .zip(["alpha_tool", "beta_tool", "gamma_tool"])
    {
        assert_eq!(entry["name"], json!(expected));
        assert!(
            entry.get("parameters").is_some(),
            "each bulk entry carries the full parameter schema: {entry:?}"
        );
        assert!(
            entry.get("error").is_none(),
            "a known tool must not report a per-entry error: {entry:?}"
        );
    }

    // Every described tool is disclosed, exactly as a single-name describe
    // does — otherwise the follow-up call is rejected as not model-visible.
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible surface after bulk describe");
    for name in ["alpha_tool", "beta_tool", "gamma_tool"] {
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == name),
            "bulk describe must disclose {name} to the executor surface"
        );
    }
    assert!(
        inner
            .invocations
            .lock()
            .expect("invocations lock")
            .is_empty(),
        "describe must never dispatch the described tools"
    );
}

/// Back-compat: the pre-existing single `name` argument keeps its exact
/// flat result shape, so a model (or transcript) that learned the old shape
/// is unaffected by the bulk addition.
#[tokio::test]
async fn tool_describe_single_name_result_shape_is_unchanged() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome =
        invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, json!({"name": "alpha_tool"})).await;

    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let summary = match &outcome {
        Resolution::Done(outcome) => Some(outcome.summary.as_str()),
        _ => None,
    };
    assert_eq!(
        summary,
        Some("tool_describe returned schema"),
        "the single-name summary stays byte-exact (not the plural bulk form)"
    );
    let output = recorded_output(&writer);
    assert_eq!(output["name"], json!("alpha_tool"));
    assert_eq!(output["capability_id"], json!("fixture.alpha"));
    assert!(output.get("parameters").is_some());
    assert!(
        output.get("results").is_none(),
        "single-name describe keeps the flat shape, not the bulk envelope"
    );
}

/// The `names` array is bounded (schema `maxItems`) so one bulk describe can
/// never blow the context it is meant to save. The bound is enforced at
/// invoke time too — a provider that ignores `maxItems` must get a
/// recoverable failure, not an unbounded result.
#[tokio::test]
async fn tool_describe_bulk_rejects_more_names_than_the_bound() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let port = disclosure_port(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let too_many: Vec<String> = (0..=MAX_DESCRIBE_BATCH_SIZE)
        .map(|index| format!("alpha_tool_{index}"))
        .collect();
    // The model-visible bound message is derived from MAX_DESCRIBE_BATCH_SIZE
    // at the failure site, so this expectation must be built the same way.
    let too_many_message =
        format!("tool_describe accepts at most {MAX_DESCRIBE_BATCH_SIZE} names per call");
    for (arguments, expected) in [
        (json!({"names": too_many}), too_many_message.as_str()),
        (
            json!({"names": []}),
            TOOL_DESCRIBE_NAMES_MUST_BE_NON_EMPTY_MESSAGE,
        ),
        (
            json!({"names": [7]}),
            TOOL_DESCRIBE_NAMES_MUST_BE_STRINGS_MESSAGE,
        ),
        (
            json!({"names": "alpha_tool"}),
            TOOL_DESCRIBE_NAMES_MUST_BE_ARRAY_MESSAGE,
        ),
        (
            json!({"name": "alpha_tool", "names": "garbage"}),
            TOOL_DESCRIBE_NAMES_MUST_BE_ARRAY_MESSAGE,
        ),
        (
            json!({"name": 42, "names": ["alpha_tool"]}),
            TOOL_DESCRIBE_EITHER_NAME_OR_NAMES_MESSAGE,
        ),
        (json!({}), TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE),
        (
            json!({ "names": null }),
            TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE,
        ),
        // The single-name shape never echoes the requested spelling, so it
        // stays uncapped: an over-long single name falls through to the
        // catalog lookup exactly as it did before the bulk shape existed.
        (
            json!({"name": "a".repeat(MAX_DESCRIBE_NAME_BYTES + 1)}),
            TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE,
        ),
    ] {
        let outcome = invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, arguments).await;
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
            "unexpected bulk describe outcome for {expected}: {outcome:?}"
        );
    }
    assert!(
        inner
            .invocations
            .lock()
            .expect("invocations lock")
            .is_empty(),
        "bounded-input rejections must not dispatch anything"
    );
}

/// Opacity (#7166 section 1 / #5712): bulk describe must not become an
/// enumeration oracle. A policy-excluded tool that genuinely exists and a
/// name that does not exist at all must produce byte-identical per-entry
/// outcomes, and neither may fail the whole call — the valid names in the
/// same request still come back with their schemas.
#[tokio::test]
async fn tool_describe_bulk_reports_denied_and_unknown_names_identically() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let policy = CapabilitySurfacePolicy::allow_only(
        ["fixture.read_file", "fixture.alpha"]
            .into_iter()
            .map(|id| CapabilityId::new(id).expect("valid capability id")),
    );
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(policy),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["alpha_tool", "beta_tool", "no_such_tool_at_all"]}),
    )
    .await;

    assert!(
        matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
        "per-name failures stay per-entry; the call itself succeeds: {outcome:?}"
    );
    let output = recorded_output(&writer);
    let results = output["results"]
        .as_array()
        .expect("bulk describe returns a results array")
        .clone();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["name"], json!("alpha_tool"));
    assert!(
        results[0].get("parameters").is_some(),
        "the permitted tool still returns its schema"
    );

    // `beta_tool` exists but is excluded; `no_such_tool_at_all` does not
    // exist. Modulo the echoed request name, the entries must be identical —
    // no capability_id, no schema, no distinguishing message.
    let denied = results[1].clone();
    let unknown = results[2].clone();
    assert_eq!(denied["name"], json!("beta_tool"));
    assert_eq!(unknown["name"], json!("no_such_tool_at_all"));
    let strip_name = |mut entry: Value| {
        if let Some(object) = entry.as_object_mut() {
            object.remove("name");
        }
        entry
    };
    assert_eq!(
        strip_name(denied.clone()),
        strip_name(unknown.clone()),
        "an excluded tool and a nonexistent tool must be indistinguishable"
    );
    assert_eq!(
        denied["error"],
        json!(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE),
        "per-entry failure reuses the single-name opacity message"
    );
    assert!(denied.get("parameters").is_none());
    assert!(denied.get("capability_id").is_none());

    // The excluded tool must not be disclosed to the executor surface.
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible surface after bulk describe");
    assert!(
        !surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.safe_name == "beta_tool"),
        "a denied bulk-describe entry must not disclose the excluded tool"
    );
}

/// A bridge name inside a bulk request is rejected per entry, matching the
/// single-name guard, and must not silently return a bridge schema.
#[tokio::test]
async fn tool_describe_bulk_rejects_bridge_names_per_entry() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": [TOOL_SEARCH_NAME, "alpha_tool"]}),
    )
    .await;

    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results[0]["error"],
        json!(TOOL_DESCRIBE_BRIDGE_TARGET_MESSAGE)
    );
    assert!(results[0].get("parameters").is_none());
    assert!(
        results[1].get("parameters").is_some(),
        "one bad name must not cost the caller the good ones"
    );
}

/// Duplicates cost the caller nothing: the same name twice collapses to one
/// entry so a repeated name cannot multiply the result size.
#[tokio::test]
async fn tool_describe_bulk_deduplicates_repeated_names() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["alpha_tool", "alpha_tool", "beta_tool"]}),
    )
    .await;

    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        2,
        "repeated names collapse to one entry each: {results:?}"
    );

    // The length bound is applied after dedup, so the model-facing
    // collapse promise holds: nine spellings of one name are accepted and
    // collapse to a single entry instead of failing the whole call.
    let nine_copies: Vec<String> = vec!["alpha_tool".to_string(); MAX_DESCRIBE_BATCH_SIZE + 1];
    let outcome =
        invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, json!({"names": nine_copies})).await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        1,
        "nine copies of one name collapse to one entry: {results:?}"
    );
}

/// The byte bound fails its own entry, never the whole call: one over-long
/// (necessarily unresolvable) spelling in a batch must not cost the model
/// the schemas it got right, exactly like an unknown name. The echoed
/// spelling is truncated so the bound survives the per-entry path.
#[tokio::test]
async fn tool_describe_bulk_overlong_name_fails_its_entry_only() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["a".repeat(MAX_DESCRIBE_NAME_BYTES + 1), "alpha_tool"]}),
    )
    .await;

    assert!(
        matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
        "an over-long name is a per-entry failure, not a whole-call one: {outcome:?}"
    );
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0]["error"],
        json!(TOOL_DESCRIBE_NAME_TOO_LONG_MESSAGE)
    );
    let echoed = results[0]["name"].as_str().expect("echoed name");
    assert!(
        echoed.len() <= MAX_DESCRIBE_NAME_ECHO,
        "the over-long echo must be byte-bounded: {echoed}"
    );
    assert!(
        results[0].get("parameters").is_none(),
        "an over-long spelling never discloses a schema"
    );
    assert!(
        results[1].get("parameters").is_some(),
        "the valid name in the same batch still returns its schema"
    );

    // A multibyte over-long name must never be cut mid-character: the echo
    // stays valid UTF-8 and byte-bounded even when the byte boundary falls
    // inside a character.
    let multibyte: String = "é".repeat(MAX_DESCRIBE_NAME_BYTES); // 256 bytes
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": [multibyte, "alpha_tool"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results[0]["error"],
        json!(TOOL_DESCRIBE_NAME_TOO_LONG_MESSAGE)
    );
    let echoed = results[0]["name"].as_str().expect("multibyte echo");
    assert!(
        echoed.len() <= MAX_DESCRIBE_NAME_ECHO && echoed.is_char_boundary(echoed.len()),
        "the multibyte echo must be byte-bounded and char-aligned: {echoed}"
    );

    // Exactly MAX_DESCRIBE_NAME_BYTES is NOT over-long (strict `>`), so it
    // falls through to the catalog lookup and fails as a plain unknown.
    let exactly_max = "a".repeat(MAX_DESCRIBE_NAME_BYTES);
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": [exactly_max, "alpha_tool"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results[0]["error"],
        json!(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE),
        "an exactly-128-byte name takes the catalog path, not the too-long branch"
    );
    assert_eq!(results[0]["name"], json!(exactly_max));
}

/// Dedup collapses spellings that resolve to the same catalog tool, not
/// just byte-identical spellings: the dotted capability-id form and the
/// encoded wire form are two spellings of one tool, and one schema must
/// not be returned twice in a single result.
#[tokio::test]
async fn tool_describe_bulk_deduplicates_resolved_aliases() {
    let inner = spy_port_with(vec![
        provider_definition("fixture.alpha", "fixture__alpha", "Alpha operation"),
        provider_definition("fixture.beta", "fixture__beta", "Beta operation"),
    ]);
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    // Two spellings of one tool: dotted capability id + encoded wire name.
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["fixture.alpha", "fixture__alpha"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        1,
        "alias spellings of one tool collapse to one entry: {results:?}"
    );
    assert_eq!(results[0]["name"], json!("fixture__alpha"));

    // The post-dedup length bound honors the collapse promise: nine raw
    // spellings that dedup to eight distinct names are accepted, because the
    // bound is applied after trim+dedup (which only shrinks).
    let nine_spellings: Vec<String> = [
        "fixture__alpha",
        "fixture__alpha",
        "fixture__beta",
        "u_1",
        "u_2",
        "u_3",
        "u_4",
        "u_5",
        "u_6",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let outcome =
        invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, json!({"names": nine_spellings})).await;
    assert!(
        matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
        "nine spellings deduping to eight names succeed: {outcome:?}"
    );
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        8,
        "dedup brings the spelling set to the bound: {results:?}"
    );
}

/// Both describe shapes record the *resolved* catalog name in
/// `disclosed_names`: a dotted capability-id spelling must disclose the
/// encoded wire name, so a later `tool_call` in the spelling the surface
/// advertised stays model-visible.
#[tokio::test]
async fn tool_describe_bulk_discloses_resolved_name_for_dotted_spelling() {
    let inner = spy_port_with(vec![provider_definition(
        "fixture.alpha",
        "fixture__alpha",
        "Alpha operation",
    )]);
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["fixture.alpha"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results[0]["name"],
        json!("fixture__alpha"),
        "the dotted spelling resolves to the encoded catalog name"
    );

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible surface after bulk describe");
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.safe_name == "fixture__alpha"),
        "bulk describe must disclose the resolved catalog name"
    );
}

/// `name` and `names` are alternatives, exactly as the advertised schema
/// presents them: both together is a recoverable caller error (no schema
/// can express a combined bound across two properties, and no caller is
/// directed to send both), while nulls stay treated as absent so callers
/// that serialize missing optionals as null keep working.
#[tokio::test]
async fn tool_describe_rejects_name_and_names_together() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    for arguments in [
        json!({"name": "alpha_tool", "names": ["beta_tool", "gamma_tool"]}),
        json!({"name": "alpha_tool", "names": []}),
        json!({"name": 42, "names": ["alpha_tool"]}),
    ] {
        let outcome = invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, arguments).await;
        assert!(
            matches!(
                outcome,
                Resolution::Done(ref output)
                    if matches!(
                        &output.verdict,
                        ToolVerdict::RecoverableFailure { diagnostic, .. }
                            if diagnostic.model_visible_text()
                                == Some(TOOL_DESCRIBE_EITHER_NAME_OR_NAMES_MESSAGE)
                    )
            ),
            "both shapes together are a recoverable caller error: {outcome:?}"
        );
    }

    // A null `name` is treated as absent (some callers serialize missing
    // optionals as null), so the array alone drives the bulk shape.
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"name": null, "names": ["alpha_tool"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("bulk envelope");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], json!("alpha_tool"));

    // A null `names` is treated as absent, so `name` alone keeps the flat
    // single-name shape.
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"name": "alpha_tool", "names": null}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    assert_eq!(output["name"], json!("alpha_tool"));
    assert!(
        output.get("results").is_none(),
        "null names keeps the flat single-name shape"
    );

    assert!(
        inner
            .invocations
            .lock()
            .expect("invocations lock")
            .is_empty(),
        "describe must never dispatch the described tools"
    );
}

/// The advertised `maxItems` bound holds at exactly `MAX_DESCRIBE_BATCH_SIZE`:
/// eight requested names must succeed with eight result entries, so an
/// off-by-one in the invoke-time bound check cannot slip through. Unknown
/// names are fine here — per-entry failures do not fail the call.
#[tokio::test]
async fn tool_describe_bulk_accepts_exactly_max_names() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    // Five fixture tools plus three unknowns: the batch succeeds with all
    // eight entries, so the advertised bound holds at exactly
    // MAX_DESCRIBE_BATCH_SIZE while per-entry failures stay recoverable.
    let exactly_max: Vec<String> = [
        "alpha_tool",
        "beta_tool",
        "gamma_tool",
        "extra_tool_1",
        "extra_tool_2",
        "nope_1",
        "nope_2",
        "nope_3",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let outcome =
        invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, json!({"names": exactly_max})).await;

    let summary = match &outcome {
        Resolution::Done(outcome) => Some(outcome.summary.as_str()),
        _ => None,
    };
    assert!(
        matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
        "exactly MAX_DESCRIBE_BATCH_SIZE names succeeds: {outcome:?}"
    );
    let output = recorded_output(&writer);
    let results = output["results"]
        .as_array()
        .expect("bulk describe returns a results array");
    assert_eq!(
        results.len(),
        MAX_DESCRIBE_BATCH_SIZE,
        "one entry per requested name"
    );
    assert_eq!(
        results
            .iter()
            .filter(|entry| entry.get("parameters").is_some())
            .count(),
        5,
        "the five real tools carry schemas: {results:?}"
    );
    assert_eq!(
        summary,
        Some("tool_describe returned schemas"),
        "a batch with at least one schema keeps the success summary"
    );
}

/// When every entry of a bulk describe fails to resolve, the batch is
/// exactly the single-name failure case and must keep its verdict class:
/// a recoverable failure, so failure-kind tracking and recovery routing
/// see it, instead of a success verdict that claims progress.
#[tokio::test]
async fn tool_describe_bulk_all_entries_failing_is_a_recoverable_failure() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["nope_1", "nope_2", "   ", "a".repeat(MAX_DESCRIBE_NAME_BYTES + 1)]}),
    )
    .await;
    assert!(
        matches!(
            outcome,
            Resolution::Done(ref output)
                if matches!(
                    &output.verdict,
                    ToolVerdict::RecoverableFailure { diagnostic, .. }
                        if diagnostic.model_visible_text()
                            == Some(TOOL_DESCRIBE_RETURNED_NO_SCHEMAS_MESSAGE)
                )
        ),
        "an all-error batch is a recoverable failure, not a success: {outcome:?}"
    );
    assert!(
        writer.outputs.lock().expect("outputs lock").is_empty(),
        "an all-error batch must not persist a result envelope"
    );
    assert!(
        inner
            .invocations
            .lock()
            .expect("invocations lock")
            .is_empty(),
        "describe must never dispatch the described tools"
    );
}

/// An empty (or whitespace-only) array entry is schema-valid — the items
/// schema has no `minLength` — but unresolvable, so it must fail only its
/// own entry like any other unknown name: the valid names in the same
/// batch keep their schemas.
#[tokio::test]
async fn tool_describe_bulk_empty_name_fails_its_entry_only() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    for arguments in [
        json!({"names": ["", "alpha_tool"]}),
        json!({"names": ["   ", "alpha_tool"]}),
        // A null item is soft-absence inside the array too: it fails its own
        // entry as an unresolvable empty name, not the whole call.
        json!({"names": [null, "alpha_tool"]}),
    ] {
        let outcome = invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, arguments).await;
        assert!(
            matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()),
            "an empty name is a per-entry failure, not a whole-call one: {outcome:?}"
        );
        let output = recorded_output(&writer);
        let results = output["results"].as_array().expect("results array");
        assert_eq!(results.len(), 2, "both requested entries come back");
        assert_eq!(
            results[0]["error"],
            json!(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE)
        );
        assert!(
            results[0].get("parameters").is_none(),
            "an empty spelling never discloses a schema"
        );
        assert!(
            results[1].get("parameters").is_some(),
            "the valid name in the same batch still returns its schema"
        );
    }
}

/// Alias spellings that fail (denied or unknown) keep one error entry per
/// requested spelling: resolved-name dedup only collapses spellings that
/// actually resolve to a schema. The per-entry contract says nothing
/// about collapsing failures, and the opacity boundary forbids telling the
/// model the two spellings are the same excluded tool.
#[tokio::test]
async fn tool_describe_bulk_denied_alias_spellings_keep_per_spelling_entries() {
    let inner = spy_port_with(vec![
        provider_definition("fixture.alpha", "fixture__alpha", "Alpha operation"),
        provider_definition("fixture.beta", "fixture__beta", "Beta operation"),
    ]);
    let writer = Arc::new(RecordingWriter::default());
    let policy = CapabilitySurfacePolicy::allow_only(
        ["fixture.alpha"]
            .into_iter()
            .map(|id| CapabilityId::new(id).expect("valid capability id")),
    );
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(policy),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    // Partial batch: the two denied spellings keep one opaque entry each
    // while the permitted tool still returns its schema.
    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["fixture.beta", "fixture__beta", "fixture.alpha"]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(
        results.len(),
        3,
        "failing alias spellings keep one entry each in a partial batch: {results:?}"
    );
    assert_eq!(
        results[0]["error"],
        json!(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE)
    );
    assert_eq!(
        results[1]["error"],
        json!(TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE)
    );
    assert_eq!(
        results[0]["name"],
        json!("fixture.beta"),
        "each failing spelling keeps its own echo"
    );
    assert_eq!(results[1]["name"], json!("fixture__beta"));
    assert!(
        results[2].get("parameters").is_some(),
        "the permitted tool in the same batch still returns its schema"
    );
}

/// Bulk entries are trimmed before resolution, so a whitespace-padded
/// spelling of a real tool resolves like its unpadded form.
#[tokio::test]
async fn tool_describe_bulk_trims_whitespace_padded_names() {
    let inner = spy_port_with(bulk_describe_fixture_definitions());
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome = invoke_bridge_call(
        &port,
        TOOL_DESCRIBE_NAME,
        json!({"names": ["  alpha_tool  ", " beta_tool "]}),
    )
    .await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    let results = output["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["name"], json!("alpha_tool"));
    assert_eq!(results[1]["name"], json!("beta_tool"));
    assert!(
        results
            .iter()
            .all(|entry| entry.get("parameters").is_some()),
        "whitespace-padded spellings resolve like their unpadded forms"
    );
}

/// The single-name path records the *resolved* catalog name too (aligned
/// with the bulk path): a dotted capability-id spelling discloses the
/// encoded wire name the model can actually call, instead of a spelling
/// that never matches the catalog.
#[tokio::test]
async fn tool_describe_single_name_discloses_resolved_name_for_dotted_spelling() {
    let inner = spy_port_with(vec![provider_definition(
        "fixture.alpha",
        "fixture__alpha",
        "Alpha operation",
    )]);
    let writer = Arc::new(RecordingWriter::default());
    let port = disclosure_port_with(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        run_context(TurnId::new()).await,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::clone(&writer) as Arc<dyn LoopCapabilityResultWriter>,
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface builds turn state");

    let outcome =
        invoke_bridge_call(&port, TOOL_DESCRIBE_NAME, json!({"name": "fixture.alpha"})).await;
    assert!(matches!(outcome, Resolution::Done(ref o) if o.verdict.is_success()));
    let output = recorded_output(&writer);
    assert_eq!(
        output["name"],
        json!("fixture__alpha"),
        "the dotted spelling resolves to the encoded catalog name"
    );

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible surface after single-name describe");
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.safe_name == "fixture__alpha"),
        "single-name describe must disclose the resolved catalog name"
    );
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
    let port = disclosure_port(
        Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
        first_run_context,
        Arc::clone(&promoted_by_scope),
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
    assert_eq!(
        surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.safe_name == "read_file")
            .expect("read_file descriptor")
            .concurrency_hint,
        ConcurrencyHint::SafeForParallel,
        "visible surface must preserve inner descriptor metadata"
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
    assert!(
        matches!(
            outcome,
            Resolution::Done(ref o)
                if matches!(
                    o.verdict,
                    ToolVerdict::RecoverableFailure {
                        error_kind: FailureKind::InputEncode,
                        ..
                    }
                )
        ),
        "fallback must be a recoverable InvalidInput failure, not run death"
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
    assert!(
        matches!(
            outcome,
            Resolution::Done(ref o)
                if matches!(
                    o.verdict,
                    ToolVerdict::RecoverableFailure {
                        error_kind: FailureKind::InputEncode,
                        ..
                    }
                )
        ),
        "recursive tool_call must be a recoverable InvalidInput failure, not run death"
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

    for (arguments, expected) in [
        (json!({}), TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE),
        (json!({"name": 42}), TOOL_DESCRIBE_REQUIRES_NAME_MESSAGE),
        (
            json!({"name": TOOL_SEARCH_NAME}),
            TOOL_DESCRIBE_BRIDGE_TARGET_MESSAGE,
        ),
        (
            json!({"name": "does_not_exist"}),
            TOOL_DESCRIBE_UNKNOWN_TARGET_MESSAGE,
        ),
    ] {
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_DESCRIBE_NAME,
                arguments,
            )))
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
    assert!(
        matches!(
            outcome,
            Resolution::Done(ref o)
                if matches!(
                    o.verdict,
                    ToolVerdict::RecoverableFailure {
                        error_kind: FailureKind::InputEncode,
                        ..
                    }
                )
        ),
        "unknown-target tool_call must be a recoverable InvalidInput failure"
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
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                arguments,
            )))
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

fn disclosure_port(
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
) -> ToolDisclosureCapabilityPort {
    disclosure_port_with(
        inner,
        run_context,
        promoted_by_scope,
        Arc::new(TestWriter),
        // Unnarrowed — most unit tests here exercise disclosure mechanics,
        // not profile narrowing (that's the integration tier).
        Arc::new(CapabilitySurfacePolicy::allow_all()),
    )
}

fn disclosure_port_with(
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    policy: Arc<CapabilitySurfacePolicy>,
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
        policy,
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
