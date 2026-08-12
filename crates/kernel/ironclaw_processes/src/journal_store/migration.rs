use std::collections::{HashMap, VecDeque};

use chrono::Utc;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    Timestamp,
    ids::{InvocationId, ProcessId},
    path::ScopedPath,
    resource::ResourceScope,
    turn::{TurnActor, TurnRunId, TurnScope},
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::{ProcessJournalMaterializedState, ProcessJournalStoreError};
use crate::{
    JournaledProcessSnapshot, ProcessCheckpointId, ProcessCheckpointPayload,
    ProcessCheckpointRecord, ProcessCheckpointRef, ProcessJournalCursor, ProcessKind,
    ProcessLeaseSnapshot, ProcessLifecycleStatus, ProcessSuspension, ProcessTreeReservation,
};

pub(super) fn legacy_turn_record_contains_data(
    path: &str,
    body: &[u8],
) -> Result<bool, ProcessJournalStoreError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| ProcessJournalStoreError::Deserialization(error.to_string()))?;
    if path.ends_with("/meta/state.json") {
        return Ok(value
            .get("journal_seq")
            .and_then(Value::as_u64)
            .is_some_and(|sequence| sequence > 0));
    }
    const COLLECTIONS: &[&str] = &[
        "turns",
        "runs",
        "active_locks",
        "checkpoints",
        "loop_checkpoints",
        "idempotency_records",
        "events",
        "admission_reservations",
        "spawn_tree_reservations",
    ];
    Ok(COLLECTIONS.iter().any(|key| {
        value.get(*key).is_some_and(|collection| match collection {
            Value::Array(values) => !values.is_empty(),
            Value::Object(values) => !values.is_empty(),
            _ => false,
        })
    }))
}

pub(super) async fn import_deployed_legacy_authorities<F>(
    filesystem: &ScopedFilesystem<F>,
    state: &mut ProcessJournalMaterializedState,
) -> Result<usize, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut collections = legacy_blob_collections(filesystem).await?;
    for name in [
        "turns",
        "runs",
        "loop_checkpoints",
        "idempotency_records",
        "spawn_tree_reservations",
    ] {
        let rows = legacy_row_collection(filesystem, name).await?;
        collections.entry(name).or_default().extend(rows);
    }

    let actors = legacy_turn_actors(collections.get("turns").map(Vec::as_slice).unwrap_or(&[]))?;
    let mut imported = 0usize;
    for run in collections.get("runs").map(Vec::as_slice).unwrap_or(&[]) {
        let snapshot = legacy_turn_snapshot(run, &actors)?;
        let was_present = state.processes.contains_key(&snapshot.process_id);
        state.import_deployed_snapshot(snapshot);
        imported = imported.saturating_add(usize::from(!was_present));
    }

    let checkpoint_states = legacy_checkpoint_state_records(filesystem).await?;
    for checkpoint in collections
        .get("loop_checkpoints")
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if let Some(checkpoint) = legacy_loop_checkpoint(checkpoint, &checkpoint_states)? {
            state.import_deployed_checkpoint(checkpoint);
        }
    }
    for reservation in collections
        .get("spawn_tree_reservations")
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if let Some(reservation) = legacy_tree_reservation(reservation)? {
            state.import_deployed_tree_reservation(reservation);
        }
    }

    let capability_runs = legacy_run_state_records(filesystem).await?;
    for run in capability_runs {
        let snapshot = legacy_capability_snapshot(&run)?;
        let was_present = state.processes.contains_key(&snapshot.process_id);
        state.import_deployed_snapshot(snapshot);
        imported = imported.saturating_add(usize::from(!was_present));
    }

    import_legacy_idempotency(
        state,
        collections
            .get("idempotency_records")
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    )?;
    Ok(imported)
}

async fn legacy_blob_collections<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<HashMap<&'static str, Vec<Value>>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let path = scoped("/turns/state.json")?;
    if filesystem.resolve(&ResourceScope::system(), &path).is_err() {
        return Ok(HashMap::new());
    }
    let Some(versioned) = get_optional(filesystem, &path).await? else {
        return Ok(HashMap::new());
    };
    let root: Value = decode(&versioned.entry.body)?;
    let mut collections = HashMap::new();
    for name in [
        "turns",
        "runs",
        "loop_checkpoints",
        "idempotency_records",
        "spawn_tree_reservations",
    ] {
        collections.insert(name, collection_values(root.get(name)));
    }
    Ok(collections)
}

async fn legacy_row_collection<F>(
    filesystem: &ScopedFilesystem<F>,
    collection: &'static str,
) -> Result<Vec<Value>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let directory = scoped(&format!("/turns/rows/v1/{collection}"))?;
    if filesystem
        .resolve(&ResourceScope::system(), &directory)
        .is_err()
    {
        return Ok(Vec::new());
    }
    // Legacy import is an explicit one-time migration. It must enumerate the
    // complete deployed collection: the bounded directory API silently
    // truncates at its limit and would make the initialization sentinel
    // permanent after importing only a prefix.
    let entries = match filesystem
        .list_dir(&ResourceScope::system(), &directory)
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut values = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::File {
            continue;
        }
        let path = scoped(&format!("{}/{}", directory.as_str(), entry.name))?;
        let Some(versioned) = get_optional(filesystem, &path).await? else {
            continue;
        };
        let value: Value = decode(&versioned.entry.body)?;
        match value {
            Value::Object(mut object) if object.contains_key("journal_seq") => {
                if let Some(value) = object.remove("value")
                    && !value.is_null()
                {
                    values.push(value);
                }
            }
            value => values.push(value),
        }
    }
    Ok(values)
}

fn legacy_turn_actors(
    turns: &[Value],
) -> Result<HashMap<String, TurnActor>, ProcessJournalStoreError> {
    let mut actors = HashMap::new();
    for turn in turns {
        let Some(turn_id) = turn.get("turn_id") else {
            continue;
        };
        let turn_id = turn_id
            .as_str()
            .ok_or_else(|| invalid_legacy("legacy turn_id is not a string"))?;
        let Some(actor) = turn.get("actor") else {
            continue;
        };
        let actor = serde_json::from_value(actor.clone()).map_err(deserialization)?;
        actors.insert(turn_id.to_string(), actor);
    }
    Ok(actors)
}

fn legacy_turn_snapshot(
    run: &Value,
    actors: &HashMap<String, TurnActor>,
) -> Result<JournaledProcessSnapshot, ProcessJournalStoreError> {
    let run_id: TurnRunId = required(run, "run_id")?;
    let process_id = ProcessId::from_uuid(run_id.as_uuid());
    let turn_id = required_string(run, "turn_id")?;
    let turn_scope: TurnScope = required(run, "scope")?;
    let actor = actors.get(&turn_id).cloned();
    let mut scope = turn_scope.to_resource_scope();
    scope.invocation_id = InvocationId::from_uuid(run_id.as_uuid());
    let owner_user_id = turn_scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| actor.as_ref().map(|actor| actor.user_id.clone()));
    if let Some(owner) = owner_user_id.as_ref() {
        scope.user_id = owner.clone();
    }

    let status = turn_status(required_string(run, "status")?.as_str())?;
    let suspension = turn_suspension(run, status)?;
    let checkpoint_ref =
        optional_string(run, "checkpoint_id").map(ProcessCheckpointRef::from_trusted);
    let failure = optional(run, "failure")?;
    let journal_cursor = ProcessJournalCursor(
        run.get("event_cursor")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    );
    let created_at: Timestamp = required(run, "received_at")?;
    let lease = legacy_turn_lease(run)?;
    let parent_process_id = optional_turn_process_id(run, "parent_run_id")?;
    let root_process_id = optional_turn_process_id(run, "spawn_tree_root_run_id")?;
    let metadata = legacy_turn_metadata(run, actor)?;

    Ok(JournaledProcessSnapshot {
        process_id,
        process_kind: ProcessKind::AgentTurn,
        scope,
        status,
        suspension,
        checkpoint_ref,
        // Legacy turn rows predate the recorded checkpoint kind; unknown
        // reads as side-effecting, so recovery keeps failing these closed.
        checkpoint_kind: None,
        input_ref: None,
        failure,
        journal_cursor,
        lease,
        crash_reclaim_count: 0,
        created_at,
        owner_user_id,
        concurrency_class: None,
        parent_process_id,
        root_process_id,
        metadata: json!({ "agent_turn": metadata }),
    })
}

fn legacy_turn_metadata(
    run: &Value,
    actor: Option<TurnActor>,
) -> Result<Value, ProcessJournalStoreError> {
    let profile = run
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_legacy("turn run is missing profile"))?;
    let mut metadata = Map::new();
    copy_required(run, &mut metadata, "turn_id")?;
    if let Some(actor) = actor {
        metadata.insert(
            "actor".to_string(),
            serde_json::to_value(actor).map_err(serialization)?,
        );
    }
    for name in [
        "accepted_message_ref",
        "source_binding_ref",
        "reply_target_binding_ref",
        "resolved_model_route",
        "model_usage",
        "subagent_depth",
        "product_context",
        "auth_resume_disposition",
    ] {
        if let Some(value) = run.get(name) {
            metadata.insert(name.to_string(), value.clone());
        }
    }
    metadata.insert(
        "resolved_run_profile_id".to_string(),
        profile
            .get("id")
            .cloned()
            .ok_or_else(|| invalid_legacy("turn run profile is missing id"))?,
    );
    metadata.insert(
        "resolved_run_profile_version".to_string(),
        profile
            .get("version")
            .cloned()
            .ok_or_else(|| invalid_legacy("turn run profile is missing version"))?,
    );
    metadata.insert(
        "resolved_run_profile".to_string(),
        profile
            .get("resolved")
            .cloned()
            .unwrap_or_else(|| Value::Object(profile.clone())),
    );
    Ok(Value::Object(metadata))
}

fn turn_status(raw: &str) -> Result<ProcessLifecycleStatus, ProcessJournalStoreError> {
    match raw {
        "Queued" | "queued" => Ok(ProcessLifecycleStatus::Queued),
        "Running" | "running" => Ok(ProcessLifecycleStatus::Running),
        "BlockedApproval"
        | "blocked_approval"
        | "BlockedAuth"
        | "blocked_auth"
        | "BlockedResource"
        | "blocked_resource"
        | "BlockedDependentRun"
        | "blocked_dependent_run"
        | "BlockedExternalTool"
        | "blocked_external_tool" => Ok(ProcessLifecycleStatus::Suspended),
        "CancelRequested" | "cancel_requested" => Ok(ProcessLifecycleStatus::CancelRequested),
        "Cancelled" | "cancelled" => Ok(ProcessLifecycleStatus::Cancelled),
        "Completed" | "completed" => Ok(ProcessLifecycleStatus::Completed),
        "Failed" | "failed" => Ok(ProcessLifecycleStatus::Failed),
        "RecoveryRequired" | "recovery_required" => Ok(ProcessLifecycleStatus::RecoveryRequired),
        other => Err(invalid_legacy(format!(
            "unknown legacy turn status {other}"
        ))),
    }
}

fn turn_suspension(
    run: &Value,
    status: ProcessLifecycleStatus,
) -> Result<Option<ProcessSuspension>, ProcessJournalStoreError> {
    if status != ProcessLifecycleStatus::Suspended {
        return Ok(None);
    }
    let kind = match required_string(run, "status")?.as_str() {
        "BlockedApproval" | "blocked_approval" => "approval",
        "BlockedAuth" | "blocked_auth" => "authorization",
        "BlockedResource" | "blocked_resource" => "resource",
        "BlockedDependentRun" | "blocked_dependent_run" => "awaiting_child_process",
        "BlockedExternalTool" | "blocked_external_tool" => "external_tool",
        other => {
            return Err(invalid_legacy(format!(
                "legacy suspended turn has invalid status {other}"
            )));
        }
    };
    let suspension = json!({
        "kind": kind,
        "gate_ref": run.get("gate_ref").cloned().unwrap_or(Value::Null),
        "activity_id": run.get("blocked_activity_id").cloned().unwrap_or(Value::Null),
        "credential_requirements": run
            .get("credential_requirements")
            .cloned()
            .unwrap_or_else(|| json!([])),
    });
    serde_json::from_value(suspension)
        .map(Some)
        .map_err(deserialization)
}

fn legacy_turn_lease(
    run: &Value,
) -> Result<Option<ProcessLeaseSnapshot>, ProcessJournalStoreError> {
    let (Some(worker_id), Some(lease_token)) = (
        optional_string(run, "runner_id"),
        optional_string(run, "lease_token"),
    ) else {
        return Ok(None);
    };
    serde_json::from_value(json!({
        "worker_id": worker_id,
        "lease_token": lease_token,
        "lease_expires_at": run.get("lease_expires_at").cloned().unwrap_or(Value::Null),
        "last_heartbeat_at": run.get("last_heartbeat_at").cloned().unwrap_or(Value::Null),
        "claim_count": run.get("claim_count").and_then(Value::as_u64).unwrap_or_default(),
    }))
    .map(Some)
    .map_err(deserialization)
}

fn legacy_loop_checkpoint(
    value: &Value,
    checkpoint_states: &HashMap<String, Value>,
) -> Result<Option<ProcessCheckpointRecord>, ProcessJournalStoreError> {
    let state_ref = required_string(value, "state_ref")?;
    let payload = match value.get("payload").filter(|payload| !payload.is_null()) {
        Some(payload) => serde_json::from_value(payload.clone()).map_err(deserialization)?,
        None => {
            let stored = checkpoint_states.get(&state_ref).ok_or_else(|| {
                invalid_legacy(format!(
                    "legacy loop checkpoint {state_ref} has no checkpoint-state payload"
                ))
            })?;
            validate_checkpoint_state_metadata(value, stored)?;
            let payload_hex = required_string(stored, "payload_hex")?;
            hex::decode(payload_hex).map_err(|error| {
                invalid_legacy(format!(
                    "legacy checkpoint-state payload {state_ref} is not valid hex: {error}"
                ))
            })?
        }
    };
    let run_id: TurnRunId = required(value, "run_id")?;
    let turn_scope: TurnScope = required(value, "scope")?;
    let mut scope = turn_scope.to_resource_scope();
    scope.invocation_id = InvocationId::from_uuid(run_id.as_uuid());
    Ok(Some(ProcessCheckpointRecord {
        checkpoint_id: ProcessCheckpointId::from_trusted(required_string(value, "checkpoint_id")?),
        process_id: ProcessId::from_uuid(run_id.as_uuid()),
        scope,
        state_ref: ProcessCheckpointRef::from_trusted(state_ref),
        payload: ProcessCheckpointPayload::new(payload)
            .map_err(|error| invalid_legacy(error.to_string()))?,
        created_at: required(value, "created_at")?,
        metadata: json!({
            "turn_id": value.get("turn_id").cloned().unwrap_or(Value::Null),
            "schema_id": value.get("schema_id").cloned().unwrap_or(Value::Null),
            "schema_version": value.get("schema_version").cloned().unwrap_or(Value::Null),
            "kind": value.get("kind").cloned().unwrap_or(Value::Null),
            "gate_ref": value.get("gate_ref").cloned().unwrap_or(Value::Null),
        }),
    }))
}

fn validate_checkpoint_state_metadata(
    checkpoint: &Value,
    stored: &Value,
) -> Result<(), ProcessJournalStoreError> {
    for field in [
        "state_ref",
        "scope",
        "turn_id",
        "run_id",
        "schema_id",
        "schema_version",
        "kind",
    ] {
        if checkpoint.get(field) != stored.get(field) {
            return Err(invalid_legacy(format!(
                "legacy checkpoint-state metadata mismatch for {field}"
            )));
        }
    }
    Ok(())
}

fn legacy_tree_reservation(
    value: &Value,
) -> Result<Option<ProcessTreeReservation>, ProcessJournalStoreError> {
    let root: TurnRunId = required(value, "root_run_id")?;
    let released = value
        .get("released_children")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .cloned()
                .map(serde_json::from_value::<TurnRunId>)
                .map(|result| result.map(|id| ProcessId::from_uuid(id.as_uuid())))
                .collect::<Result<_, _>>()
        })
        .transpose()
        .map_err(deserialization)?
        .unwrap_or_default();
    Ok(Some(ProcessTreeReservation {
        root_process_id: ProcessId::from_uuid(root.as_uuid()),
        descendant_count: value
            .get("descendant_count")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        released_processes: released,
    }))
}

pub(super) async fn legacy_run_state_records<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<Vec<Value>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    legacy_authority_records(filesystem, "run-state", |path| {
        path.split('/').any(|segment| segment == "runs")
    })
    .await
}

async fn legacy_checkpoint_state_records<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<HashMap<String, Value>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let records = legacy_authority_records(filesystem, "checkpoint-state", |_| true).await?;
    let mut by_state_ref = HashMap::new();
    for record in records {
        let state_ref = required_string(&record, "state_ref")?;
        if let Some(previous) = by_state_ref.insert(state_ref.clone(), record.clone())
            && previous != record
        {
            return Err(invalid_legacy(format!(
                "conflicting legacy checkpoint-state record {state_ref}"
            )));
        }
    }
    Ok(by_state_ref)
}

async fn legacy_authority_records<F>(
    filesystem: &ScopedFilesystem<F>,
    authority: &str,
    include_file: impl Fn(&str) -> bool,
) -> Result<Vec<Value>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut pending = VecDeque::new();
    for raw_root in [format!("/{authority}"), "/legacy-tenants".to_string()] {
        let root = scoped(&raw_root)?;
        if filesystem.resolve(&ResourceScope::system(), &root).is_ok() {
            pending.push_back(root);
        }
    }
    let mut records = Vec::new();
    while let Some(path) = pending.pop_front() {
        let entries = match filesystem.list_dir(&ResourceScope::system(), &path).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let child = scoped(&format!(
                "{}/{}",
                path.as_str().trim_end_matches('/'),
                entry.name
            ))?;
            match entry.file_type {
                FileType::Directory => pending.push_back(child),
                FileType::File
                    if child
                        .as_str()
                        .split('/')
                        .any(|segment| segment == authority)
                        && include_file(child.as_str()) =>
                {
                    if let Some(versioned) = get_optional(filesystem, &child).await? {
                        records.push(decode(&versioned.entry.body)?);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(records)
}

fn legacy_capability_snapshot(
    value: &Value,
) -> Result<JournaledProcessSnapshot, ProcessJournalStoreError> {
    let invocation_id: InvocationId = required(value, "invocation_id")?;
    let process_id = ProcessId::from_uuid(invocation_id.as_uuid());
    let scope: ResourceScope = required(value, "scope")?;
    let raw_status = required_string(value, "status")?;
    let status = match raw_status.as_str() {
        "Running" | "running" => ProcessLifecycleStatus::Running,
        "BlockedApproval" | "blocked_approval" | "BlockedAuth" | "blocked_auth" => {
            ProcessLifecycleStatus::Suspended
        }
        "Completed" | "completed" => ProcessLifecycleStatus::Completed,
        "Failed" | "failed" => ProcessLifecycleStatus::Failed,
        other => {
            return Err(invalid_legacy(format!(
                "unknown legacy capability status {other}"
            )));
        }
    };
    let suspension = if status == ProcessLifecycleStatus::Suspended {
        let kind = if raw_status.eq_ignore_ascii_case("BlockedAuth")
            || raw_status.eq_ignore_ascii_case("blocked_auth")
        {
            "authorization"
        } else {
            "approval"
        };
        serde_json::from_value(json!({
            "kind": kind,
            "gate_ref": Value::Null,
            "activity_id": Value::Null,
            "credential_requirements": [],
        }))
        .map(Some)
        .map_err(deserialization)?
    } else {
        None
    };
    Ok(JournaledProcessSnapshot {
        process_id,
        process_kind: ProcessKind::CapabilityInvocationState,
        scope: scope.clone(),
        status,
        suspension,
        checkpoint_ref: None,
        checkpoint_kind: None,
        input_ref: None,
        failure: None,
        journal_cursor: ProcessJournalCursor(0),
        lease: None,
        crash_reclaim_count: 0,
        created_at: Utc::now(),
        owner_user_id: value
            .get("authenticated_actor_user_id")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(deserialization)?,
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        metadata: json!({
            "record_type": "capability_run",
            "invocation_id": invocation_id,
            "capability_id": value.get("capability_id").cloned().unwrap_or(Value::Null),
            "authenticated_actor_user_id": value
                .get("authenticated_actor_user_id")
                .cloned()
                .unwrap_or(Value::Null),
            "approval_request_id": value
                .get("approval_request_id")
                .cloned()
                .unwrap_or(Value::Null),
            "error_kind": value.get("error_kind").cloned().unwrap_or(Value::Null),
            "legacy_scope": scope,
        }),
    })
}

fn import_legacy_idempotency(
    state: &mut ProcessJournalMaterializedState,
    records: &[Value],
) -> Result<(), ProcessJournalStoreError> {
    for record in records {
        let Some(operation_id) = optional_string(record, "key") else {
            continue;
        };
        let Some(run_id) = legacy_idempotency_run_id(record)? else {
            continue;
        };
        let process_id = ProcessId::from_uuid(run_id.as_uuid());
        let Some(snapshot) = state.processes.get(&process_id).cloned() else {
            continue;
        };
        match required_string(record, "operation")?.as_str() {
            "Submit" | "submit" => {
                state.import_deployed_submit_idempotency(&operation_id, snapshot)?;
            }
            "Retry" | "retry" => {
                state.import_deployed_submit_idempotency(
                    format!("retry:{operation_id}").as_str(),
                    snapshot,
                )?;
            }
            "Resume" | "resume" => {
                state.import_deployed_control_idempotency("resume", &operation_id, snapshot);
            }
            "Cancel" | "cancel" => {
                state.import_deployed_control_idempotency("cancel", &operation_id, snapshot);
            }
            _ => {}
        }
    }
    Ok(())
}

fn legacy_idempotency_run_id(
    record: &Value,
) -> Result<Option<TurnRunId>, ProcessJournalStoreError> {
    if let Some(value) = record.get("run_id").filter(|value| !value.is_null()) {
        return serde_json::from_value(value.clone())
            .map(Some)
            .map_err(deserialization);
    }
    let replay = record.get("replay");
    let candidate = replay.and_then(|value| {
        value
            .get("run_id")
            .or_else(|| value.as_object().and_then(|object| object.values().next()))
            .and_then(|value| value.get("run_id").or(Some(value)))
    });
    candidate
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(deserialization)
}

fn optional_turn_process_id(
    value: &Value,
    field: &str,
) -> Result<Option<ProcessId>, ProcessJournalStoreError> {
    optional::<TurnRunId>(value, field)
        .map(|result| result.map(|id| ProcessId::from_uuid(id.as_uuid())))
}

fn required<T>(value: &Value, field: &str) -> Result<T, ProcessJournalStoreError>
where
    T: DeserializeOwned,
{
    let value = value
        .get(field)
        .cloned()
        .ok_or_else(|| invalid_legacy(format!("legacy record is missing {field}")))?;
    serde_json::from_value(value).map_err(deserialization)
}

fn optional<T>(value: &Value, field: &str) -> Result<Option<T>, ProcessJournalStoreError>
where
    T: DeserializeOwned,
{
    value
        .get(field)
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(deserialization)
}

fn required_string(value: &Value, field: &str) -> Result<String, ProcessJournalStoreError> {
    optional_string(value, field)
        .ok_or_else(|| invalid_legacy(format!("legacy record is missing {field}")))
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn copy_required(
    source: &Value,
    target: &mut Map<String, Value>,
    field: &str,
) -> Result<(), ProcessJournalStoreError> {
    target.insert(
        field.to_string(),
        source
            .get(field)
            .cloned()
            .ok_or_else(|| invalid_legacy(format!("legacy record is missing {field}")))?,
    );
    Ok(())
}

fn collection_values(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(values)) => values.clone(),
        Some(Value::Object(values)) => values.values().cloned().collect(),
        _ => Vec::new(),
    }
}

async fn get_optional<F>(
    filesystem: &ScopedFilesystem<F>,
    path: &ScopedPath,
) -> Result<Option<ironclaw_filesystem::VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    match filesystem.get(&ResourceScope::system(), path).await {
        Ok(value) => Ok(value),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn scoped(path: &str) -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new(path).map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))
}

fn decode<T>(bytes: &[u8]) -> Result<T, ProcessJournalStoreError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(deserialization)
}

fn serialization(error: serde_json::Error) -> ProcessJournalStoreError {
    ProcessJournalStoreError::Serialization(error.to_string())
}

fn deserialization(error: serde_json::Error) -> ProcessJournalStoreError {
    ProcessJournalStoreError::Deserialization(error.to_string())
}

fn invalid_legacy(message: impl Into<String>) -> ProcessJournalStoreError {
    ProcessJournalStoreError::Deserialization(message.into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    use super::*;

    #[tokio::test]
    async fn legacy_row_collection_imports_beyond_one_backend_page() {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").expect("turns alias"),
            VirtualPath::new("/engine/turns").expect("turns target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("migration mount view");
        let filesystem = ScopedFilesystem::with_fixed_view(Arc::new(InMemoryBackend::new()), view);
        for index in 0..=ironclaw_filesystem::Page::MAX_LIMIT {
            filesystem
                .put(
                    &ResourceScope::system(),
                    &scoped(&format!("/turns/rows/v1/runs/{index:04}.json"))
                        .expect("legacy row path"),
                    Entry::bytes(
                        serde_json::to_vec(&json!({
                            "journal_seq": index + 1,
                            "value": {"row": index}
                        }))
                        .expect("serialize legacy row"),
                    ),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed legacy row");
        }

        let rows = legacy_row_collection(&filesystem, "runs")
            .await
            .expect("enumerate complete legacy collection");
        assert_eq!(
            rows.len(),
            ironclaw_filesystem::Page::MAX_LIMIT as usize + 1
        );
    }
}
