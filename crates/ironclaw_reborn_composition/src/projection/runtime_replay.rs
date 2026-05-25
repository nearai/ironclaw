use ironclaw_event_projections::{
    CapabilityActivityProjection, ProjectionReplay, ProjectionSnapshot, RunStatusProjection,
};

use super::WEBUI_RUNTIME_ITEM_MAX_PAYLOADS;

pub(crate) enum RuntimePayloadCandidate {
    State { runs: Vec<RunStatusProjection> },
    CapabilityActivity(CapabilityActivityProjection),
    CapabilityDisplayPreview(CapabilityActivityProjection),
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
        .saturating_add(activity_payloads.min(transition_count))
        .saturating_add(replay.capability_activities.len())
}

fn append_activity_replay_candidates(
    replay: &ProjectionReplay,
    max_activities: usize,
    candidates: &mut Vec<RuntimePayloadCandidate>,
) {
    let transitions = &replay.capability_activity_transitions;
    let mut emitted_activities = 0usize;

    for activity in transitions.iter().take(max_activities) {
        candidates.push(RuntimePayloadCandidate::CapabilityActivity(
            activity.clone(),
        ));
        emitted_activities = emitted_activities.saturating_add(1);
    }

    for activity in replay.capability_activities.iter() {
        if emitted_activities >= max_activities {
            break;
        }
        if transitions
            .iter()
            .any(|transition| same_activity_event(transition, activity))
        {
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

fn same_activity_event(
    left: &CapabilityActivityProjection,
    right: &CapabilityActivityProjection,
) -> bool {
    left.invocation_id == right.invocation_id
        && left.status == right.status
        && left.last_cursor == right.last_cursor
}
