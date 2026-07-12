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

use std::collections::HashMap;

use ironclaw::agent::routine::{Routine, RoutineAction, Trigger};
use ironclaw_host_api::ProjectId;
use ironclaw_triggers::{TriggerRecord, TriggerSchedule, TriggerSourceKind, TriggerState};
use uuid::Uuid;

use crate::convert::threads::{ImportMessage, ImportRole, ThreadImport, write_thread};
use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;
use crate::v2_model::{self, EngineThread, Mission, MissionCadence, MissionStatus};

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    convert_routines(src, tgt, options, report).await?;
    convert_missions(src, tgt, options, report).await?;
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
    let state = if routine.enabled {
        TriggerState::Scheduled
    } else {
        TriggerState::Paused
    };
    let now = routine.next_fire_at.unwrap_or(routine.created_at);

    record_routine_field_losses(report, &source_id, &routine, is_cron);

    // A malformed source user id is a per-item loss, not a run abort.
    let Some(creator_user_id) =
        report.valid_user_id(Domain::Routine, &source_id, "user_id", &routine.user_id)
    else {
        return Ok(());
    };

    let record = TriggerRecord {
        trigger_id: ironclaw_triggers::TriggerId::new(),
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
        tgt.trigger_repo
            .upsert_trigger(record)
            .await
            .map_err(|e| MigrationError::WriteTarget {
                domain: format!("trigger for {source_id}"),
                reason: e.to_string(),
            })?;
    }
    report.stats.routines += 1;
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
) -> Result<(), MigrationError> {
    let users = src.distinct_users().await?;
    for user_id in &users {
        let docs =
            src.db
                .list_documents(user_id, None)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "memory_documents(engine)".into(),
                    reason: e.to_string(),
                })?;

        // Index engine threads by id so mission thread_history can resolve them.
        let mut engine_threads: HashMap<Uuid, EngineThread> = HashMap::new();
        let mut missions: Vec<Mission> = Vec::new();
        for doc in &docs {
            if !v2_model::is_engine_path(&doc.path) {
                continue;
            }
            if doc.path.ends_with("mission.json") {
                match serde_json::from_str::<Mission>(&doc.content) {
                    Ok(mission) => missions.push(mission),
                    Err(e) => report.record_loss(
                        Domain::Mission,
                        doc.path.clone(),
                        "*",
                        LossReason::Unparseable,
                        format!("could not parse mission.json: {e}"),
                    ),
                }
            } else if doc.path.contains("/threads/") && doc.path.ends_with(".json") {
                match serde_json::from_str::<EngineThread>(&doc.content) {
                    Ok(thread) => {
                        engine_threads.insert(thread.id, thread);
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

        // Threads referenced by a mission's `thread_history`; anything parsed but
        // never referenced has no Reborn owner to migrate it under.
        let referenced: std::collections::HashSet<Uuid> = missions
            .iter()
            .flat_map(|m| m.thread_history.iter().copied())
            .collect();

        for mission in &missions {
            convert_mission(tgt, options, report, user_id, mission, &engine_threads).await?;
        }

        for id in engine_threads.keys() {
            if !referenced.contains(id) {
                report.record_loss(
                    Domain::Mission,
                    format!("thread:{id}"),
                    "*",
                    LossReason::NoTargetConcept,
                    "engine thread blob is not referenced by any mission thread_history; \
                     there is no Reborn mission owner to migrate it under"
                        .to_string(),
                );
            }
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
    engine_threads: &HashMap<Uuid, EngineThread>,
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
                    let state = match mission.status {
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
                        let next_run_at = mission_next_run_at(report, &source_id, mission);
                        let record = TriggerRecord {
                            trigger_id: ironclaw_triggers::TriggerId::new(),
                            tenant_id: tgt.tenant_id.clone(),
                            creator_user_id,
                            agent_id: Some(tgt.agent_id.clone()),
                            project_id: Option::<ProjectId>::None,
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
                            tgt.trigger_repo.upsert_trigger(record).await.map_err(|e| {
                                MigrationError::WriteTarget {
                                    domain: format!("trigger for {source_id}"),
                                    reason: e.to_string(),
                                }
                            })?;
                        }
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

    // Migrate the mission's threads under ThreadScope.mission_id.
    for tid in &mission.thread_history {
        let Some(thread) = engine_threads.get(tid) else {
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
        let import = ThreadImport {
            thread_id: thread.id,
            owner_user: owner.clone(),
            title: thread.title.clone().or_else(|| Some(mission.name.clone())),
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

/// The trigger's `next_run_at` for a migrated mission. A mission with an
/// explicit `next_fire_at` uses it; otherwise the fallback is the **migration
/// time**, not `mission.created_at` — the latter can be the `epoch_fallback`
/// synthesized when a drifted blob omits `created_at`, which would produce a
/// `1970`-dated, immediately-due trigger. The synthesized fallback is recorded
/// as a `Degraded` loss so it is never silent.
fn mission_next_run_at(
    report: &mut MigrationReport,
    source_id: &str,
    mission: &Mission,
) -> chrono::DateTime<chrono::Utc> {
    match mission.next_fire_at {
        Some(next_fire_at) => next_fire_at,
        None => {
            report.record_loss(
                Domain::Mission,
                source_id,
                "next_fire_at",
                LossReason::Degraded,
                "mission had no next_fire_at; the trigger's next run was synthesized to the \
                 migration time (mission created_at may be an epoch fallback, which would make \
                 the trigger immediately due)"
                    .to_string(),
            );
            chrono::Utc::now()
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
