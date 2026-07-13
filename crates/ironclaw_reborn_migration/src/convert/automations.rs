//! Automations converter: v1 routines + engine-v2 missions → Reborn triggers
//! (plus mission threads).
//!
//! **The "no losses" core.** Reborn's trigger substrate only supports scheduled
//! (cron/once) sources, so a cron routine/mission converts to a `TriggerRecord`
//! losslessly, while every other trigger source and every automation-only field
//! Reborn cannot hold is recorded on the report rather than dropped silently:
//!
//! - `Trigger::Cron` / `MissionCadence::Cron` → `TriggerSchedule::Cron` ✅
//! - `Trigger::{Event,SystemEvent,Webhook,Manual}` /
//!   `MissionCadence::{OnEvent,OnSystemEvent,Webhook,Manual}` → **no trigger**
//!   (Reborn has only `TriggerSourceKind::Schedule`); recorded as a loss.
//! - routine guardrails / notify / run counters, mission focus / approach /
//!   success-criteria / notify → **no target field**; recorded.
//! - `routine_runs` history → **no public run-history insert** on
//!   `TriggerRepository`; recorded per routine.
//! - Reborn has no `Failed` trigger state → a failed routine/mission maps to
//!   `Paused` and the degrade is recorded.
//!
//! Engine-v2 mission threads (`thread_history`) are migrated as Reborn threads
//! scoped under `ThreadScope.mission_id` — the one place "mission" survives in
//! Reborn (a scope dimension, not a durable entity).

use std::collections::{BTreeMap, BTreeSet};

use ironclaw::agent::routine::{Routine, RoutineAction, Trigger};
use ironclaw_host_api::ProjectId;
use ironclaw_reborn_identity::RebornUserStatus;
use ironclaw_triggers::{TriggerRecord, TriggerSchedule, TriggerSourceKind, TriggerState};
use uuid::Uuid;

use crate::convert::threads::{ImportMessage, ImportRole, ThreadImport, write_thread};
use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;
use crate::target::ids;
use crate::v2_model::{self, EngineThread, Mission, MissionCadence, MissionStatus};

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    imported_project_ids: &BTreeSet<String>,
) -> Result<(), MigrationError> {
    convert_routines(src, tgt, options, report).await?;
    convert_missions(src, tgt, options, report, imported_project_ids).await?;
    Ok(())
}

// ── v1 routines ─────────────────────────────────────────────────────────────

async fn convert_routines(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let routines = src
        .db
        .list_all_routines()
        .await
        .map_err(|e| MigrationError::ReadSource {
            domain: "routines".into(),
            reason: e.to_string(),
        })?;

    for routine in routines {
        convert_routine(tgt, options, report, routine).await?;
    }
    Ok(())
}

async fn convert_routine(
    tgt: &RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    routine: Routine,
) -> Result<(), MigrationError> {
    let source_id = format!("routine:{}", routine.name);

    // Only cron routines have a Reborn trigger target.
    let (schedule, is_cron) = match &routine.trigger {
        Trigger::Cron { schedule, timezone } => {
            let tz = timezone.clone().unwrap_or_else(|| "UTC".to_string());
            match TriggerSchedule::cron_with_timezone(schedule.clone(), tz) {
                Ok(schedule) => (Some(schedule), true),
                Err(e) => {
                    report.record_loss(
                        Domain::Routine,
                        &source_id,
                        "trigger.cron",
                        LossReason::Unparseable,
                        format!("invalid cron expression '{schedule}': {e}"),
                    );
                    (None, true)
                }
            }
        }
        other => {
            report.record_loss(
                Domain::Routine,
                &source_id,
                format!("trigger.{}", trigger_tag(other)),
                LossReason::NoTargetConcept,
                "Reborn triggers support only scheduled (cron/once) sources; \
                 event/system-event/webhook/manual routines have no trigger target \
                 (consider a Reborn hook)"
                    .to_string(),
            );
            (None, false)
        }
    };

    let Some(schedule) = schedule else {
        // Nothing to write; losses already recorded.
        record_routine_field_losses(report, &source_id, &routine, is_cron);
        return Ok(());
    };

    let prompt = routine_prompt(&routine.action);
    // v1 routines have no terminal-failed status distinct from disabled;
    // consecutive_failures is recorded as a field loss below.
    let mut state = if routine.enabled {
        TriggerState::Scheduled
    } else {
        TriggerState::Paused
    };
    let now = match routine.next_fire_at {
        Some(next_fire_at) => next_fire_at,
        None => {
            report.record_loss(
                Domain::Routine,
                &source_id,
                "next_fire_at",
                LossReason::Degraded,
                "routine had no next_fire_at; its deterministic created_at is retained and the routine is imported Paused",
            );
            state = TriggerState::Paused;
            routine.created_at
        }
    };

    record_routine_field_losses(report, &source_id, &routine, is_cron);

    // A malformed source user id is a per-item loss, not a run abort.
    let Some(creator_user_id) =
        report.valid_user_id(Domain::Routine, &source_id, "user_id", &routine.user_id)
    else {
        return Ok(());
    };
    pause_for_inactive_owner(
        tgt,
        report,
        Domain::Routine,
        &source_id,
        &creator_user_id,
        &mut state,
    )
    .await?;

    let migration_identity = ids::MigrationIdentity::from_report(report)?;
    let record = TriggerRecord {
        trigger_id: migration_identity.trigger_id(
            "routine",
            &routine.id.to_string(),
            &tgt.tenant_id,
            &tgt.agent_id,
        )?,
        tenant_id: tgt.tenant_id.clone(),
        creator_user_id,
        agent_id: Some(tgt.agent_id.clone()),
        project_id: Option::<ProjectId>::None,
        name: routine.name.clone(),
        source: TriggerSourceKind::Schedule,
        schedule,
        prompt,
        // v1 routines have no per-trigger delivery routing; migrated triggers
        // use the creator's outbound delivery preference like before.
        delivery_target: None,
        state,
        next_run_at: now,
        last_run_at: routine.last_run_at,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: routine.created_at,
    };

    if !options.dry_run {
        tgt.compare_and_upsert_trigger(&source_id, record).await?;
    }
    report.stats.routines += 1;
    report.stats.triggers += 1;
    Ok(())
}

/// Compose the single Reborn trigger prompt from a v1 routine action, recording
/// the action fields Reborn's `prompt`-only trigger cannot hold.
fn routine_prompt(action: &RoutineAction) -> String {
    match action {
        RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        RoutineAction::FullJob {
            title, description, ..
        } => {
            if title.trim().is_empty() {
                description.clone()
            } else {
                format!("{title}\n\n{description}")
            }
        }
    }
}

/// Record every routine field that has no Reborn trigger representation.
fn record_routine_field_losses(
    report: &mut MigrationReport,
    source_id: &str,
    routine: &Routine,
    is_cron: bool,
) {
    // Action extras beyond the prompt.
    match &routine.action {
        RoutineAction::Lightweight {
            context_paths,
            max_tokens,
            use_tools,
            max_tool_rounds,
            ..
        } => {
            if !context_paths.is_empty() || *max_tokens != 0 || *use_tools || *max_tool_rounds != 0
            {
                report.record_loss(
                    Domain::Routine,
                    source_id,
                    "action.lightweight_params",
                    LossReason::NoTargetField,
                    "Reborn trigger carries only a prompt; context_paths/max_tokens/\
                     use_tools/max_tool_rounds are dropped"
                        .to_string(),
                );
            }
        }
        RoutineAction::FullJob { max_iterations, .. } => {
            let _ = max_iterations;
            report.record_loss(
                Domain::Routine,
                source_id,
                "action.full_job_params",
                LossReason::NoTargetField,
                "Reborn trigger carries only a prompt; full-job max_iterations is dropped"
                    .to_string(),
            );
        }
    }

    // Guardrails + notify + run counters.
    report.record_loss(
        Domain::Routine,
        source_id,
        "guardrails+notify+counters",
        LossReason::NoTargetField,
        "Reborn triggers have no cooldown/max_concurrent/dedup, notify config, \
         run_count, or consecutive_failures fields"
            .to_string(),
    );

    // Run history: no public insert path on TriggerRepository.
    if is_cron {
        report.record_loss(
            Domain::Routine,
            source_id,
            "routine_runs",
            LossReason::NoTargetField,
            "TriggerRepository exposes no public run-history insert; historical \
             routine_runs cannot be written to trigger_run_history"
                .to_string(),
        );
    }
}

fn trigger_tag(trigger: &Trigger) -> &'static str {
    match trigger {
        Trigger::Cron { .. } => "cron",
        Trigger::Event { .. } => "event",
        Trigger::SystemEvent { .. } => "system_event",
        Trigger::Webhook { .. } => "webhook",
        Trigger::Manual => "manual",
    }
}

// ── engine-v2 missions ──────────────────────────────────────────────────────

async fn convert_missions(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    imported_project_ids: &BTreeSet<String>,
) -> Result<(), MigrationError> {
    let users = src.distinct_users().await?;
    let mut engine_threads: BTreeMap<Uuid, IndexedEngineThread> = BTreeMap::new();
    let mut missions: BTreeMap<Uuid, IndexedMission> = BTreeMap::new();
    for user_id in &users {
        let docs = src.all_memory_documents(user_id).await?;
        for doc in &docs {
            if !v2_model::is_engine_path(&doc.path) {
                continue;
            }
            if doc.path.ends_with("mission.json") {
                match parse_engine_document::<Mission>(&doc.content) {
                    Ok((representation, mission)) => {
                        let owner = if mission.user_id.is_empty() {
                            user_id.clone()
                        } else {
                            mission.user_id.clone()
                        };
                        let indexed = IndexedMission {
                            source: engine_document_source(doc),
                            owner,
                            representation,
                            mission,
                        };
                        insert_mission(&mut missions, indexed)?;
                    }
                    Err(e) => report.record_loss(
                        Domain::Mission,
                        doc.path.clone(),
                        "*",
                        LossReason::Unparseable,
                        format!("could not parse mission.json: {e}"),
                    ),
                }
            } else if doc.path.contains("/threads/") && doc.path.ends_with(".json") {
                match parse_engine_document::<EngineThread>(&doc.content) {
                    Ok((representation, thread)) => {
                        let indexed = IndexedEngineThread {
                            source: engine_document_source(doc),
                            representation,
                            thread,
                        };
                        insert_engine_thread(&mut engine_threads, indexed)?;
                    }
                    Err(e) => report.record_loss(
                        Domain::Mission,
                        doc.path.clone(),
                        "*",
                        LossReason::Unparseable,
                        format!("could not parse engine thread blob: {e}"),
                    ),
                }
            }
        }
    }

    let referenced: BTreeSet<Uuid> = missions
        .values()
        .flat_map(|indexed| indexed.mission.thread_history.iter().copied())
        .collect();
    for indexed in missions.values() {
        convert_mission(
            tgt,
            options,
            report,
            &indexed.owner,
            &indexed.mission,
            &engine_threads,
            imported_project_ids,
        )
        .await?;
    }
    for id in engine_threads.keys() {
        if !referenced.contains(id) {
            report.record_loss(
                Domain::Mission,
                format!("thread:{id}"),
                "*",
                LossReason::NoTargetConcept,
                "engine thread blob is not referenced by any mission thread_history; there is no Reborn mission owner to migrate it under",
            );
        }
    }
    Ok(())
}

async fn convert_mission(
    tgt: &RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    user_id: &str,
    mission: &Mission,
    engine_threads: &BTreeMap<Uuid, IndexedEngineThread>,
    imported_project_ids: &BTreeSet<String>,
) -> Result<(), MigrationError> {
    let source_id = format!("mission:{}", mission.name);
    let owner = if mission.user_id.is_empty() {
        user_id.to_string()
    } else {
        mission.user_id.clone()
    };

    // Mission-only fields with no Reborn home.
    record_mission_field_losses(report, &source_id, mission);

    // Cadence → trigger (cron only).
    match &mission.cadence {
        MissionCadence::Cron {
            expression,
            timezone,
        } => {
            let tz = timezone.clone().unwrap_or_else(|| "UTC".to_string());
            match TriggerSchedule::cron_with_timezone(expression.clone(), tz) {
                Ok(schedule) => {
                    let mut state = match mission.status {
                        MissionStatus::Active => TriggerState::Scheduled,
                        MissionStatus::Paused => TriggerState::Paused,
                        MissionStatus::Completed => TriggerState::Completed,
                        MissionStatus::Failed => {
                            report.record_loss(
                                Domain::Mission,
                                &source_id,
                                "status.failed",
                                LossReason::Degraded,
                                "Reborn has no Failed trigger state; mapped to Paused".to_string(),
                            );
                            TriggerState::Paused
                        }
                    };
                    // A malformed mission owner id is a per-item loss (the
                    // trigger is skipped); mission threads below still validate
                    // their own owner independently. Loss recorded by the helper.
                    if let Some(creator_user_id) =
                        report.valid_user_id(Domain::Mission, &source_id, "user_id", &owner)
                    {
                        let (next_run_at, synthesized_next_run) =
                            mission_next_run_at(report, &source_id, mission);
                        // A source without a durable next fire cannot safely be
                        // armed. Keep the deterministic historical timestamp,
                        // but require an operator to review and resume it.
                        if synthesized_next_run && state == TriggerState::Scheduled {
                            state = TriggerState::Paused;
                        }
                        pause_for_inactive_owner(
                            tgt,
                            report,
                            Domain::Mission,
                            &source_id,
                            &creator_user_id,
                            &mut state,
                        )
                        .await?;
                        let project_id = resolve_mission_project(
                            report,
                            &source_id,
                            mission,
                            imported_project_ids,
                            &mut state,
                        )?;
                        let migration_identity = ids::MigrationIdentity::from_report(report)?;
                        let record = TriggerRecord {
                            trigger_id: migration_identity.trigger_id(
                                "mission",
                                &mission.id.to_string(),
                                &tgt.tenant_id,
                                &tgt.agent_id,
                            )?,
                            tenant_id: tgt.tenant_id.clone(),
                            creator_user_id,
                            agent_id: Some(tgt.agent_id.clone()),
                            project_id,
                            name: mission.name.clone(),
                            source: TriggerSourceKind::Schedule,
                            schedule,
                            // v1 missions have no per-trigger delivery routing.
                            delivery_target: None,
                            prompt: if mission.goal.trim().is_empty() {
                                mission.name.clone()
                            } else {
                                mission.goal.clone()
                            },
                            state,
                            next_run_at,
                            last_run_at: None,
                            last_fired_slot: None,
                            last_status: None,
                            active_fire_slot: None,
                            active_run_ref: None,
                            created_at: mission.created_at,
                        };
                        if !options.dry_run {
                            tgt.compare_and_upsert_trigger(&source_id, record).await?;
                        }
                        report.stats.triggers += 1;
                    }
                }
                Err(e) => report.record_loss(
                    Domain::Mission,
                    &source_id,
                    "cadence.cron",
                    LossReason::Unparseable,
                    format!("invalid cron expression '{expression}': {e}"),
                ),
            }
        }
        other => report.record_loss(
            Domain::Mission,
            &source_id,
            format!("cadence.{}", other.tag()),
            LossReason::NoTargetConcept,
            "Reborn triggers support only scheduled sources; non-cron mission \
             cadences have no trigger target"
                .to_string(),
        ),
    }

    report.stats.missions += 1;

    let thread_project_id = mission
        .project_id
        .filter(|id| imported_project_ids.contains(id.to_string().as_str()));
    let mut migrated_thread_ids = BTreeSet::new();
    for tid in &mission.thread_history {
        if !migrated_thread_ids.insert(*tid) {
            continue;
        }
        let Some(indexed_thread) = engine_threads.get(tid) else {
            report.record_loss(
                Domain::Mission,
                &source_id,
                format!("thread:{tid}"),
                LossReason::Unparseable,
                "mission thread_history references a thread blob not found in the \
                 engine runtime documents"
                    .to_string(),
            );
            continue;
        };
        let thread = &indexed_thread.thread;
        let import = ThreadImport {
            thread_id: thread.id,
            owner_user: owner.clone(),
            title: thread.title.clone().or_else(|| Some(mission.name.clone())),
            project_id: thread_project_id,
            mission_id: Some(mission.id),
            provenance: serde_json::json!({
                "source": "engine_v2_mission_thread",
                "mission": mission.name,
                "goal": thread.goal,
                "created_at": thread.created_at.to_rfc3339(),
            }),
            messages: thread
                .messages
                .iter()
                .map(|m| ImportMessage {
                    role: engine_role(m.role),
                    raw_role: format!("{:?}", m.role),
                    content: m.content.clone(),
                    created_at: m.timestamp,
                    orig_id: None,
                })
                .collect(),
        };
        if options.dry_run {
            report.stats.threads += 1;
            report.stats.messages += import
                .messages
                .iter()
                .filter(|m| m.role != ImportRole::Other)
                .count();
            // Match the real write path, which records a loss per non-user/
            // assistant transcript message, so `--dry-run` reports the same gap
            // set instead of under-counting.
            crate::convert::threads::record_other_role_losses(report, &import);
        } else {
            write_thread(tgt, options, report, import).await?;
        }
    }

    Ok(())
}

struct IndexedMission {
    source: String,
    owner: String,
    representation: serde_json::Value,
    mission: Mission,
}

struct IndexedEngineThread {
    source: String,
    representation: serde_json::Value,
    thread: EngineThread,
}

fn parse_engine_document<T: serde::de::DeserializeOwned>(
    content: &str,
) -> Result<(serde_json::Value, T), serde_json::Error> {
    let representation: serde_json::Value = serde_json::from_str(content)?;
    let parsed = serde_json::from_value(representation.clone())?;
    Ok((representation, parsed))
}

fn engine_document_source(document: &ironclaw::workspace::MemoryDocument) -> String {
    format!(
        "user={} agent={} path={}",
        document.user_id,
        document
            .agent_id
            .map_or_else(|| "unscoped".to_string(), |id| id.to_string()),
        document.path
    )
}

fn insert_mission(
    missions: &mut BTreeMap<Uuid, IndexedMission>,
    candidate: IndexedMission,
) -> Result<(), MigrationError> {
    if let Some(existing) = missions.get(&candidate.mission.id) {
        if existing.representation == candidate.representation && existing.owner == candidate.owner
        {
            return Ok(());
        }
        return Err(divergent_engine_document(
            "mission",
            candidate.mission.id,
            &existing.source,
            &candidate.source,
        ));
    }
    missions.insert(candidate.mission.id, candidate);
    Ok(())
}

fn insert_engine_thread(
    threads: &mut BTreeMap<Uuid, IndexedEngineThread>,
    candidate: IndexedEngineThread,
) -> Result<(), MigrationError> {
    if let Some(existing) = threads.get(&candidate.thread.id) {
        if existing.representation == candidate.representation {
            return Ok(());
        }
        return Err(divergent_engine_document(
            "thread",
            candidate.thread.id,
            &existing.source,
            &candidate.source,
        ));
    }
    threads.insert(candidate.thread.id, candidate);
    Ok(())
}

fn divergent_engine_document(
    kind: &str,
    id: Uuid,
    existing_source: &str,
    candidate_source: &str,
) -> MigrationError {
    MigrationError::ReadSource {
        domain: format!("engine {kind} {id}"),
        reason: format!(
            "source documents {existing_source} and {candidate_source} contain divergent state for the same durable id"
        ),
    }
}

async fn pause_for_inactive_owner(
    tgt: &RebornTarget,
    report: &mut MigrationReport,
    domain: Domain,
    source_id: &str,
    creator_user_id: &ironclaw_host_api::UserId,
    state: &mut TriggerState,
) -> Result<(), MigrationError> {
    let owner = tgt
        .user_directory(creator_user_id.clone())
        .get_user(creator_user_id)
        .await
        .map_err(|error| MigrationError::WriteTarget {
            domain: format!("trigger owner {creator_user_id}"),
            reason: error.to_string(),
        })?;
    if !matches!(owner, Some(user) if user.status == RebornUserStatus::Active) {
        *state = TriggerState::Paused;
        report.record_loss(
            domain,
            source_id,
            "owner.status",
            LossReason::Degraded,
            "automation owner is not an active migrated user; trigger imported Paused",
        );
    }
    Ok(())
}

fn resolve_mission_project(
    report: &mut MigrationReport,
    source_id: &str,
    mission: &Mission,
    imported_project_ids: &BTreeSet<String>,
    state: &mut TriggerState,
) -> Result<Option<ProjectId>, MigrationError> {
    let Some(source_project_id) = mission.project_id else {
        return Ok(None);
    };
    let project_id = ProjectId::new(source_project_id.to_string()).map_err(|error| {
        MigrationError::InvalidInput(format!(
            "mission {} has invalid project id: {error}",
            mission.id
        ))
    })?;
    if imported_project_ids.contains(project_id.as_str()) {
        return Ok(Some(project_id));
    }
    *state = TriggerState::Paused;
    report.record_loss(
        Domain::Mission,
        source_id,
        "project_id",
        LossReason::Degraded,
        "mission references a project that was not imported; project scope was omitted and the trigger imported Paused",
    );
    Ok(None)
}

/// The trigger's `next_run_at` for a migrated mission. A mission with an
/// explicit `next_fire_at` uses it; otherwise the deterministic source
/// `created_at` is retained and an active mission is forced to `Paused`. The
/// degraded fallback is recorded so an old timestamp can never silently arm an
/// immediately-due trigger.
fn mission_next_run_at(
    report: &mut MigrationReport,
    source_id: &str,
    mission: &Mission,
) -> (chrono::DateTime<chrono::Utc>, bool) {
    match mission.next_fire_at {
        Some(next_fire_at) => (next_fire_at, false),
        None => {
            report.record_loss(
                Domain::Mission,
                source_id,
                "next_fire_at",
                LossReason::Degraded,
                "mission had no next_fire_at; its deterministic created_at is retained and an \
                 active mission is imported Paused so migration cannot fire it unexpectedly"
                    .to_string(),
            );
            (mission.created_at, true)
        }
    }
}

fn record_mission_field_losses(report: &mut MigrationReport, source_id: &str, mission: &Mission) {
    if mission.current_focus.is_some()
        || !mission.approach_history.is_empty()
        || mission.success_criteria.is_some()
        || !mission.notify_channels.is_empty()
    {
        report.record_loss(
            Domain::Mission,
            source_id,
            "mission_only_fields",
            LossReason::NoTargetConcept,
            "Reborn has no durable mission entity; current_focus, approach_history, \
             success_criteria, and notify_channels have no target"
                .to_string(),
        );
    }
}

fn engine_role(role: v2_model::MessageRole) -> ImportRole {
    match role {
        v2_model::MessageRole::User => ImportRole::User,
        v2_model::MessageRole::Assistant => ImportRole::Assistant,
        v2_model::MessageRole::System | v2_model::MessageRole::ActionResult => ImportRole::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IndexedEngineThread, IndexedMission, insert_engine_thread, insert_mission,
        parse_engine_document,
    };
    use std::collections::BTreeMap;

    #[test]
    fn exact_engine_documents_are_deduplicated_by_durable_id() {
        let mission_json = serde_json::json!({
            "id": "11111111-1111-4111-8111-111111111111",
            "user_id": "alice",
            "name": "daily",
            "cadence": "Manual"
        });
        let content = mission_json.to_string();
        let (representation, mission) =
            parse_engine_document::<crate::v2_model::Mission>(&content).unwrap();
        let mut missions = BTreeMap::new();
        insert_mission(
            &mut missions,
            IndexedMission {
                source: "first".to_string(),
                owner: "alice".to_string(),
                representation: representation.clone(),
                mission: mission.clone(),
            },
        )
        .unwrap();
        insert_mission(
            &mut missions,
            IndexedMission {
                source: "second".to_string(),
                owner: "alice".to_string(),
                representation,
                mission,
            },
        )
        .unwrap();
        assert_eq!(missions.len(), 1);
    }

    #[test]
    fn divergent_engine_documents_are_rejected_by_durable_id() {
        let first = serde_json::json!({
            "id": "22222222-2222-4222-8222-222222222222",
            "title": "first"
        });
        let second = serde_json::json!({
            "id": "22222222-2222-4222-8222-222222222222",
            "title": "second"
        });
        let (first_representation, first_thread) =
            parse_engine_document::<crate::v2_model::EngineThread>(&first.to_string()).unwrap();
        let (second_representation, second_thread) =
            parse_engine_document::<crate::v2_model::EngineThread>(&second.to_string()).unwrap();
        let mut threads = BTreeMap::new();
        insert_engine_thread(
            &mut threads,
            IndexedEngineThread {
                source: "first".to_string(),
                representation: first_representation,
                thread: first_thread,
            },
        )
        .unwrap();
        let error = insert_engine_thread(
            &mut threads,
            IndexedEngineThread {
                source: "second".to_string(),
                representation: second_representation,
                thread: second_thread,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("divergent state"));
    }
}
