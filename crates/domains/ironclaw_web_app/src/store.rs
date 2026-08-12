//! Durable push-subscription storage: one JSON document per (tenant, user)
//! on the scoped filesystem plane, mutated exclusively through the shared
//! bounded CAS path (`.claude/rules/database.md`). Backend selection stays
//! with composition; this store never branches on a backend.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::resource::ResourceScope;
use serde::{Deserialize, Serialize};

use crate::error::WebAppError;
use crate::subscription::{MAX_SUBSCRIPTIONS_PER_USER, PushEndpoint, PushSubscriptionRecord};

/// One document per (tenant, user): the scoped-filesystem mount view already
/// prefixes `/tenants/<tenant>/users/<user>`, so the alias-relative path is
/// constant. Composition grants the `/web-push` alias in the per-user mount
/// view — both the alias and this path keep the pre-rename spelling because
/// the alias resolves to a physical per-user subpath, and moving it would
/// orphan every persisted browser enrollment.
const SUBSCRIPTIONS_DOCUMENT: &str = "/web-push/subscriptions.json";
const DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Outcome of an upsert: whether the endpoint enrolled fresh or refreshed an
/// existing enrollment (same endpoint, e.g. rotated keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushSubscriptionUpsertOutcome {
    Enrolled,
    Refreshed,
}

/// Typed store contract for push subscriptions.
#[async_trait]
pub trait WebAppSubscriptionStore: Send + Sync {
    /// Insert or refresh (by endpoint) one browser enrollment.
    async fn upsert_subscription(
        &self,
        scope: &ResourceScope,
        record: PushSubscriptionRecord,
    ) -> Result<PushSubscriptionUpsertOutcome, WebAppError>;

    /// Remove one enrollment by endpoint. Returns whether it existed.
    async fn remove_subscription(
        &self,
        scope: &ResourceScope,
        endpoint: &PushEndpoint,
    ) -> Result<bool, WebAppError>;

    /// All current enrollments for the scope's user, newest first.
    async fn list_subscriptions(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<PushSubscriptionRecord>, WebAppError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SubscriptionDocument {
    schema_version: u32,
    tenant_id: String,
    user_id: String,
    subscriptions: Vec<PushSubscriptionRecord>,
}

impl SubscriptionDocument {
    fn empty(scope: &ResourceScope) -> Self {
        Self {
            schema_version: DOCUMENT_SCHEMA_VERSION,
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            subscriptions: Vec::new(),
        }
    }

    /// Defense in depth beyond path scoping: a document whose recorded owner
    /// disagrees with the requesting scope is corrupt or misrouted; refuse
    /// rather than serve another user's enrollment set.
    fn validate_owner(&self, scope: &ResourceScope) -> Result<(), WebAppError> {
        if self.tenant_id != scope.tenant_id.to_string()
            || self.user_id != scope.user_id.to_string()
        {
            return Err(WebAppError::InvalidScope {
                reason: "stored subscription document does not belong to the requested scope"
                    .to_string(),
            });
        }
        Ok(())
    }
}

/// Filesystem-plane implementation over a [`ScopedFilesystem`].
pub struct FilesystemWebAppSubscriptionStore<F>
where
    F: RootFilesystem + ?Sized,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemWebAppSubscriptionStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

fn document_path() -> Result<ScopedPath, WebAppError> {
    ScopedPath::new(SUBSCRIPTIONS_DOCUMENT)
        .map_err(|error| WebAppError::store(format!("subscription path rejected: {error}")))
}

fn decode_document(bytes: &[u8]) -> Result<SubscriptionDocument, WebAppError> {
    serde_json::from_slice(bytes).map_err(|error| {
        WebAppError::store(format!("subscription document decode failed: {error}"))
    })
}

fn encode_document(document: &SubscriptionDocument) -> Result<Entry, WebAppError> {
    let bytes = serde_json::to_vec(document).map_err(|error| {
        WebAppError::store(format!("subscription document encode failed: {error}"))
    })?;
    Ok(Entry::bytes(bytes).with_content_type(ContentType::json()))
}

fn map_cas_error(error: CasUpdateError<WebAppError>) -> WebAppError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        other => WebAppError::store(format!("subscription document update failed: {other}")),
    }
}

#[async_trait]
impl<F> WebAppSubscriptionStore for FilesystemWebAppSubscriptionStore<F>
where
    F: RootFilesystem + ?Sized,
{
    async fn upsert_subscription(
        &self,
        scope: &ResourceScope,
        record: PushSubscriptionRecord,
    ) -> Result<PushSubscriptionUpsertOutcome, WebAppError> {
        let path = document_path()?;
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_document,
            encode_document,
            move |current: Option<SubscriptionDocument>| {
                let record = record.clone();
                let scope = scope.clone();
                async move {
                    let mut document =
                        current.unwrap_or_else(|| SubscriptionDocument::empty(&scope));
                    document.validate_owner(&scope)?;
                    if let Some(existing) = document
                        .subscriptions
                        .iter_mut()
                        .find(|existing| existing.endpoint == record.endpoint)
                    {
                        if existing.keys == record.keys && existing.user_agent == record.user_agent
                        {
                            return Ok(CasApply::no_op(
                                document,
                                PushSubscriptionUpsertOutcome::Refreshed,
                            ));
                        }
                        existing.keys = record.keys.clone();
                        existing.user_agent = record.user_agent.clone();
                        return Ok(CasApply::new(
                            document,
                            PushSubscriptionUpsertOutcome::Refreshed,
                        ));
                    }
                    if document.subscriptions.len() >= MAX_SUBSCRIPTIONS_PER_USER {
                        return Err(WebAppError::SubscriptionLimitReached {
                            limit: MAX_SUBSCRIPTIONS_PER_USER,
                        });
                    }
                    // Newest first so the settings UI lists recent browsers on top.
                    document.subscriptions.insert(0, record);
                    Ok(CasApply::new(
                        document,
                        PushSubscriptionUpsertOutcome::Enrolled,
                    ))
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn remove_subscription(
        &self,
        scope: &ResourceScope,
        endpoint: &PushEndpoint,
    ) -> Result<bool, WebAppError> {
        let path = document_path()?;
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_document,
            encode_document,
            move |current: Option<SubscriptionDocument>| {
                let endpoint = endpoint.clone();
                let scope = scope.clone();
                async move {
                    let Some(mut document) = current else {
                        return Ok(CasApply::no_op(SubscriptionDocument::empty(&scope), false));
                    };
                    document.validate_owner(&scope)?;
                    let before = document.subscriptions.len();
                    document
                        .subscriptions
                        .retain(|existing| existing.endpoint != endpoint);
                    if document.subscriptions.len() == before {
                        return Ok(CasApply::no_op(document, false));
                    }
                    Ok(CasApply::new(document, true))
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn list_subscriptions(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<PushSubscriptionRecord>, WebAppError> {
        let path = document_path()?;
        let entry = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(WebAppError::store)?;
        let Some(entry) = entry else {
            return Ok(Vec::new());
        };
        let document = decode_document(&entry.entry.body)?;
        document.validate_owner(scope)?;
        Ok(document.subscriptions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::PushSubscriptionKeys;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::{InvocationId, TenantId, UserId};

    fn scope(user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").expect("tenant"),
            user_id: UserId::new(user).expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn store() -> FilesystemWebAppSubscriptionStore<InMemoryBackend> {
        FilesystemWebAppSubscriptionStore::new(Arc::new(ScopedFilesystem::new(
            Arc::new(InMemoryBackend::new()),
            |scope: &ResourceScope| {
                use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
                use ironclaw_host_api::path::{MountAlias, VirtualPath};
                // Mirrors composition's persisted `/web-push` per-user
                // alias (pre-rename spelling kept — see SUBSCRIPTIONS_DOCUMENT).
                MountView::new(vec![MountGrant::new(
                    MountAlias::new("/web-push")?,
                    VirtualPath::new(format!(
                        "/tenants/{}/users/{}/web-push",
                        scope.tenant_id, scope.user_id
                    ))?,
                    MountPermissions::read_write_list_delete(),
                )])
            },
        )))
    }

    fn keys(seed: u8) -> PushSubscriptionKeys {
        let mut point = vec![0x04u8];
        point.extend_from_slice(&[seed; 64]);
        PushSubscriptionKeys::new(
            URL_SAFE_NO_PAD.encode(point),
            URL_SAFE_NO_PAD.encode([seed; 16]),
        )
        .expect("keys")
    }

    fn record(token: &str, seed: u8) -> PushSubscriptionRecord {
        PushSubscriptionRecord::new(
            PushEndpoint::new(format!("https://push.alpha.example/send/{token}"))
                .expect("endpoint"),
            keys(seed),
            Some("Browser".to_string()),
            "2026-08-08T00:00:00Z",
        )
    }

    #[tokio::test]
    async fn enroll_refresh_list_and_remove_round_trip() {
        let store = store();
        let scope = scope("user1");

        let outcome = store
            .upsert_subscription(&scope, record("alpha", 1))
            .await
            .expect("enroll");
        assert_eq!(outcome, PushSubscriptionUpsertOutcome::Enrolled);

        // Same endpoint, rotated keys → refresh, not duplicate.
        let outcome = store
            .upsert_subscription(&scope, record("alpha", 2))
            .await
            .expect("refresh");
        assert_eq!(outcome, PushSubscriptionUpsertOutcome::Refreshed);

        store
            .upsert_subscription(&scope, record("beta", 3))
            .await
            .expect("second browser");

        let listed = store.list_subscriptions(&scope).await.expect("list");
        assert_eq!(listed.len(), 2);
        assert!(
            listed[0].endpoint.as_str().ends_with("beta"),
            "newest first"
        );

        let removed = store
            .remove_subscription(&scope, &listed[1].endpoint)
            .await
            .expect("remove");
        assert!(removed);
        let listed = store.list_subscriptions(&scope).await.expect("list");
        assert_eq!(listed.len(), 1);

        let removed_again = store
            .remove_subscription(
                &scope,
                &PushEndpoint::new("https://push.alpha.example/send/alpha").expect("endpoint"),
            )
            .await
            .expect("remove absent");
        assert!(!removed_again);
    }

    #[tokio::test]
    async fn scopes_are_isolated_per_user() {
        let store = store();
        let first = scope("user1");
        let second = scope("user2");
        store
            .upsert_subscription(&first, record("alpha", 1))
            .await
            .expect("enroll");
        assert!(
            store
                .list_subscriptions(&second)
                .await
                .expect("list")
                .is_empty(),
            "another user's scope must not see the enrollment"
        );
    }

    #[tokio::test]
    async fn the_per_user_cap_fails_closed() {
        let store = store();
        let scope = scope("user1");
        for index in 0..MAX_SUBSCRIPTIONS_PER_USER {
            store
                .upsert_subscription(&scope, record(&format!("t{index}"), index as u8))
                .await
                .expect("enroll under cap");
        }
        let over_cap = store
            .upsert_subscription(&scope, record("overflow", 99))
            .await;
        assert!(matches!(
            over_cap,
            Err(WebAppError::SubscriptionLimitReached { .. })
        ));
    }

    /// Two browsers enrolling at once against the same (empty) document must
    /// BOTH land: `cas_update` re-reads and re-applies on conflict, so the
    /// loser of the first commit retries against the winner's document rather
    /// than clobbering it. `.claude/rules/database.md` requires conflict/retry
    /// behavior to be proven at the public domain-operation seam for new
    /// persistence — drive it through `upsert_subscription`, not `cas_update`.
    #[tokio::test]
    async fn concurrent_enrollments_from_two_browsers_both_persist() {
        let store = Arc::new(store());
        let scope = scope("user1");

        let first = {
            let store = Arc::clone(&store);
            let scope = scope.clone();
            tokio::spawn(async move { store.upsert_subscription(&scope, record("alpha", 1)).await })
        };
        let second = {
            let store = Arc::clone(&store);
            let scope = scope.clone();
            tokio::spawn(async move { store.upsert_subscription(&scope, record("beta", 2)).await })
        };
        first.await.expect("join alpha").expect("enroll alpha");
        second.await.expect("join beta").expect("enroll beta");

        let listed = store.list_subscriptions(&scope).await.expect("list");
        assert_eq!(
            listed.len(),
            2,
            "a lost enrollment means the CAS retry clobbered instead of re-applying"
        );
        let endpoints: std::collections::BTreeSet<&str> = listed
            .iter()
            .map(|record| record.endpoint.as_str())
            .collect();
        assert!(endpoints.iter().any(|endpoint| endpoint.ends_with("alpha")));
        assert!(endpoints.iter().any(|endpoint| endpoint.ends_with("beta")));
    }
}
