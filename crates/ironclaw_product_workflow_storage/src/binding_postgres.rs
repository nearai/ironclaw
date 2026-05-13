//! Postgres-backed [`ConversationBindingService`] implementation.
//!
//! Schema lives in `migrations/V28__product_inbound_actions_and_bindings.sql`.

use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::{EnsureThreadRequest, SessionThreadService, ThreadScope};

use crate::error::{pool_error, postgres_error};
use crate::identifiers::derive_user_id;

#[derive(Clone)]
pub struct PostgresConversationBindingService {
    pool: Pool,
    thread_service: Arc<dyn SessionThreadService>,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
}

impl PostgresConversationBindingService {
    pub fn new(
        pool: Pool,
        thread_service: Arc<dyn SessionThreadService>,
        default_tenant_id: TenantId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            pool,
            thread_service,
            default_tenant_id,
            default_agent_id,
        }
    }
}

#[async_trait]
impl ConversationBindingService for PostgresConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let client = self.pool.get().await.map_err(pool_error)?;
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        let actor_kind = request.external_actor_ref.kind();
        let actor_id = request.external_actor_ref.id();

        let existing = client
            .query_opt(
                "SELECT tenant_id, user_id, thread_id, agent_id, project_id \
                 FROM product_bindings \
                 WHERE adapter_id = $1 \
                   AND installation_id = $2 \
                   AND external_conversation_fingerprint = $3 \
                   AND external_actor_kind = $4 \
                   AND external_actor_id = $5",
                &[
                    &request.adapter_id.as_str(),
                    &request.installation_id.as_str(),
                    &conversation_fingerprint.as_str(),
                    &actor_kind,
                    &actor_id,
                ],
            )
            .await
            .map_err(postgres_error)?;

        if let Some(row) = existing {
            let tenant_id_str: String = row.get("tenant_id");
            let user_id_str: String = row.get("user_id");
            let thread_id_str: String = row.get("thread_id");
            let agent_id_str: Option<String> = row.get("agent_id");
            let project_id_str: Option<String> = row.get("project_id");

            return Ok(ResolvedBinding {
                tenant_id: TenantId::new(tenant_id_str).map_err(|e| {
                    ProductWorkflowError::BindingResolutionFailed {
                        reason: e.to_string(),
                    }
                })?,
                user_id: UserId::new(user_id_str).map_err(|e| {
                    ProductWorkflowError::BindingResolutionFailed {
                        reason: e.to_string(),
                    }
                })?,
                thread_id: ThreadId::new(thread_id_str).map_err(|e| {
                    ProductWorkflowError::BindingResolutionFailed {
                        reason: e.to_string(),
                    }
                })?,
                agent_id: agent_id_str.map(AgentId::new).transpose().map_err(|e| {
                    ProductWorkflowError::BindingResolutionFailed {
                        reason: e.to_string(),
                    }
                })?,
                project_id: project_id_str
                    .map(ProjectId::new)
                    .transpose()
                    .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                        reason: e.to_string(),
                    })?,
            });
        }

        let user_id = derive_user_id(&request)?;
        let scope = ThreadScope {
            tenant_id: self.default_tenant_id.clone(),
            agent_id: self.default_agent_id.clone(),
            project_id: None,
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        let ensure_request = EnsureThreadRequest {
            scope,
            thread_id: None,
            created_by_actor_id: format!(
                "{}:{}",
                request.adapter_id.as_str(),
                request.installation_id.as_str()
            ),
            title: None,
            metadata_json: None,
        };
        let thread_record = self
            .thread_service
            .ensure_thread(ensure_request)
            .await
            .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                reason: format!("ensure_thread failed: {e}"),
            })?;
        let thread_id = thread_record.thread_id.clone();

        // Single-roundtrip upsert: the `ON CONFLICT ... DO UPDATE SET adapter_id
        // = EXCLUDED.adapter_id` is a "fake update" that lets us return the
        // canonical `thread_id` on conflict — when a concurrent inbound beat us
        // to the insert, we get back the row they wrote, not ours. The actual
        // column values don't change (the fake update writes the same
        // `adapter_id` value that was already there).
        let upsert_row = client
            .query_one(
                "INSERT INTO product_bindings \
                 (adapter_id, installation_id, external_conversation_fingerprint, \
                  external_actor_kind, external_actor_id, \
                  tenant_id, user_id, thread_id, agent_id, project_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL) \
                 ON CONFLICT (adapter_id, installation_id, external_conversation_fingerprint, external_actor_kind, external_actor_id) \
                 DO UPDATE SET adapter_id = EXCLUDED.adapter_id \
                 RETURNING thread_id",
                &[
                    &request.adapter_id.as_str(),
                    &request.installation_id.as_str(),
                    &conversation_fingerprint.as_str(),
                    &actor_kind,
                    &actor_id,
                    &self.default_tenant_id.as_str(),
                    &user_id.as_str(),
                    &thread_id.as_str(),
                    &self.default_agent_id.as_str(),
                ],
            )
            .await
            .map_err(postgres_error)?;
        let canonical_thread_id_str: String = upsert_row.get("thread_id");
        let canonical_thread_id = ThreadId::new(canonical_thread_id_str).map_err(|e| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            }
        })?;

        Ok(ResolvedBinding {
            tenant_id: self.default_tenant_id.clone(),
            user_id,
            thread_id: canonical_thread_id,
            agent_id: Some(self.default_agent_id.clone()),
            project_id: None,
        })
    }
}
