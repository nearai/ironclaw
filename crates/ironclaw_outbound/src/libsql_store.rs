use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionCursor;
use ironclaw_turns::TurnScope;

use crate::db::{
    DeliveryRowColumns, LIBSQL_SCHEMA, SubscriptionRowColumns, db_error, delivery_identity_payload,
    failure_kind_key, from_json, projection_agent_db_value, require_one_affected,
    scope_agent_db_value, scope_project_db_value, subscription_identity_payload, to_json,
    validate_delivery_attempt_row, validate_delivery_row, validate_policy_row,
    validate_subscription_row,
};
use crate::validation::{
    validate_advance_request, validate_delivery_attempt, validate_policy,
    validate_subscription_record, validate_subscription_request,
};
use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundDeliveryId, OutboundError, OutboundStateStore, ProjectionSubscriptionId,
    ProjectionSubscriptionRecord, ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

#[cfg(feature = "libsql")]
pub struct LibSqlOutboundStateStore {
    db: Arc<::libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlOutboundStateStore {
    pub fn new(db: Arc<::libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), OutboundError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_SCHEMA).await.map_err(db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<::libsql::Connection, OutboundError> {
        let conn = self.db.connect().map_err(db_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl OutboundStateStore for LibSqlOutboundStateStore {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO reborn_outbound_notification_policies \
             (tenant_id, thread_id, agent_id, project_id, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(tenant_id, thread_id, agent_id, project_id) DO UPDATE SET \
             payload = excluded.payload",
            ::libsql::params![
                policy.scope.tenant_id.as_str(),
                policy.scope.thread_id.as_str(),
                scope_agent_db_value(&policy.scope),
                scope_project_db_value(&policy.scope),
                to_json(&policy)?,
            ],
        )
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT agent_id, project_id, payload FROM reborn_outbound_notification_policies \
                 WHERE tenant_id = ?1 AND thread_id = ?2 AND agent_id = ?3 AND project_id = ?4",
                ::libsql::params![
                    scope.tenant_id.as_str(),
                    scope.thread_id.as_str(),
                    scope_agent_db_value(&scope),
                    scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(ThreadNotificationPolicy::default_for_scope(scope));
        };
        let payload: String = row.get(2).map_err(db_error)?;
        let policy = validate_policy_row(from_json::<ThreadNotificationPolicy>(&payload)?, &scope)?;
        Ok(policy)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = conn
            .execute(
                "INSERT INTO reborn_outbound_projection_subscriptions \
                 (subscription_id, tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(subscription_id) DO UPDATE SET \
                 cursor_runtime = excluded.cursor_runtime, payload = excluded.payload \
                 WHERE reborn_outbound_projection_subscriptions.identity_payload = excluded.identity_payload",
                ::libsql::params![
                    record.subscription_id.as_str(),
                    record.scope.stream.tenant_id.as_str(),
                    record.actor.user_id.as_str(),
                    projection_agent_db_value(&record.scope),
                    record.thread_id.as_str(),
                    record.cursor.as_ref().map(|cursor| cursor.runtime.as_u64() as i64),
                    identity_payload,
                    to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        self.run_migrations().await?;
        let Some(record) = self.load_subscription(&request.subscription_id).await? else {
            return Ok(None);
        };
        validate_subscription_request(&record, &request)?;
        Ok(record.cursor)
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut record) = self.load_subscription(&request.subscription_id).await? else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(&record, &request)?;
        record.cursor = Some(request.cursor);
        let conn = self.connect().await?;
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = conn
            .execute(
                "UPDATE reborn_outbound_projection_subscriptions \
                 SET cursor_runtime = ?3, payload = ?4 WHERE subscription_id = ?1 AND identity_payload = ?2",
                ::libsql::params![
                    record.subscription_id.as_str(),
                    identity_payload,
                    record
                        .cursor
                        .as_ref()
                        .map(|cursor| cursor.runtime.as_u64() as i64),
                    to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = conn
            .execute(
                "INSERT INTO reborn_outbound_delivery_attempts \
                 (delivery_id, tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, attempted_at, status_updated_at, failure_kind, identity_payload, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12) \
                 ON CONFLICT(delivery_id) DO UPDATE SET \
                 delivery_id = reborn_outbound_delivery_attempts.delivery_id \
                 WHERE reborn_outbound_delivery_attempts.identity_payload = excluded.identity_payload",
                ::libsql::params![
                    attempt.delivery_id.to_string(),
                    attempt.scope.tenant_id.as_str(),
                    attempt.scope.thread_id.as_str(),
                    scope_agent_db_value(&attempt.scope),
                    scope_project_db_value(&attempt.scope),
                    attempt.candidate.target.as_str(),
                    attempt.candidate.kind.as_str(),
                    attempt.status.as_str(),
                    attempt.attempted_at.to_rfc3339(),
                    attempt.failure_kind.map(failure_kind_key),
                    identity_payload,
                    to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut attempt) = self.load_delivery(request.delivery_id).await? else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        let conn = self.connect().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = conn
            .execute(
                "UPDATE reborn_outbound_delivery_attempts \
                 SET status = ?7, status_updated_at = ?8, failure_kind = ?9, payload = ?10 \
                 WHERE delivery_id = ?1 AND tenant_id = ?2 AND thread_id = ?3 AND agent_id = ?4 AND project_id = ?5 AND identity_payload = ?6",
                ::libsql::params![
                    request.delivery_id.to_string(),
                    request.scope.tenant_id.as_str(),
                    request.scope.thread_id.as_str(),
                    scope_agent_db_value(&request.scope),
                    scope_project_db_value(&request.scope),
                    identity_payload,
                    request.status.as_str(),
                    request.updated_at.to_rfc3339(),
                    request.failure_kind.map(failure_kind_key),
                    to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT payload FROM reborn_outbound_delivery_attempts \
                 WHERE tenant_id = ?1 AND thread_id = ?2 AND agent_id = ?3 AND project_id = ?4 \
                 ORDER BY attempted_at, delivery_id",
                ::libsql::params![
                    scope.tenant_id.as_str(),
                    scope.thread_id.as_str(),
                    scope_agent_db_value(&scope),
                    scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let mut deliveries = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            let payload: String = row.get(0).map_err(db_error)?;
            let attempt = validate_delivery_attempt_row(
                from_json::<OutboundDeliveryAttempt>(&payload)?,
                &scope,
            )?;
            deliveries.push(attempt);
        }
        Ok(deliveries)
    }
}

#[cfg(feature = "libsql")]
impl LibSqlOutboundStateStore {
    async fn load_subscription(
        &self,
        subscription_id: &ProjectionSubscriptionId,
    ) -> Result<Option<ProjectionSubscriptionRecord>, OutboundError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload \
                 FROM reborn_outbound_projection_subscriptions WHERE subscription_id = ?1",
                ::libsql::params![subscription_id.as_str()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0).map_err(db_error)?;
        let user_id: String = row.get(1).map_err(db_error)?;
        let agent_id: String = row.get(2).map_err(db_error)?;
        let thread_id: String = row.get(3).map_err(db_error)?;
        let cursor_runtime: Option<i64> = row.get(4).map_err(db_error)?;
        let identity_payload: String = row.get(5).map_err(db_error)?;
        let payload: String = row.get(6).map_err(db_error)?;
        let record = validate_subscription_row(
            from_json::<ProjectionSubscriptionRecord>(&payload)?,
            subscription_id,
            SubscriptionRowColumns {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: &agent_id,
                thread_id: &thread_id,
                cursor_runtime,
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(record))
    }

    async fn load_delivery(
        &self,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<OutboundDeliveryAttempt>, OutboundError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, failure_kind, identity_payload, payload \
                 FROM reborn_outbound_delivery_attempts WHERE delivery_id = ?1",
                ::libsql::params![delivery_id.to_string()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0).map_err(db_error)?;
        let thread_id: String = row.get(1).map_err(db_error)?;
        let agent_id: String = row.get(2).map_err(db_error)?;
        let project_id: String = row.get(3).map_err(db_error)?;
        let target_ref: String = row.get(4).map_err(db_error)?;
        let kind: String = row.get(5).map_err(db_error)?;
        let status: String = row.get(6).map_err(db_error)?;
        let failure_kind: Option<String> = row.get(7).map_err(db_error)?;
        let identity_payload: String = row.get(8).map_err(db_error)?;
        let payload: String = row.get(9).map_err(db_error)?;
        let attempt = validate_delivery_row(
            from_json::<OutboundDeliveryAttempt>(&payload)?,
            delivery_id,
            DeliveryRowColumns {
                tenant_id: &tenant_id,
                thread_id: &thread_id,
                agent_id: &agent_id,
                project_id: &project_id,
                target_ref: &target_ref,
                kind: &kind,
                status: &status,
                failure_kind: failure_kind.as_deref(),
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(attempt))
    }
}
