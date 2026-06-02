use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, PermissionMode, ResourceScope, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use ironclaw_triggers::{
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerSchedule, TriggerSourceKind, TriggerState,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{
    FIRST_PARTY_MAX_OUTPUT_BYTES, bounded_input_size, bounded_output_bytes,
    first_party_capability_manifest, input_error, resource_profile,
};

const DEFAULT_TRIGGER_LIST_LIMIT: usize = 100;
const MAX_TRIGGER_LIST_LIMIT: usize = 100;

pub const TRIGGER_CREATE_CAPABILITY_ID: &str = "builtin.trigger_create";
pub const TRIGGER_LIST_CAPABILITY_ID: &str = "builtin.trigger_list";
pub const TRIGGER_REMOVE_CAPABILITY_ID: &str = "builtin.trigger_remove";

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            TRIGGER_CREATE_CAPABILITY_ID,
            "Create a caller-scoped scheduled trigger",
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            TRIGGER_LIST_CAPABILITY_ID,
            "List scheduled triggers owned by the current caller scope",
            vec![EffectKind::DispatchCapability],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            TRIGGER_REMOVE_CAPABILITY_ID,
            "Remove a caller-scoped scheduled trigger",
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            PermissionMode::Ask,
            resource_profile(),
        )?,
    ])
}

pub(super) fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    repository: Arc<dyn TriggerRepository>,
) -> Result<(), HostApiError> {
    let handler = Arc::new(TriggerManagementToolHandler { repository });
    registry.insert_handler(
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?, handler);
    Ok(())
}

struct TriggerManagementToolHandler {
    repository: Arc<dyn TriggerRepository>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for TriggerManagementToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        bounded_input_size(request.capability_id.as_str(), &request.input)?;
        let started = Instant::now();
        let output = match request.capability_id.as_str() {
            TRIGGER_CREATE_CAPABILITY_ID => {
                create_trigger(&*self.repository, &request.scope, request.input).await?
            }
            TRIGGER_LIST_CAPABILITY_ID => {
                list_triggers(&*self.repository, &request.scope, request.input).await?
            }
            TRIGGER_REMOVE_CAPABILITY_ID => {
                remove_trigger(&*self.repository, &request.scope, request.input).await?
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let output_bytes = bounded_output_bytes(&output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            usage_with_elapsed(started, output_bytes),
        ))
    }
}

#[derive(Deserialize)]
struct TriggerCreateInput {
    name: String,
    prompt: String,
    cron: String,
}

#[derive(Deserialize)]
struct TriggerRemoveInput {
    trigger_id: String,
}

#[derive(Deserialize)]
struct TriggerListInput {
    limit: Option<usize>,
}

async fn create_trigger(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerCreateInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let schedule = TriggerSchedule::cron(input.cron).map_err(trigger_input_error)?;
    let now = Utc::now();
    let next_run_at = schedule
        .next_slot_after(now)
        .map_err(trigger_input_error)?
        .ok_or_else(input_error)?;
    let record = TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id: scope.tenant_id.clone(),
        creator_user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        name: input.name,
        source: TriggerSourceKind::Schedule,
        schedule,
        completion_policy: TriggerCompletionPolicy::Recurring,
        prompt: input.prompt,
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: now,
    };
    record.validate().map_err(trigger_input_error)?;
    repository
        .upsert_trigger(record.clone())
        .await
        .map_err(trigger_repository_error)?;
    Ok(json!({
        "trigger": trigger_output(&record),
    }))
}

async fn list_triggers(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerListInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let limit = input
        .limit
        .unwrap_or(DEFAULT_TRIGGER_LIST_LIMIT)
        .min(MAX_TRIGGER_LIST_LIMIT);
    let records = repository
        .list_scoped_triggers(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            limit,
        )
        .await
        .map_err(trigger_repository_error)?
        .into_iter()
        .map(|record| trigger_list_output(&record))
        .collect::<Vec<_>>();
    Ok(json!({ "triggers": records }))
}

async fn remove_trigger(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerRemoveInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let trigger_id = TriggerId::parse(&input.trigger_id).map_err(trigger_input_error)?;
    let removed = repository
        .remove_scoped_trigger(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            trigger_id,
        )
        .await
        .map_err(trigger_repository_error)?;
    Ok(json!({
        "removed": removed.is_some(),
        "trigger": removed.as_ref().map(trigger_output),
    }))
}

fn trigger_output(record: &TriggerRecord) -> Value {
    json!({
        "trigger_id": record.trigger_id.to_string(),
        "tenant_id": record.tenant_id.as_str(),
        "creator_user_id": record.creator_user_id.as_str(),
        "agent_id": record.agent_id.as_ref().map(|id| id.as_str()),
        "project_id": record.project_id.as_ref().map(|id| id.as_str()),
        "name": record.name,
        "source": record.source,
        "schedule": record.schedule,
        "completion_policy": record.completion_policy,
        "prompt": record.prompt,
        "state": record.state,
        "next_run_at": record.next_run_at,
        "last_run_at": record.last_run_at,
        "last_fired_slot": record.last_fired_slot,
        "last_status": record.last_status,
        "active_fire_slot": record.active_fire_slot,
        "active_run_ref": record.active_run_ref.as_ref().map(ToString::to_string),
        "created_at": record.created_at,
    })
}

fn trigger_list_output(record: &TriggerRecord) -> Value {
    let mut output = trigger_output(record);
    if let Some(object) = output.as_object_mut() {
        object.remove("prompt");
    }
    output
}

fn trigger_input_error(_error: TriggerError) -> FirstPartyCapabilityError {
    input_error()
}

fn trigger_repository_error(error: TriggerError) -> FirstPartyCapabilityError {
    tracing::debug!(
        trigger_error = %error,
        "trigger management capability repository operation failed"
    );
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
}

fn usage_with_elapsed(started: Instant, output_bytes: u64) -> ResourceUsage {
    ResourceUsage {
        wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        output_bytes,
        ..ResourceUsage::default()
    }
}
