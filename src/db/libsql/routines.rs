//! Routine-related RoutineStore implementation for LibSqlBackend.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::{params, params_from_iter};
use uuid::Uuid;

use super::{
    LibSqlBackend, ROUTINE_COLUMNS, ROUTINE_RUN_COLUMNS, fmt_opt_ts, fmt_ts, get_i64, get_text,
    opt_text, opt_text_owned, row_to_routine_libsql, row_to_routine_run_libsql,
};
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::db::RoutineStore;
use crate::error::DatabaseError;

#[async_trait]
impl RoutineStore for LibSqlBackend {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i64;
        let max_concurrent = routine.guardrails.max_concurrent as i64;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i64);

        conn.execute(
                r#"
                INSERT INTO routines (
                    id, name, description, user_id, workspace_id, enabled,
                    trigger_type, trigger_config, action_type, action_config,
                    cooldown_secs, max_concurrent, dedup_window_secs,
                    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
                    state, next_fire_at, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18,
                    ?19, ?20, ?21, ?22
                )
                "#,
                params![
                    routine.id.to_string(),
                    routine.name.as_str(),
                    routine.description.as_str(),
                    routine.user_id.as_str(),
                    opt_text_owned(routine.workspace_id.map(|id| id.to_string())),
                    routine.enabled as i64,
                    trigger_type,
                    trigger_config.to_string(),
                    action_type,
                    action_config.to_string(),
                    cooldown_secs,
                    max_concurrent,
                    dedup_window_secs,
                    opt_text(routine.notify.channel.as_deref()),
                    opt_text(routine.notify.user.as_deref()),
                    routine.notify.on_success as i64,
                    routine.notify.on_failure as i64,
                    routine.notify.on_attention as i64,
                    routine.state.to_string(),
                    fmt_opt_ts(&routine.next_fire_at),
                    fmt_ts(&routine.created_at),
                    fmt_ts(&routine.updated_at),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {} FROM routines WHERE id = ?1", ROUTINE_COLUMNS),
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_routine_libsql(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        workspace_id: Option<Uuid>,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE user_id = ?1 AND workspace_id IS ?2 AND name = ?3",
                    ROUTINE_COLUMNS
                ),
                params![user_id, opt_text_owned(workspace_id.map(|id| id.to_string())), name],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_routine_libsql(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE user_id = ?1 AND workspace_id IS NULL ORDER BY name",
                    ROUTINE_COLUMNS
                ),
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn list_routines_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE workspace_id = ?1 ORDER BY name",
                    ROUTINE_COLUMNS
                ),
                params![workspace_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!("SELECT {} FROM routines ORDER BY name", ROUTINE_COLUMNS),
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type IN ('event', 'system_event')",
                    ROUTINE_COLUMNS
                ),
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type = 'cron' AND next_fire_at IS NOT NULL AND next_fire_at <= ?1",
                    ROUTINE_COLUMNS
                ),
                params![now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut routines = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routines.push(row_to_routine_libsql(&row)?);
        }
        Ok(routines)
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let trigger_type = routine.trigger.type_tag();
        let trigger_config = routine.trigger.to_config_json();
        let action_type = routine.action.type_tag();
        let action_config = routine.action.to_config_json();
        let cooldown_secs = routine.guardrails.cooldown.as_secs() as i64;
        let max_concurrent = routine.guardrails.max_concurrent as i64;
        let dedup_window_secs = routine.guardrails.dedup_window.map(|d| d.as_secs() as i64);
        let now = fmt_ts(&Utc::now());

        conn.execute(
            r#"
                UPDATE routines SET
                    name = ?2, description = ?3, workspace_id = ?4, enabled = ?5,
                    trigger_type = ?6, trigger_config = ?7,
                    action_type = ?8, action_config = ?9,
                    cooldown_secs = ?10, max_concurrent = ?11, dedup_window_secs = ?12,
                    notify_channel = ?13, notify_user = ?14,
                    notify_on_success = ?15, notify_on_failure = ?16, notify_on_attention = ?17,
                    state = ?18, next_fire_at = ?19,
                    updated_at = ?20
                WHERE id = ?1
                "#,
            params![
                routine.id.to_string(),
                routine.name.as_str(),
                routine.description.as_str(),
                opt_text_owned(routine.workspace_id.map(|id| id.to_string())),
                routine.enabled as i64,
                trigger_type,
                trigger_config.to_string(),
                action_type,
                action_config.to_string(),
                cooldown_secs,
                max_concurrent,
                dedup_window_secs,
                opt_text(routine.notify.channel.as_deref()),
                opt_text(routine.notify.user.as_deref()),
                routine.notify.on_success as i64,
                routine.notify.on_failure as i64,
                routine.notify.on_attention as i64,
                routine.state.to_string(),
                fmt_opt_ts(&routine.next_fire_at),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                UPDATE routines SET
                    last_run_at = ?2, next_fire_at = ?3,
                    run_count = ?4, consecutive_failures = ?5,
                    state = ?6, updated_at = ?7
                WHERE id = ?1
                "#,
            params![
                id.to_string(),
                fmt_ts(&last_run_at),
                fmt_opt_ts(&next_fire_at),
                run_count as i64,
                consecutive_failures as i64,
                state.to_string(),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let count = conn
            .execute(
                "DELETE FROM routines WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(count > 0)
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
                INSERT INTO routine_runs (
                    id, routine_id, trigger_type, trigger_detail,
                    started_at, status, job_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
            params![
                run.id.to_string(),
                run.routine_id.to_string(),
                run.trigger_type.as_str(),
                opt_text(run.trigger_detail.as_deref()),
                fmt_ts(&run.started_at),
                run.status.to_string(),
                opt_text_owned(run.job_id.map(|id| id.to_string())),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
                UPDATE routine_runs SET
                    completed_at = ?5, status = ?2,
                    result_summary = ?3, tokens_used = ?4
                WHERE id = ?1
                "#,
            params![
                id.to_string(),
                status.to_string(),
                opt_text(result_summary),
                tokens_used.map(|t| t as i64),
                now,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_runs WHERE routine_id = ?1 ORDER BY started_at DESC LIMIT ?2",
                    ROUTINE_RUN_COLUMNS
                ),
                params![routine_id.to_string(), limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut runs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            runs.push(row_to_routine_run_libsql(&row)?);
        }
        Ok(runs)
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) as cnt FROM routine_runs WHERE routine_id = ?1 AND status = 'running'",
                params![routine_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(get_i64(&row, 0)),
            None => Ok(0),
        }
    }

    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError> {
        if routine_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut counts = HashMap::new();
        let conn = self.connect().await?;

        // Query all running routines and filter in memory
        // This is simpler for libSQL than building dynamic parameter lists
        let mut rows = conn
            .query(
                "SELECT routine_id, COUNT(*) as cnt FROM routine_runs
                 WHERE status = 'running'
                 GROUP BY routine_id",
                params![],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to batch count running routines: {}", e))
            })?;

        let routine_id_set: HashSet<Uuid> = routine_ids.iter().copied().collect();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = get_text(&row, 0);
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| DatabaseError::Query(format!("Invalid routine UUID: {}", e)))?;

            // Only include if this routine ID was requested
            if routine_id_set.contains(&id) {
                let cnt: i64 = get_i64(&row, 1);
                counts.insert(id, cnt);
            }
        }

        // Ensure all requested IDs are in the map (defaults to 0 for no running runs)
        for id in routine_ids {
            counts.entry(*id).or_insert(0);
        }

        Ok(counts)
    }

    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError> {
        if routine_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self.connect().await?;
        let requested_rows = (1..=routine_ids.len())
            .map(|i| format!("(?{i})"))
            .collect::<Vec<_>>()
            .join(", ");
        let requested_ids = routine_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        let sql = format!(
            "WITH requested(routine_id) AS (VALUES {requested_rows})
             SELECT r1.routine_id, r1.status
             FROM routine_runs r1
             JOIN (
                 SELECT rr.routine_id, MAX(rr.started_at) AS max_started_at
                 FROM routine_runs rr
                 JOIN requested req ON req.routine_id = rr.routine_id
                 GROUP BY rr.routine_id
             ) latest
               ON latest.routine_id = r1.routine_id
              AND latest.max_started_at = r1.started_at"
        );
        let mut rows = conn
            .query(&sql, params_from_iter(requested_ids))
            .await
            .map_err(|e| {
                DatabaseError::Query(format!("Failed to batch get last run status: {}", e))
            })?;
        let mut statuses = HashMap::new();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = get_text(&row, 0);
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| DatabaseError::Query(format!("Invalid routine UUID: {}", e)))?;

            let status_str: String = get_text(&row, 1);
            if let std::result::Result::Ok(status) = status_str.parse::<RunStatus>() {
                statuses.insert(id, status);
            }
        }

        Ok(statuses)
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE routine_runs SET job_id = ?1 WHERE id = ?2",
            params![job_id.to_string(), run_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(uid) = user_id {
            conn.query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type = 'webhook' \
                     AND user_id = ?2 \
                     AND (json_extract(trigger_config, '$.path') = ?1 \
                     OR (json_extract(trigger_config, '$.path') IS NULL AND CAST(id AS TEXT) = ?1))",
                    ROUTINE_COLUMNS
                ),
                params![path, uid],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                &format!(
                    "SELECT {} FROM routines WHERE enabled = 1 AND trigger_type = 'webhook' \
                     AND (json_extract(trigger_config, '$.path') = ?1 \
                     OR (json_extract(trigger_config, '$.path') IS NULL AND CAST(id AS TEXT) = ?1))",
                    ROUTINE_COLUMNS
                ),
                params![path],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_routine_libsql(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM routine_runs WHERE status = 'running' AND job_id IS NOT NULL",
                    ROUTINE_RUN_COLUMNS
                ),
                params![],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut runs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            runs.push(row_to_routine_run_libsql(&row)?);
        }
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, Trigger,
    };
    use crate::db::{Database, RoutineStore};

    fn test_routine(user_id: &str, name: &str) -> Routine {
        Routine {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "test routine".to_string(),
            user_id: user_id.to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: Vec::new(),
                max_tokens: 128,
                use_tools: false,
                max_tool_rounds: 1,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn test_run(routine_id: Uuid, status: RunStatus, started_at: DateTime<Utc>) -> RoutineRun {
        RoutineRun {
            id: Uuid::new_v4(),
            routine_id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at,
            completed_at: None,
            status,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: started_at,
        }
    }

    #[tokio::test]
    async fn batch_get_last_run_status_is_scoped_to_requested_routines() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("routine-status.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let requested = test_routine("user-1", "requested");
        let other = test_routine("user-1", "other");
        backend.create_routine(&requested).await.unwrap();
        backend.create_routine(&other).await.unwrap();

        let now = Utc::now();
        backend
            .create_routine_run(&test_run(requested.id, RunStatus::Ok, now))
            .await
            .unwrap();
        backend
            .create_routine_run(&test_run(
                other.id,
                RunStatus::Failed,
                now + chrono::Duration::seconds(1),
            ))
            .await
            .unwrap();

        let statuses = backend
            .batch_get_last_run_status(&[requested.id])
            .await
            .unwrap();

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses.get(&requested.id), Some(&RunStatus::Ok));
        assert!(!statuses.contains_key(&other.id));
    }
}
