//! Operator repair tooling for local-dev trigger-fire access (#4992).
//!
//! A local-dev Reborn trigger can be stranded when its persisted
//! `creator_user_id` has no active row in `local_reborn_access` for the
//! trigger's exact scope — e.g. the SSO identity index was dropped and the same
//! account minted a fresh `UserId`, or the trigger carries a non-default
//! agent/project scope that was never seeded. The fire-time checker now
//! self-heals an *absent* scope on the next fire, but operators still need a way
//! to inspect strands up front and to reassign triggers whose creator id is
//! truly gone.
//!
//! This module keeps the libSQL substrate handle private to composition (the
//! same boundary `open_local_trigger_access_store` honors) and exposes one
//! high-level API the CLI calls.

use std::path::Path;
use std::sync::Arc;

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn::local_trigger_access::{
    LocalAccessState, LocalTriggerAccessRole, LocalTriggerAccessSeed, LocalTriggerAccessSource,
    RebornLibSqlLocalTriggerAccessStore, RebornLocalTriggerAccessStoreError,
};
use ironclaw_triggers::{LibSqlTriggerRepository, TriggerError, TriggerRecord, TriggerRepository};

/// What the repair pass should do beyond reporting.
#[derive(Debug, Clone)]
pub enum TriggerAccessRepairAction {
    /// Report stranded triggers only; make no changes.
    Report,
    /// Seed exact-scope active access for each stranded creator. Idempotent and
    /// non-reactivating: a deliberately revoked (inactive) row is left as-is.
    Reseed,
    /// Reassign each stranded trigger to an explicit user id and seed access.
    Reassign(UserId),
    /// Reassign each stranded trigger to the single active SSO-admitted owner
    /// and seed access. Fails if there is not exactly one such owner.
    ReassignToCurrentSsoOwner,
}

/// One trigger whose creator lacks active local access for its exact scope.
#[derive(Debug, Clone)]
pub struct StrandedTrigger {
    pub trigger_id: String,
    pub name: String,
    pub creator_user_id: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    /// Whether the scope has no row at all (`Absent`) or a deliberately
    /// deactivated row (`Revoked`). `--reseed` only helps `Absent`.
    pub access_state: LocalAccessState,
    pub trigger_state: String,
}

/// Outcome of a repair pass.
#[derive(Debug, Clone)]
pub struct TriggerAccessRepairReport {
    pub total_triggers: usize,
    pub stranded: Vec<StrandedTrigger>,
    pub reseeded: usize,
    pub reassigned: usize,
    pub reassigned_to: Option<String>,
}

/// Failure modes of the repair pass.
#[derive(Debug, thiserror::Error)]
pub enum TriggerAccessRepairError {
    #[error("trigger repository backend failure: {0}")]
    Trigger(String),
    #[error("local trigger access store failure: {0}")]
    Access(String),
    #[error(
        "cannot resolve a single current SSO owner: {found} active SSO-admitted owners. \
         Pass an explicit target user id instead."
    )]
    AmbiguousSsoOwner { found: usize },
    #[error(
        "no active SSO-admitted owner found to reassign to. \
         Log in via SSO first, or pass an explicit target user id."
    )]
    NoSsoOwner,
}

impl From<TriggerError> for TriggerAccessRepairError {
    fn from(error: TriggerError) -> Self {
        Self::Trigger(error.to_string())
    }
}

impl From<RebornLocalTriggerAccessStoreError> for TriggerAccessRepairError {
    fn from(error: RebornLocalTriggerAccessStoreError) -> Self {
        Self::Access(error.to_string())
    }
}

/// Run a repair pass over the local-dev substrate DB at `path` for `tenant_id`.
///
/// Opens the trigger repository and the local access store on one shared libSQL
/// handle (both ride `reborn-local-dev.db`), enumerates the tenant's triggers,
/// collects those whose creator lacks active access for the trigger's exact
/// scope, and applies `action`. Always returns the stranded set so callers can
/// report regardless of the action taken.
pub async fn repair_local_trigger_access(
    path: &Path,
    tenant_id: &TenantId,
    action: TriggerAccessRepairAction,
) -> Result<TriggerAccessRepairReport, TriggerAccessRepairError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            TriggerAccessRepairError::Access(format!("create substrate dir: {err}"))
        })?;
    }
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|err| TriggerAccessRepairError::Access(format!("open substrate db: {err}")))?,
    );
    let repository = LibSqlTriggerRepository::new(Arc::clone(&db));
    repository.run_migrations().await?;
    let access = RebornLibSqlLocalTriggerAccessStore::open(Arc::clone(&db)).await?;

    let triggers = repository.list_triggers(tenant_id.clone()).await?;
    let total_triggers = triggers.len();

    let mut stranded_records: Vec<(TriggerRecord, LocalAccessState)> = Vec::new();
    for trigger in triggers {
        let state = access
            .local_access_state(
                tenant_id,
                &trigger.creator_user_id,
                trigger.agent_id.as_ref(),
                trigger.project_id.as_ref(),
            )
            .await?;
        if state != LocalAccessState::Active {
            stranded_records.push((trigger, state));
        }
    }

    let stranded: Vec<StrandedTrigger> = stranded_records
        .iter()
        .map(|(trigger, state)| StrandedTrigger {
            trigger_id: trigger.trigger_id.to_string(),
            name: trigger.name.clone(),
            creator_user_id: trigger.creator_user_id.as_str().to_string(),
            agent_id: trigger.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: trigger
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string()),
            access_state: *state,
            trigger_state: format!("{:?}", trigger.state),
        })
        .collect();

    let mut report = TriggerAccessRepairReport {
        total_triggers,
        stranded,
        reseeded: 0,
        reassigned: 0,
        reassigned_to: None,
    };

    match action {
        TriggerAccessRepairAction::Report => {}
        TriggerAccessRepairAction::Reseed => {
            for (trigger, _) in &stranded_records {
                seed_creator_access(&access, tenant_id, trigger, &trigger.creator_user_id).await?;
                report.reseeded += 1;
            }
        }
        TriggerAccessRepairAction::Reassign(target) => {
            apply_reassign(&repository, &access, tenant_id, &stranded_records, &target).await?;
            report.reassigned = stranded_records.len();
            report.reassigned_to = Some(target.as_str().to_string());
        }
        TriggerAccessRepairAction::ReassignToCurrentSsoOwner => {
            let target = resolve_current_sso_owner(&access, tenant_id).await?;
            apply_reassign(&repository, &access, tenant_id, &stranded_records, &target).await?;
            report.reassigned = stranded_records.len();
            report.reassigned_to = Some(target.as_str().to_string());
        }
    }

    Ok(report)
}

async fn resolve_current_sso_owner(
    access: &RebornLibSqlLocalTriggerAccessStore,
    tenant_id: &TenantId,
) -> Result<UserId, TriggerAccessRepairError> {
    let owners = access
        .list_active_user_ids_for_source(tenant_id, LocalTriggerAccessSource::LocalDevSsoBootstrap)
        .await?;
    match owners.len() {
        0 => Err(TriggerAccessRepairError::NoSsoOwner),
        1 => Ok(owners.into_iter().next().expect("len checked")),
        found => Err(TriggerAccessRepairError::AmbiguousSsoOwner { found }),
    }
}

async fn apply_reassign(
    repository: &LibSqlTriggerRepository,
    access: &RebornLibSqlLocalTriggerAccessStore,
    tenant_id: &TenantId,
    stranded_records: &[(TriggerRecord, LocalAccessState)],
    target: &UserId,
) -> Result<(), TriggerAccessRepairError> {
    for (trigger, _) in stranded_records {
        let mut updated = trigger.clone();
        updated.creator_user_id = target.clone();
        repository.upsert_trigger(updated).await?;
        seed_creator_access(access, tenant_id, trigger, target).await?;
    }
    Ok(())
}

async fn seed_creator_access(
    access: &RebornLibSqlLocalTriggerAccessStore,
    tenant_id: &TenantId,
    trigger: &TriggerRecord,
    user_id: &UserId,
) -> Result<(), TriggerAccessRepairError> {
    access
        .seed_local_access(LocalTriggerAccessSeed {
            tenant_id,
            user_id,
            agent_id: trigger.agent_id.as_ref(),
            project_id: trigger.project_id.as_ref(),
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevTriggerCreateBootstrap,
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId};
    use ironclaw_triggers::{
        TriggerCompletionPolicy, TriggerId, TriggerSchedule, TriggerSourceKind, TriggerState,
    };
    use std::path::PathBuf;

    fn ts(seconds: i64) -> ironclaw_host_api::Timestamp {
        use chrono::TimeZone;
        chrono::Utc
            .timestamp_opt(seconds, 0)
            .single()
            .expect("valid timestamp")
    }

    fn stranded_record(trigger_id: &str, tenant: &TenantId, creator: &UserId) -> TriggerRecord {
        TriggerRecord {
            trigger_id: TriggerId::parse(trigger_id).expect("ulid"),
            tenant_id: tenant.clone(),
            creator_user_id: creator.clone(),
            agent_id: Some(AgentId::new("repair-agent").expect("agent")),
            project_id: Some(ProjectId::new("repair-project").expect("project")),
            name: "repair test".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::cron("0 8 * * *").expect("cron"),
            completion_policy: TriggerCompletionPolicy::Recurring,
            prompt: "do the thing".to_string(),
            state: TriggerState::Scheduled,
            next_run_at: ts(1_704_067_200),
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: ts(1_704_067_200),
        }
    }

    async fn seed_trigger(path: &std::path::Path, record: TriggerRecord) {
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("build db"),
        );
        let repo = LibSqlTriggerRepository::new(db);
        repo.run_migrations().await.expect("migrate");
        repo.upsert_trigger(record).await.expect("seed trigger");
    }

    fn db_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("reborn-local-dev.db")
    }

    #[tokio::test]
    async fn report_lists_stranded_then_reseed_grants_access() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = db_path(&dir);
        let tenant = TenantId::new("reborn-cli").expect("tenant");
        let creator = UserId::new("stranded-creator").expect("user");
        seed_trigger(
            &path,
            stranded_record("01J0000000000000000000A001", &tenant, &creator),
        )
        .await;

        let report = repair_local_trigger_access(&path, &tenant, TriggerAccessRepairAction::Report)
            .await
            .expect("report");
        assert_eq!(report.total_triggers, 1);
        assert_eq!(report.stranded.len(), 1);
        assert_eq!(report.stranded[0].access_state, LocalAccessState::Absent);
        assert_eq!(report.reseeded, 0);

        let applied =
            repair_local_trigger_access(&path, &tenant, TriggerAccessRepairAction::Reseed)
                .await
                .expect("reseed");
        assert_eq!(applied.reseeded, 1);

        // After reseed the creator now has active access — no longer stranded.
        let after = repair_local_trigger_access(&path, &tenant, TriggerAccessRepairAction::Report)
            .await
            .expect("report after reseed");
        assert!(
            after.stranded.is_empty(),
            "reseed should clear the strand for the exact scope"
        );
    }

    #[tokio::test]
    async fn reassign_rewrites_creator_and_seeds_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = db_path(&dir);
        let tenant = TenantId::new("reborn-cli").expect("tenant");
        let creator = UserId::new("gone-creator").expect("user");
        let target = UserId::new("current-owner").expect("user");
        let trigger_id = "01J0000000000000000000A002";
        seed_trigger(&path, stranded_record(trigger_id, &tenant, &creator)).await;

        let applied = repair_local_trigger_access(
            &path,
            &tenant,
            TriggerAccessRepairAction::Reassign(target.clone()),
        )
        .await
        .expect("reassign");
        assert_eq!(applied.reassigned, 1);
        assert_eq!(applied.reassigned_to.as_deref(), Some(target.as_str()));

        // The persisted trigger now belongs to the target, who has active access.
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("reopen db"),
        );
        let repo = LibSqlTriggerRepository::new(Arc::clone(&db));
        let reloaded = repo
            .get_trigger(tenant.clone(), TriggerId::parse(trigger_id).expect("ulid"))
            .await
            .expect("get trigger")
            .expect("trigger present");
        assert_eq!(reloaded.creator_user_id, target);

        let access = RebornLibSqlLocalTriggerAccessStore::open(db)
            .await
            .expect("open access");
        assert!(
            access
                .has_active_local_access(
                    &tenant,
                    &target,
                    reloaded.agent_id.as_ref(),
                    reloaded.project_id.as_ref()
                )
                .await
                .expect("check access"),
            "target must have active access for the trigger scope after reassign"
        );
    }
}
