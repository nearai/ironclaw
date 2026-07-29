use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_processes::{
    GetProcessCheckpointRequest, ProcessCheckpointId, ProcessCheckpointPayload,
    ProcessCheckpointPort, ProcessCheckpointRecord, ProcessCheckpointRef,
    RecordProcessCheckpointRequest,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    CheckpointSchemaId, GetLoopCheckpointRequest, LoopCheckpointKind, LoopCheckpointRecord,
    LoopCheckpointStateRef, LoopCheckpointStore, LoopGateRef, PutLoopCheckpointRequest,
    RunProfileVersion, TurnCheckpointId, TurnError, TurnId, TurnRunId, TurnScope,
};

pub struct ProcessLoopCheckpointStore {
    checkpoints: Arc<dyn ProcessCheckpointPort<Error = TurnError>>,
}

impl ProcessLoopCheckpointStore {
    pub fn new(checkpoints: Arc<dyn ProcessCheckpointPort<Error = TurnError>>) -> Self {
        Self { checkpoints }
    }
}

#[async_trait]
impl LoopCheckpointStore for ProcessLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let checkpoint_id = TurnCheckpointId::new();
        let process_record = self
            .checkpoints
            .record_process_checkpoint(RecordProcessCheckpointRequest {
                checkpoint_id: process_checkpoint_id(checkpoint_id),
                process_id: ironclaw_host_api::ProcessId::from_uuid(request.run_id.as_uuid()),
                scope: process_checkpoint_scope(&request.scope, request.run_id),
                state_ref: ProcessCheckpointRef::new(request.state_ref.as_str()).map_err(
                    |error| TurnError::InvalidRequest {
                        reason: error.to_string(),
                    },
                )?,
                payload: ProcessCheckpointPayload::new(request.payload.into_payload_bytes())
                    .map_err(|error| TurnError::InvalidRequest {
                        reason: error.to_string(),
                    })?,
                created_at: chrono::Utc::now(),
                link_to_process: request.kind != LoopCheckpointKind::Final,
                metadata: json!({
                    "turn_id": request.turn_id,
                    "schema_id": request.schema_id,
                    "schema_version": request.schema_version,
                    "kind": request.kind,
                    "gate_ref": request.gate_ref,
                }),
            })
            .await?;
        loop_checkpoint_record_from_process(
            checkpoint_id,
            request.scope,
            request.run_id,
            process_record,
        )
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let Some(record) = self
            .checkpoints
            .get_process_checkpoint(GetProcessCheckpointRequest {
                checkpoint_id: process_checkpoint_id(request.checkpoint_id),
                process_id: ironclaw_host_api::ProcessId::from_uuid(request.run_id.as_uuid()),
                scope: process_checkpoint_scope(&request.scope, request.run_id),
            })
            .await?
        else {
            return Ok(None);
        };
        loop_checkpoint_record_from_process(
            request.checkpoint_id,
            request.scope,
            request.run_id,
            record,
        )
        .map(Some)
    }
}

fn process_checkpoint_id(checkpoint_id: TurnCheckpointId) -> ProcessCheckpointId {
    ProcessCheckpointId::from_trusted(checkpoint_id.as_uuid().to_string())
}

fn process_checkpoint_scope(scope: &TurnScope, run_id: TurnRunId) -> ResourceScope {
    let mut process_scope = scope.to_resource_scope();
    process_scope.invocation_id = InvocationId::from_uuid(run_id.as_uuid());
    process_scope
}

fn loop_checkpoint_record_from_process(
    checkpoint_id: TurnCheckpointId,
    scope: TurnScope,
    run_id: TurnRunId,
    record: ProcessCheckpointRecord,
) -> Result<LoopCheckpointRecord, TurnError> {
    #[derive(Deserialize)]
    struct Metadata {
        turn_id: TurnId,
        schema_id: CheckpointSchemaId,
        schema_version: RunProfileVersion,
        kind: LoopCheckpointKind,
        gate_ref: Option<LoopGateRef>,
    }

    let metadata: Metadata =
        serde_json::from_value(record.metadata).map_err(|error| TurnError::Unavailable {
            reason: format!("process checkpoint metadata is invalid: {error}"),
        })?;
    let state_ref = LoopCheckpointStateRef::new(record.state_ref.as_str()).map_err(|reason| {
        TurnError::Unavailable {
            reason: format!("process checkpoint state ref is invalid: {reason}"),
        }
    })?;
    let payload =
        crate::RedactedCheckpointPayload::new(record.payload.into_bytes()).map_err(|reason| {
            TurnError::Unavailable {
                reason: format!("process checkpoint payload is invalid: {reason}"),
            }
        })?;
    Ok(LoopCheckpointRecord {
        checkpoint_id,
        scope,
        turn_id: metadata.turn_id,
        run_id,
        state_ref,
        payload: Some(payload),
        schema_id: metadata.schema_id,
        schema_version: metadata.schema_version,
        kind: metadata.kind,
        gate_ref: metadata.gate_ref,
        created_at: record.created_at,
    })
}
