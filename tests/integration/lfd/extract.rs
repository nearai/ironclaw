//! Pinned outcome EXTRACTION (SCHEMA.md §6): reads the harness recorders,
//! assertion surfaces, and PERSISTED state after a scenario — never
//! runner-local echoes, and never anything a profile hands us. Profiles
//! assemble harnesses; this module observes them.

use std::collections::BTreeMap;

use crate::case::StateQuery;
use crate::outcome::{EgressRecord, EventRecord, GateRecord, ReplyRecord, ToolInvocationRecord};
use crate::profiles::{LfdProfile, ProfileError};
use crate::reborn_support::builder::RebornIntegrationHarness;

/// `safe_summary` prefixes the executor writes for model-visible capability
/// errors — see `ToolErrorClass::summary_prefix` in
/// `tests/integration/support/assertions.rs`.
const FAILED_SUMMARY_PREFIX: &str = "capability failed with ";
const DENIED_SUMMARY_PREFIX: &str = "capability denied with ";

/// Everything observed from one executed case, pre-`seq` assignment.
pub struct Extraction {
    pub replies: Vec<ReplyRecord>,
    pub tool_invocations: Vec<ToolInvocationRecord>,
    pub egress: Vec<EgressRecord>,
    pub events: Vec<EventRecord>,
    pub gates: Vec<GateRecord>,
    /// Raw text surfaces fed to the leak scan: non-user transcript messages
    /// (assistant replies + tool-result envelopes incl. model observations),
    /// serialized turn events, recorded tool outputs, and tool params.
    pub scan_surfaces: Vec<String>,
}

/// How a state query failed, discriminating the two non-`ran` statuses.
#[derive(Debug)]
pub enum StateQueryFailure {
    /// Unknown query kind (→ `status: "unsupported"`, per the dispatcher
    /// contract in SCHEMA.md §1/§2).
    Unsupported(String),
    /// A supported kind raised while reading persisted state (→ `status: "error"`).
    Failed(String),
}

/// Extract replies, tool invocations, egress, events, and gates from the
/// harness after the scenario ran. `reply_channel` labels reply records (the
/// harness ingress is channel-less; see the runner for how it is chosen).
pub async fn extract(
    harness: &RebornIntegrationHarness,
    reply_channel: &str,
) -> Result<Extraction, String> {
    let history = match harness
        .thread_harness
        .history(harness.binding.thread_id.clone())
        .await
    {
        Ok(history) => history,
        // Protocol profiles can legitimately skip every inbound entry
        // (Slack unmentioned public messages, duplicate-only replays, etc.).
        // In that case no thread record is created and extraction should
        // produce empty reply/tool/event lanes, not an error outcome.
        Err(error) if error.to_string().contains("unknown thread") => Vec::new(),
        Err(error) => return Err(format!("thread history read failed: {error}")),
    };

    let mut scan_surfaces = Vec::new();

    // --- replies: persisted finalized assistant messages, in sequence order —
    let replies: Vec<ReplyRecord> = history
        .iter()
        .filter(|message| {
            message.kind == ironclaw_threads::MessageKind::Assistant
                && message.status == ironclaw_threads::MessageStatus::Finalized
        })
        .filter_map(|message| message.content.as_ref())
        .map(|text| ReplyRecord {
            channel: reply_channel.to_string(),
            text: text.clone(),
            seq: 0,
        })
        .collect();

    // Leak-scan surface: every non-User transcript message (assistant replies,
    // tool-result reference envelopes with safe summaries / model
    // observations, system messages). User messages are the case's own input
    // and would count injected-by-design secrets as leaks.
    for message in &history {
        if message.kind != ironclaw_threads::MessageKind::User
            && let Some(content) = &message.content
        {
            scan_surfaces.push(content.clone());
        }
    }

    // --- persisted tool-result envelopes, chronological (params + ok) -------
    struct PersistedToolResult {
        params_json: Option<String>,
        ok: bool,
    }
    let mut persisted_tool_results = Vec::new();
    for message in history
        .iter()
        .filter(|message| message.kind == ironclaw_threads::MessageKind::ToolResultReference)
    {
        // Fail loud on undecodable envelopes (matches the support tree's
        // fail-loud contract): a silent skip would degrade into a misleading
        // `ok: true` join below.
        let content = message
            .content
            .as_deref()
            .ok_or("ToolResultReference message missing content")?;
        let envelope: ironclaw_threads::ToolResultReferenceEnvelope = serde_json::from_str(content)
            .map_err(|error| format!("failed to decode ToolResultReferenceEnvelope: {error}"))?;
        let summary = envelope.safe_summary.as_str();
        let ok = !summary.starts_with(FAILED_SUMMARY_PREFIX)
            && !summary.starts_with(DENIED_SUMMARY_PREFIX);
        let params_json = match &message.tool_result_provider_call {
            Some(call) => Some(
                serde_json::to_string(&call.arguments)
                    .map_err(|error| format!("tool arguments do not serialize: {error}"))?,
            ),
            None => None,
        };
        if let Some(params) = &params_json {
            scan_surfaces.push(params.clone());
        }
        persisted_tool_results.push(PersistedToolResult { params_json, ok });
    }

    // --- tool invocations: recorder dispatch order, joined index-wise with
    // the persisted envelopes above (both chronological; 1:1 on ungated
    // flows — a gate-parked re-dispatch records two invocations for one
    // persisted result, degrading the join gracefully to `params_json: null`).
    let all_invocations = harness.capability_recorder.invocations();
    let invocations = &all_invocations[harness.baseline_invocation_count..];
    let all_results = harness.capability_recorder.capability_results();
    let recorded_results = &all_results[harness.baseline_result_count..];
    let tool_invocations: Vec<ToolInvocationRecord> = invocations
        .iter()
        .enumerate()
        .map(|(index, invocation)| {
            let persisted = persisted_tool_results.get(index);
            ToolInvocationRecord {
                name: invocation.capability_id.as_str().to_string(),
                params_json: persisted
                    .and_then(|result| result.params_json.clone())
                    .unwrap_or_else(|| "null".to_string()),
                ok: persisted.map(|result| result.ok).unwrap_or_else(|| {
                    // No persisted envelope at this position: fall back to the
                    // in-process result recorder (Completed-path writes only).
                    recorded_results.iter().any(|result| {
                        result.capability_id.as_str() == invocation.capability_id.as_str()
                    })
                }),
                seq: 0,
            }
        })
        .collect();
    for result in recorded_results {
        if let Ok(output) = serde_json::to_string(&result.output) {
            scan_surfaces.push(output);
        }
    }

    // --- egress: the recording RuntimeHttpEgress lane, baseline-sliced (R2) —
    let all_egress = harness.capability_recorder.runtime_http_requests();
    let egress: Vec<EgressRecord> = all_egress[harness.baseline_egress_count..]
        .iter()
        .map(|request| EgressRecord {
            method: request.method.to_string().to_uppercase(),
            url: request.url.clone(),
            seq: 0,
        })
        .collect();

    // --- events + gates: the in-memory TurnEventSink, baseline-sliced (R2) —
    let all_events = harness
        ._shared
        .turn_event_sink
        .as_ref()
        .map(|sink| sink.events())
        .unwrap_or_default();
    let event_delta = &all_events[harness.baseline_turn_event_count.min(all_events.len())..];
    let mut events = Vec::new();
    let mut gates = Vec::new();
    for (index, event) in event_delta.iter().enumerate() {
        events.push(EventRecord {
            kind: serde_variant_name(&event.kind).unwrap_or_else(|| format!("{:?}", event.kind)),
            seq: 0,
        });
        if let Ok(serialized) = serde_json::to_string(event) {
            scan_surfaces.push(serialized);
        }
        // Gates are derived from Blocked lifecycle events; the sink observes
        // blocked→resumed transitions but not the approve/deny decision
        // itself, so resolution granularity is `resumed` vs `blocked`.
        if event.kind == ironclaw_turns::TurnEventKind::Blocked
            && let Some(gate) = &event.blocked_gate
        {
            let resumed = event_delta[index + 1..].iter().any(|later| {
                later.run_id == event.run_id && later.kind == ironclaw_turns::TurnEventKind::Resumed
            });
            let resolution = if resumed { "resumed" } else { "blocked" };
            gates.push(GateRecord {
                kind: serde_variant_name(&gate.gate_kind)
                    .unwrap_or_else(|| format!("{:?}", gate.gate_kind)),
                resolution: resolution.to_string(),
                seq: 0,
            });
        }
    }

    for reply in &replies {
        scan_surfaces.push(reply.text.clone());
    }

    Ok(Extraction {
        replies,
        tool_invocations,
        egress,
        events,
        gates,
        scan_surfaces,
    })
}

/// Assign the Outcome `seq` values: one monotone counter walked over the lanes
/// in a FIXED order (tool_invocations → egress → gates → replies → events).
/// Order is real WITHIN a lane (each recorder is chronological); cross-lane
/// order is a stable convention, not a reconstructed interleave — the
/// harness's recorders are separate lanes with no shared clock.
///
/// CONTRACT LIMITATION (SCHEMA.md §3 `ordered`): because cross-lane `seq` is
/// concatenation order, NOT temporal truth, an `ordered` matcher sequence is
/// only meaningful when all its members are in the SAME lane (tool→tool,
/// egress→egress). A cross-lane `ordered` (e.g. `[tool, egress, reply]`) would
/// encode this function's fixed lane priority, not real execution order — eval
/// authors must not write cross-lane `ordered` constraints until the recorders
/// expose a shared clock. Flagged as a harness API gap.
pub fn assign_seq(extraction: &mut Extraction) {
    let mut next = 1u64;
    for record in &mut extraction.tool_invocations {
        record.seq = next;
        next += 1;
    }
    for record in &mut extraction.egress {
        record.seq = next;
        next += 1;
    }
    for record in &mut extraction.gates {
        record.seq = next;
        next += 1;
    }
    for record in &mut extraction.replies {
        record.seq = next;
        next += 1;
    }
    for record in &mut extraction.events {
        record.seq = next;
        next += 1;
    }
}

/// Run the case's declarative `state_queries` AFTER the scenario, against
/// persisted state. Built-in kinds live here (pinned); unknown kinds are
/// offered to the profile and otherwise flip the outcome to
/// `status: "unsupported"`. New generic kinds belong in this match.
pub async fn run_state_queries(
    harness: &RebornIntegrationHarness,
    profile: &dyn LfdProfile,
    queries: &[StateQuery],
) -> Result<BTreeMap<String, serde_json::Value>, StateQueryFailure> {
    let mut state = BTreeMap::new();
    for query in queries {
        let value = match query.kind.as_str() {
            // The persisted transcript record at `params.index` (default 0),
            // read from thread storage — `null` when the index is out of
            // range so a missing-state contract fails on the matcher, not as
            // a runner error.
            "thread_record" => {
                let index = query
                    .params
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let history = harness
                    .thread_harness
                    .history(harness.binding.thread_id.clone())
                    .await
                    .map_err(|error| {
                        StateQueryFailure::Failed(format!(
                            "state query {:?}: thread history read failed: {error}",
                            query.id
                        ))
                    })?;
                match history.get(index) {
                    Some(record) => serde_json::to_value(record).map_err(|error| {
                        StateQueryFailure::Failed(format!(
                            "state query {:?}: record does not serialize: {error}",
                            query.id
                        ))
                    })?,
                    None => serde_json::Value::Null,
                }
            }
            other => profile
                .state_query(harness, other, &query.params)
                .await
                .map_err(|error| match error {
                    ProfileError::Unsupported(reason) => StateQueryFailure::Unsupported(format!(
                        "state query {:?}: {reason}",
                        query.id
                    )),
                    ProfileError::Harness(reason) => {
                        StateQueryFailure::Failed(format!("state query {:?}: {reason}", query.id))
                    }
                })?,
        };
        state.insert(query.id.clone(), value);
    }
    Ok(state)
}

/// The serde-serialized variant name of a unit enum (e.g.
/// `TurnEventKind::Completed` → `"Completed"`, `TurnBlockedGateKind::Approval`
/// → `"approval"`), so outcome strings match each enum's wire form.
fn serde_variant_name<T: serde::Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Some(name),
        _ => None,
    }
}
