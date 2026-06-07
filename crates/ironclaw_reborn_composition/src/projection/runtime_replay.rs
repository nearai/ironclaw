use std::collections::HashSet;

use ironclaw_event_projections::{
    CapabilityActivityProjection, CapabilityActivityStatus, ProjectionReplay, ProjectionSnapshot,
    RunStatusProjection,
};
use ironclaw_events::EventCursor;
use ironclaw_host_api::InvocationId;
use ironclaw_product_adapters::{ProductAdapterError, ProductOutboundPayload};

use super::WEBUI_RUNTIME_ITEM_MAX_PAYLOADS;

pub(crate) enum RuntimePayloadCandidate {
    State { runs: Vec<RunStatusProjection> },
    CapabilityActivity(CapabilityActivityProjection),
    CapabilityDisplayPreview(CapabilityActivityProjection),
}

pub(crate) enum RuntimePayloadResolution {
    Payload(Box<ProductOutboundPayload>),
    Pending,
    Empty,
}

pub(crate) struct RuntimePayloads {
    slots: Vec<RuntimePayloadSlot>,
}

enum RuntimePayloadSlot {
    Payload(Box<ProductOutboundPayload>),
    Pending,
}

impl RuntimePayloads {
    pub(crate) fn from_resolutions(
        resolutions: Vec<Result<RuntimePayloadResolution, ProductAdapterError>>,
    ) -> Result<Self, ProductAdapterError> {
        let mut slots = Vec::new();
        for resolution in resolutions {
            match resolution? {
                RuntimePayloadResolution::Payload(payload) => {
                    slots.push(RuntimePayloadSlot::Payload(payload));
                }
                RuntimePayloadResolution::Pending => {
                    slots.push(RuntimePayloadSlot::Pending);
                    break;
                }
                RuntimePayloadResolution::Empty => {}
            }
        }
        Ok(Self { slots })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub(crate) fn total(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn deliver_after(
        self,
        already_delivered: usize,
        capacity: usize,
    ) -> Vec<DeliveredRuntimePayload> {
        self.slots
            .into_iter()
            .enumerate()
            .skip(already_delivered)
            .filter_map(|(index, slot)| match slot {
                RuntimePayloadSlot::Payload(payload) => Some(DeliveredRuntimePayload {
                    delivered: index + 1,
                    payload: *payload,
                }),
                RuntimePayloadSlot::Pending => None,
            })
            .take(capacity)
            .collect()
    }
}

#[derive(Debug)]
pub(crate) struct DeliveredRuntimePayload {
    pub(crate) delivered: usize,
    pub(crate) payload: ProductOutboundPayload,
}

pub(crate) fn snapshot_payload_candidates(
    snapshot: ProjectionSnapshot,
) -> Vec<RuntimePayloadCandidate> {
    runtime_payload_candidates(
        snapshot.runs,
        snapshot.capability_activities,
        WEBUI_RUNTIME_ITEM_MAX_PAYLOADS,
    )
}

pub(crate) fn replay_payload_candidates(replay: &ProjectionReplay) -> Vec<RuntimePayloadCandidate> {
    let state_payloads = usize::from(!replay.runs.is_empty());
    let activity_payloads = WEBUI_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(state_payloads);
    let mut candidates = Vec::with_capacity(replay_candidate_capacity(
        state_payloads,
        activity_payloads,
        replay,
    ));

    if !replay.runs.is_empty() {
        candidates.push(RuntimePayloadCandidate::State {
            runs: replay.runs.clone(),
        });
    }

    append_activity_replay_candidates(replay, activity_payloads, &mut candidates);
    candidates
}

fn runtime_payload_candidates(
    runs: Vec<RunStatusProjection>,
    capability_activities: Vec<CapabilityActivityProjection>,
    max_payloads: usize,
) -> Vec<RuntimePayloadCandidate> {
    let state_payloads = usize::from(!runs.is_empty());
    let activity_payloads = max_payloads.saturating_sub(state_payloads);
    let mut candidates = Vec::with_capacity(
        state_payloads.saturating_add(activity_payloads.min(capability_activities.len())),
    );
    if !runs.is_empty() {
        candidates.push(RuntimePayloadCandidate::State { runs });
    }
    for activity in capability_activities.into_iter().take(activity_payloads) {
        candidates.push(RuntimePayloadCandidate::CapabilityActivity(
            activity.clone(),
        ));
        candidates.push(RuntimePayloadCandidate::CapabilityDisplayPreview(activity));
    }
    candidates
}

fn replay_candidate_capacity(
    state_payloads: usize,
    activity_payloads: usize,
    replay: &ProjectionReplay,
) -> usize {
    let transition_count = replay.capability_activity_transitions.len();
    state_payloads
        .saturating_add(activity_payloads.min(transition_count).saturating_mul(2))
        .saturating_add(replay.capability_activities.len().saturating_mul(2))
}

fn append_activity_replay_candidates(
    replay: &ProjectionReplay,
    max_activities: usize,
    candidates: &mut Vec<RuntimePayloadCandidate>,
) {
    let transitions = &replay.capability_activity_transitions;
    let transition_keys = transitions
        .iter()
        .map(activity_event_key)
        .collect::<HashSet<_>>();
    let mut emitted_activities = 0usize;

    for activity in transitions.iter().take(max_activities) {
        candidates.push(RuntimePayloadCandidate::CapabilityActivity(
            activity.clone(),
        ));
        candidates.push(RuntimePayloadCandidate::CapabilityDisplayPreview(
            activity.clone(),
        ));
        emitted_activities = emitted_activities.saturating_add(1);
    }

    for activity in replay.capability_activities.iter() {
        if emitted_activities >= max_activities {
            break;
        }
        if transition_keys.contains(&activity_event_key(activity)) {
            continue;
        }
        candidates.push(RuntimePayloadCandidate::CapabilityActivity(
            activity.clone(),
        ));
        candidates.push(RuntimePayloadCandidate::CapabilityDisplayPreview(
            activity.clone(),
        ));
        emitted_activities = emitted_activities.saturating_add(1);
    }
}

fn activity_event_key(
    activity: &CapabilityActivityProjection,
) -> (InvocationId, CapabilityActivityStatus, EventCursor) {
    (
        activity.invocation_id,
        activity.status,
        activity.last_cursor,
    )
}
