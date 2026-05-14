//! libSQL-backed [`ConversationBindingService`] implementation.
//!
//! Looks up existing bindings in the `product_bindings` table; on miss, mints a
//! new thread via the provided [`SessionThreadService`] and persists the
//! resulting `(adapter, installation, conversation, actor) -> (tenant, user,
//! thread, agent_id?, project_id?)` mapping.
//!
//! Schema lives in `src/db/libsql_migrations.rs` migration V26.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::{EnsureThreadRequest, SessionThreadService, ThreadScope};

use crate::error::{libsql_error, transient};
use crate::identifiers::derive_user_id;

#[derive(Clone)]
pub struct LibSqlConversationBindingService {
    db: Arc<::libsql::Database>,
    thread_service: Arc<dyn SessionThreadService>,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
}

impl LibSqlConversationBindingService {
    pub fn new(
        db: Arc<::libsql::Database>,
        thread_service: Arc<dyn SessionThreadService>,
        default_tenant_id: TenantId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            db,
            thread_service,
            default_tenant_id,
            default_agent_id,
        }
    }

    async fn connect(&self) -> Result<::libsql::Connection, ProductWorkflowError> {
        self.db.connect().map_err(libsql_error)
    }
}

#[async_trait]
impl ConversationBindingService for LibSqlConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let conn = self.connect().await?;
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        let actor_kind = request.external_actor_ref.kind();
        let actor_id = request.external_actor_ref.id();

        // 1. Look up existing binding.
        let mut rows = conn
            .query(
                "SELECT tenant_id, user_id, thread_id, agent_id, project_id \
                 FROM product_bindings \
                 WHERE adapter_id = ?1 \
                   AND installation_id = ?2 \
                   AND external_conversation_fingerprint = ?3 \
                   AND external_actor_kind = ?4 \
                   AND external_actor_id = ?5",
                ::libsql::params![
                    request.adapter_id.as_str(),
                    request.installation_id.as_str(),
                    conversation_fingerprint.as_str(),
                    actor_kind,
                    actor_id,
                ],
            )
            .await
            .map_err(libsql_error)?;

        if let Some(row) = rows.next().await.map_err(libsql_error)? {
            let tenant_id_str: String = row.get(0).map_err(libsql_error)?;
            let user_id_str: String = row.get(1).map_err(libsql_error)?;
            let thread_id_str: String = row.get(2).map_err(libsql_error)?;
            let agent_id_str: Option<String> = row.get(3).map_err(libsql_error)?;
            let project_id_str: Option<String> = row.get(4).map_err(libsql_error)?;

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

        // 2. Miss — derive canonical identifiers and create a thread.
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

        // 3. Persist the new binding.
        conn.execute(
            "INSERT INTO product_bindings \
             (adapter_id, installation_id, external_conversation_fingerprint, \
              external_actor_kind, external_actor_id, \
              tenant_id, user_id, thread_id, agent_id, project_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, \
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            ::libsql::params![
                request.adapter_id.as_str(),
                request.installation_id.as_str(),
                conversation_fingerprint.as_str(),
                actor_kind,
                actor_id,
                self.default_tenant_id.as_str(),
                user_id.as_str(),
                thread_id.as_str(),
                self.default_agent_id.as_str(),
            ],
        )
        .await
        .map_err(|e| match e {
            // UNIQUE violation means another concurrent inbound created the
            // binding between our SELECT and INSERT. Retry the lookup so we
            // return the canonical row instead of two threads for one chat.
            // libsql 0.6 surfaces the extended SQLite code 2067
            // (SQLITE_CONSTRAINT_UNIQUE), not the primary code 19; matching
            // on 19 alone silently fails to catch the concurrent case.
            ::libsql::Error::SqliteFailure(2067, _) => {
                transient("concurrent binding insert detected; retry")
            }
            other => libsql_error(other),
        })?;

        Ok(ResolvedBinding {
            tenant_id: self.default_tenant_id.clone(),
            user_id,
            thread_id,
            agent_id: Some(self.default_agent_id.clone()),
            project_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::AuthRequirement;
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
        ProtocolAuthEvidence,
    };
    use ironclaw_threads::InMemorySessionThreadService;

    /// Mirrors `src/db/libsql_migrations.rs` migration V26.
    const TEST_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS product_bindings (
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    external_conversation_fingerprint TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);
"#;

    async fn service() -> (LibSqlConversationBindingService, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("binding.db");
        let db = ::libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build db");
        let conn = db.connect().expect("connect");
        conn.execute_batch(TEST_SCHEMA).await.expect("schema");
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let tenant = TenantId::new("tenant_default").expect("tenant");
        let agent = AgentId::new("agent_default").expect("agent");
        let svc =
            LibSqlConversationBindingService::new(Arc::new(db), thread_service, tenant, agent);
        (svc, dir)
    }

    fn request(actor_id: &str, conversation_id: &str) -> ResolveBindingRequest {
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            },
            "telegram_install_default",
        );
        let auth_claim = evidence.claim().expect("verified claim").clone();
        ResolveBindingRequest {
            adapter_id: ProductAdapterId::new("telegram_v2").expect("adapter"),
            installation_id: AdapterInstallationId::new("install_default").expect("install"),
            external_actor_ref: ExternalActorRef::new("user", actor_id, None::<String>)
                .expect("actor"),
            external_conversation_ref: ExternalConversationRef::new(
                None,
                conversation_id,
                None,
                None,
            )
            .expect("conv"),
            auth_claim,
        }
    }

    #[tokio::test]
    async fn first_resolve_creates_thread_and_persists_binding() {
        let (svc, _dir) = service().await;
        let binding = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("resolve");
        assert_eq!(binding.tenant_id.as_str(), "tenant_default");
        assert_eq!(
            binding.agent_id.as_ref().map(|a| a.as_str()),
            Some("agent_default")
        );
        assert!(binding.user_id.as_str().contains("12345"));
    }

    #[tokio::test]
    async fn repeated_resolve_returns_same_binding() {
        let (svc, _dir) = service().await;
        let first = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("first");
        let second = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("second");
        assert_eq!(first.user_id.as_str(), second.user_id.as_str());
        assert_eq!(first.thread_id.as_str(), second.thread_id.as_str());
    }

    #[tokio::test]
    async fn different_actor_in_same_conversation_gets_different_binding() {
        let (svc, _dir) = service().await;
        let alice = svc
            .resolve_binding(request("alice", "shared_chat"))
            .await
            .expect("alice");
        let bob = svc
            .resolve_binding(request("bob", "shared_chat"))
            .await
            .expect("bob");
        assert_ne!(alice.user_id.as_str(), bob.user_id.as_str());
        assert_ne!(alice.thread_id.as_str(), bob.thread_id.as_str());
    }
}
