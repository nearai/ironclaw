//! Host-owned per-user delivery registrations (channel-adapter contract §8).
//!
//! A push subscription — endpoint plus key material, per user, revocable,
//! listed in settings — *is* a per-user delivery registration, and it used to
//! live behind three adapter methods. The consequence was not just surface
//! area: the host could not answer "is this user set up?", so there was no
//! guardrail before a delivery and the send simply failed inside the vendor
//! path. The records are host-owned now, which is what lets the coordinator
//! resolve zero registrations to a "no target" outcome before any adapter
//! call.
//!
//! **What generic code knows, and what it deliberately does not.** It knows
//! the `endpoint`, because the one security-critical check happens before
//! storage and is generic: the endpoint must target a host the channel
//! declares in `[[channel.egress]]`. Without it, enrollment is an SSRF
//! primitive that makes the host POST to an attacker's URL. Everything else
//! is an opaque, size-bounded `document` the channel's own adapter parses at
//! delivery — which is when it needs the key material anyway. A malformed
//! document fails that one delivery and is pruned on the same path that
//! already prunes an expired endpoint.
//!
//! Storage is one JSON document per (tenant, user, extension) on the scoped
//! filesystem plane, mutated exclusively through the shared bounded CAS path
//! (`.claude/rules/database.md`). Composition chooses the backend *and* the
//! per-extension document path; this store never branches on either.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::{
    DeliveryRegistration, MAX_DELIVERY_REGISTRATION_DOCUMENT_BYTES,
    MAX_DELIVERY_REGISTRATION_ENDPOINT_BYTES, MAX_DELIVERY_REGISTRATIONS_PER_USER,
};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_product_contracts::delivery::{
    DeliveryRegistrationError, DeliveryRegistrationRequest, DeliveryRegistrationScope,
    DeliveryRegistrationService,
};
use serde::{Deserialize, Serialize};

const DOCUMENT_SCHEMA_VERSION: u32 = 2;

/// Resolves one channel's registration document path.
///
/// Composition supplies this because the *path* is deployment data, not
/// channel behavior — and because one channel's document predates this store
/// and must keep its exact bytes. A generic default would have renamed it and
/// orphaned every persisted enrollment.
pub trait DeliveryRegistrationPaths: Send + Sync {
    /// The alias-relative document path for one extension, or `None` when the
    /// deployment stores no registrations for it (fail-closed: the caller
    /// treats that as "this channel cannot be enrolled here").
    fn document_path(&self, extension_id: &str) -> Option<ScopedPath>;
}

/// Filesystem-plane implementation over a per-user [`ScopedFilesystem`].
pub struct FilesystemDeliveryRegistrationStore<F>
where
    F: RootFilesystem + ?Sized,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    paths: Arc<dyn DeliveryRegistrationPaths>,
}

impl<F> FilesystemDeliveryRegistrationStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        paths: Arc<dyn DeliveryRegistrationPaths>,
    ) -> Self {
        Self { filesystem, paths }
    }

    fn path_for(
        &self,
        scope: &DeliveryRegistrationScope,
    ) -> Result<ScopedPath, DeliveryRegistrationError> {
        self.paths
            .document_path(scope.extension_id.as_str())
            .ok_or_else(|| DeliveryRegistrationError::Rejected {
                reason: "this deployment stores no delivery registrations for that channel"
                    .to_string(),
            })
    }
}

/// The persisted document.
///
/// `records` decode tolerantly: the pre-§8 shape stored channel-specific
/// fields at the top level under `subscriptions`, so the private wire reader
/// folds every extra field into the opaque `document` without interpreting
/// it. The writer emits both the canonical `records` projection and that
/// flattened rollback projection; neither changes the stable host id.
#[derive(Debug, Clone, PartialEq)]
struct RegistrationDocument {
    schema_version: u32,
    tenant_id: String,
    user_id: String,
    records: Vec<DeliveryRegistration>,
}

#[derive(Deserialize)]
struct RegistrationDocumentWire {
    schema_version: u32,
    tenant_id: String,
    user_id: String,
    #[serde(default)]
    records: Option<Vec<StoredRegistrationWire>>,
    #[serde(default)]
    subscriptions: Option<Vec<StoredRegistrationWire>>,
}

impl<'de> Deserialize<'de> for RegistrationDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RegistrationDocumentWire::deserialize(deserializer)?;
        Ok(Self {
            schema_version: wire.schema_version,
            tenant_id: wire.tenant_id,
            user_id: wire.user_id,
            // A current writer emits both keys. `records` is canonical when
            // both are present; a rolled-back previous writer drops unknown
            // `records` and leaves only `subscriptions`, which remains
            // readable on the next rollout.
            records: wire
                .records
                .or(wire.subscriptions)
                .unwrap_or_default()
                .into_iter()
                .map(StoredRegistrationWire::into_registration)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Deserialize)]
struct StoredRegistrationWire {
    #[serde(default)]
    registration_id: Option<String>,
    #[serde(default)]
    subscription_id: Option<String>,
    endpoint: String,
    #[serde(default)]
    document: Option<String>,
    created_at: String,
    #[serde(flatten)]
    legacy_document: serde_json::Map<String, serde_json::Value>,
}

impl StoredRegistrationWire {
    fn into_registration<E>(self) -> Result<DeliveryRegistration, E>
    where
        E: serde::de::Error,
    {
        let wire = self;
        let registration_id = match (wire.registration_id, wire.subscription_id) {
            (Some(current), Some(previous)) if current != previous => {
                return Err(E::custom("registration_id and subscription_id disagree"));
            }
            (Some(current), _) => current,
            (_, Some(previous)) => previous,
            (None, None) => {
                return Err(E::missing_field("registration_id"));
            }
        };
        let document = wire
            .document
            .unwrap_or_else(|| serde_json::Value::Object(wire.legacy_document).to_string());
        Ok(DeliveryRegistration {
            registration_id,
            endpoint: wire.endpoint,
            document,
            created_at: wire.created_at,
        })
    }
}

impl Serialize for RegistrationDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let subscriptions = self
            .records
            .iter()
            .map(legacy_subscription_value)
            .collect::<Vec<_>>();
        serde_json::json!({
            "schema_version": self.schema_version,
            "tenant_id": self.tenant_id,
            "user_id": self.user_id,
            "records": self.records,
            "subscriptions": subscriptions,
        })
        .serialize(serializer)
    }
}

/// Rollback projection for the immediately preceding Web Push reader. The
/// opaque document is flattened without interpreting any protocol field; a
/// non-object document simply contributes no legacy-only fields.
fn legacy_subscription_value(registration: &DeliveryRegistration) -> serde_json::Value {
    let mut fields = serde_json::from_str::<serde_json::Value>(&registration.document)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for reserved in [
        "registration_id",
        "subscription_id",
        "endpoint",
        "document",
        "created_at",
    ] {
        fields.remove(reserved);
    }
    fields.insert(
        "subscription_id".to_string(),
        serde_json::json!(registration.registration_id),
    );
    fields.insert(
        "endpoint".to_string(),
        serde_json::json!(registration.endpoint),
    );
    fields.insert(
        "created_at".to_string(),
        serde_json::json!(registration.created_at),
    );
    serde_json::Value::Object(fields)
}

impl RegistrationDocument {
    fn empty(scope: &DeliveryRegistrationScope) -> Self {
        Self {
            schema_version: DOCUMENT_SCHEMA_VERSION,
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            records: Vec::new(),
        }
    }

    /// Defense in depth beyond path scoping: a document whose recorded owner
    /// disagrees with the requesting scope is corrupt or misrouted; refuse
    /// rather than serve another user's registrations.
    fn validate_owner(
        &self,
        scope: &DeliveryRegistrationScope,
    ) -> Result<(), DeliveryRegistrationError> {
        if self.tenant_id != scope.tenant_id.to_string()
            || self.user_id != scope.user_id.to_string()
        {
            return Err(DeliveryRegistrationError::Unavailable {
                reason: "stored registration document does not belong to the requested scope"
                    .to_string(),
            });
        }
        Ok(())
    }
}

/// The one security-critical, generic, pre-storage check (§8): the submitted
/// endpoint must be an absolute HTTPS URL whose host the channel declares in
/// `[[channel.egress]]`.
///
/// `declared_hosts` comes from the resolved manifest, so this is not a second
/// allowlist that can drift from the egress policy — it is the same one.
pub fn validate_registration_endpoint(
    endpoint: &str,
    declared_hosts: &[String],
) -> Result<String, DeliveryRegistrationError> {
    let reject = |reason: &str| DeliveryRegistrationError::Rejected {
        reason: reason.to_string(),
    };
    if endpoint.is_empty() || endpoint.len() > MAX_DELIVERY_REGISTRATION_ENDPOINT_BYTES {
        return Err(reject(
            "registration endpoint is empty or exceeds its bound",
        ));
    }
    if endpoint.chars().any(char::is_control) {
        return Err(reject("registration endpoint contains control characters"));
    }
    let rest = endpoint
        .strip_prefix("https://")
        .ok_or_else(|| reject("registration endpoint must be an absolute https URL"))?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| reject("registration endpoint has no host"))?;
    // Credentials in the authority would let a submitted URL smuggle a
    // different effective host past a naive prefix comparison.
    if authority.contains('@') {
        return Err(reject("registration endpoint must not carry userinfo"));
    }
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _port)| host)
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err(reject("registration endpoint has no host"));
    }
    if !declared_hosts
        .iter()
        .any(|declared| declared.eq_ignore_ascii_case(&host))
    {
        return Err(reject(
            "registration endpoint host is not declared by this channel's egress policy",
        ));
    }
    Ok(host)
}

fn validate_document(document: &str) -> Result<(), DeliveryRegistrationError> {
    if document.len() > MAX_DELIVERY_REGISTRATION_DOCUMENT_BYTES {
        return Err(DeliveryRegistrationError::Rejected {
            reason: "registration document exceeds its size bound".to_string(),
        });
    }
    Ok(())
}

fn resource_scope(scope: &DeliveryRegistrationScope) -> ResourceScope {
    ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn decode_document(bytes: &[u8]) -> Result<RegistrationDocument, DeliveryRegistrationError> {
    let document: RegistrationDocument =
        serde_json::from_slice(bytes).map_err(|error| DeliveryRegistrationError::Unavailable {
            reason: format!("registration document decode failed: {error}"),
        })?;
    Ok(document)
}

fn encode_document(document: &RegistrationDocument) -> Result<Entry, DeliveryRegistrationError> {
    let bytes =
        serde_json::to_vec(document).map_err(|error| DeliveryRegistrationError::Unavailable {
            reason: format!("registration document encode failed: {error}"),
        })?;
    Ok(Entry::bytes(bytes).with_content_type(ContentType::json()))
}

fn map_cas_error(error: CasUpdateError<DeliveryRegistrationError>) -> DeliveryRegistrationError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        other => DeliveryRegistrationError::Unavailable {
            reason: format!("registration document update failed: {other}"),
        },
    }
}

#[async_trait]
impl<F> DeliveryRegistrationService for FilesystemDeliveryRegistrationStore<F>
where
    F: RootFilesystem + ?Sized,
{
    async fn list(
        &self,
        scope: &DeliveryRegistrationScope,
    ) -> Result<Vec<DeliveryRegistration>, DeliveryRegistrationError> {
        let path = self.path_for(scope)?;
        let resource = resource_scope(scope);
        let entry = self
            .filesystem
            .get(&resource, &path)
            .await
            .map_err(|error| DeliveryRegistrationError::Unavailable {
                reason: format!("registration document read failed: {error}"),
            })?;
        let Some(entry) = entry else {
            return Ok(Vec::new());
        };
        let document = decode_document(&entry.entry.body)?;
        document.validate_owner(scope)?;
        Ok(document.records)
    }

    async fn enroll(
        &self,
        scope: &DeliveryRegistrationScope,
        request: DeliveryRegistrationRequest,
    ) -> Result<DeliveryRegistration, DeliveryRegistrationError> {
        // The endpoint check is the caller's (it holds the manifest's
        // declared hosts); this store re-checks only what it owns: bounds.
        validate_document(&request.document)?;
        if request.endpoint.len() > MAX_DELIVERY_REGISTRATION_ENDPOINT_BYTES {
            return Err(DeliveryRegistrationError::Rejected {
                reason: "registration endpoint exceeds its bound".to_string(),
            });
        }
        let path = self.path_for(scope)?;
        let resource = resource_scope(scope);
        let created_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let stored = DeliveryRegistration {
            registration_id: uuid::Uuid::new_v4().to_string(),
            endpoint: request.endpoint,
            document: request.document,
            created_at,
        };
        cas_update(
            self.filesystem.as_ref(),
            &resource,
            &path,
            decode_document,
            encode_document,
            move |current: Option<RegistrationDocument>| {
                let stored = stored.clone();
                let scope = scope.clone();
                async move {
                    let mut document =
                        current.unwrap_or_else(|| RegistrationDocument::empty(&scope));
                    document.validate_owner(&scope)?;
                    document.schema_version = DOCUMENT_SCHEMA_VERSION;
                    if let Some(existing) = document
                        .records
                        .iter_mut()
                        .find(|existing| existing.endpoint == stored.endpoint)
                    {
                        // Re-enrolling the same endpoint refreshes it (rotated
                        // opaque material) while retaining the host id and
                        // original creation time.
                        existing.endpoint = stored.endpoint.clone();
                        existing.document = stored.document.clone();
                        let view = existing.clone();
                        return Ok(CasApply::new(document, view));
                    }
                    if document.records.len() >= MAX_DELIVERY_REGISTRATIONS_PER_USER {
                        return Err(DeliveryRegistrationError::Rejected {
                            reason: format!(
                                "a user may hold at most {MAX_DELIVERY_REGISTRATIONS_PER_USER} \
                                 registrations per channel"
                            ),
                        });
                    }
                    // Newest first so settings lists the most recent client on top.
                    document.records.insert(0, stored);
                    let view = document.records[0].clone();
                    Ok(CasApply::new(document, view))
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn remove(
        &self,
        scope: &DeliveryRegistrationScope,
        registration_id: &str,
    ) -> Result<bool, DeliveryRegistrationError> {
        let removed = self.prune(scope, &[registration_id.to_string()]).await?;
        Ok(removed > 0)
    }

    async fn prune(
        &self,
        scope: &DeliveryRegistrationScope,
        registration_ids: &[String],
    ) -> Result<usize, DeliveryRegistrationError> {
        if registration_ids.is_empty() {
            return Ok(0);
        }
        let path = self.path_for(scope)?;
        let resource = resource_scope(scope);
        let ids: Vec<String> = registration_ids.to_vec();
        cas_update(
            self.filesystem.as_ref(),
            &resource,
            &path,
            decode_document,
            encode_document,
            move |current: Option<RegistrationDocument>| {
                let ids = ids.clone();
                let scope = scope.clone();
                async move {
                    let Some(mut document) = current else {
                        return Ok(CasApply::no_op(RegistrationDocument::empty(&scope), 0usize));
                    };
                    document.validate_owner(&scope)?;
                    let before = document.records.len();
                    document
                        .records
                        .retain(|record| !ids.contains(&record.registration_id));
                    let removed = before - document.records.len();
                    if removed == 0 {
                        return Ok(CasApply::no_op(document, 0usize));
                    }
                    document.schema_version = DOCUMENT_SCHEMA_VERSION;
                    Ok(CasApply::new(document, removed))
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::{ExtensionId, TenantId, UserId};

    struct FixedPaths(&'static str);

    impl DeliveryRegistrationPaths for FixedPaths {
        fn document_path(&self, extension_id: &str) -> Option<ScopedPath> {
            if extension_id == "unmapped" {
                return None;
            }
            ScopedPath::new(self.0).ok()
        }
    }

    fn scope(user: &str) -> DeliveryRegistrationScope {
        DeliveryRegistrationScope {
            tenant_id: TenantId::new("tenant1").expect("tenant"),
            user_id: UserId::new(user).expect("user"),
            extension_id: ExtensionId::new("channel-one").expect("extension"),
        }
    }

    fn store() -> FilesystemDeliveryRegistrationStore<InMemoryBackend> {
        FilesystemDeliveryRegistrationStore::new(
            Arc::new(ScopedFilesystem::new(
                Arc::new(InMemoryBackend::new()),
                |scope: &ResourceScope| {
                    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
                    use ironclaw_host_api::path::{MountAlias, VirtualPath};
                    MountView::new(vec![MountGrant::new(
                        MountAlias::new("/registrations")?,
                        VirtualPath::new(format!(
                            "/tenants/{}/users/{}/registrations",
                            scope.tenant_id, scope.user_id
                        ))?,
                        MountPermissions::read_write_list_delete(),
                    )])
                },
            )),
            Arc::new(FixedPaths("/registrations/channel-one.json")),
        )
    }

    fn request(token: &str) -> DeliveryRegistrationRequest {
        DeliveryRegistrationRequest {
            endpoint: format!("https://push.alpha.example/send/{token}"),
            document: r#"{"keys":{"p256dh":"a","auth":"b"}}"#.to_string(),
        }
    }

    #[test]
    fn the_pre_storage_endpoint_check_admits_only_declared_hosts() {
        let declared = vec!["push.alpha.example".to_string()];
        assert_eq!(
            validate_registration_endpoint("https://push.alpha.example/send/x", &declared)
                .expect("declared host is admitted"),
            "push.alpha.example"
        );
        // Port and case are not part of the host comparison.
        assert!(
            validate_registration_endpoint("https://PUSH.Alpha.Example:443/send/x", &declared)
                .is_ok()
        );

        // Every rejection below is the SSRF primitive this check exists to
        // close: an attacker-chosen destination for a host-issued POST.
        for hostile in [
            "https://evil.example/send/x",
            "http://push.alpha.example/send/x",
            "https://push.alpha.example@evil.example/send/x",
            "//push.alpha.example/send/x",
            "https:///send/x",
            "file:///etc/passwd",
            "https://push.alpha.example.evil.example/x",
        ] {
            assert!(
                matches!(
                    validate_registration_endpoint(hostile, &declared),
                    Err(DeliveryRegistrationError::Rejected { .. })
                ),
                "expected rejection for {hostile}"
            );
        }

        // A channel that declares no egress host can enroll nothing.
        assert!(
            validate_registration_endpoint("https://push.alpha.example/x", &[]).is_err(),
            "an empty allowlist must not admit anything"
        );
    }

    #[tokio::test]
    async fn enroll_refresh_list_remove_round_trip() {
        let store = store();
        let scope = scope("user1");

        let first = store
            .enroll(&scope, request("alpha"))
            .await
            .expect("enroll");
        assert!(
            uuid::Uuid::parse_str(&first.registration_id).is_ok(),
            "new records use opaque host-minted ids"
        );
        store
            .enroll(&scope, request("beta"))
            .await
            .expect("second client");
        assert_eq!(store.list(&scope).await.expect("list").len(), 2);

        // Same endpoint again is a refresh, not a duplicate.
        let refreshed = store
            .enroll(&scope, request("alpha"))
            .await
            .expect("refresh");
        assert_eq!(
            refreshed.registration_id, first.registration_id,
            "refresh keeps the host registration identity"
        );
        assert_eq!(store.list(&scope).await.expect("list").len(), 2);

        assert!(
            store
                .remove(&scope, &first.registration_id)
                .await
                .expect("remove")
        );
        assert_eq!(store.list(&scope).await.expect("list").len(), 1);
        assert!(
            !store
                .remove(&scope, &first.registration_id)
                .await
                .expect("remove absent")
        );
    }

    #[tokio::test]
    async fn registrations_are_isolated_per_user() {
        let store = store();
        store
            .enroll(&scope("user1"), request("alpha"))
            .await
            .expect("enroll");
        assert!(
            store.list(&scope("user2")).await.expect("list").is_empty(),
            "another user's scope must not see the registration"
        );
    }

    #[tokio::test]
    async fn the_per_user_cap_fails_closed() {
        let store = store();
        let scope = scope("user1");
        for index in 0..MAX_DELIVERY_REGISTRATIONS_PER_USER {
            store
                .enroll(&scope, request(&format!("t{index}")))
                .await
                .expect("enroll under cap");
        }
        assert!(matches!(
            store.enroll(&scope, request("overflow")).await,
            Err(DeliveryRegistrationError::Rejected { .. })
        ));
    }

    #[tokio::test]
    async fn an_oversized_document_is_rejected_before_storage() {
        let store = store();
        let scope = scope("user1");
        let oversized = DeliveryRegistrationRequest {
            endpoint: "https://push.alpha.example/send/x".to_string(),
            document: "x".repeat(MAX_DELIVERY_REGISTRATION_DOCUMENT_BYTES + 1),
        };
        assert!(matches!(
            store.enroll(&scope, oversized).await,
            Err(DeliveryRegistrationError::Rejected { .. })
        ));
        assert!(store.list(&scope).await.expect("list").is_empty());
    }

    /// The forward migration, at the seam that performs it: a document
    /// written before §8 stored the channel's fields at the top level. It must
    /// still decode, and its channel-specific fields must arrive in the opaque
    /// document the adapter now parses — a rename here would orphan every
    /// live enrollment.
    #[tokio::test]
    async fn a_pre_section_eight_document_migrates_forward_on_read() {
        let store = store();
        let scope = scope("user1");
        let legacy = serde_json::json!({
            "schema_version": 1,
            "tenant_id": "tenant1",
            "user_id": "user1",
            "subscriptions": [{
                "subscription_id": "legacy-id",
                "endpoint": "https://push.alpha.example/send/legacy",
                "keys": {"p256dh": "legacy-p256dh", "auth": "legacy-auth"},
                "user_agent": "Legacy Browser",
                "created_at": "2026-08-08T00:00:00Z",
            }],
        });
        store
            .filesystem
            .put(
                &resource_scope(&scope),
                &ScopedPath::new("/registrations/channel-one.json").expect("path"),
                Entry::bytes(serde_json::to_vec(&legacy).expect("encode"))
                    .with_content_type(ContentType::json()),
                ironclaw_filesystem::CasExpectation::Any,
            )
            .await
            .expect("seed the legacy document");

        let listed = store.list(&scope).await.expect("list");
        assert_eq!(listed.len(), 1);
        let migrated_id = "legacy-id";
        assert_eq!(listed[0].registration_id, migrated_id);
        assert_eq!(listed[0].endpoint, "https://push.alpha.example/send/legacy");
        let document: serde_json::Value =
            serde_json::from_str(&listed[0].document).expect("folded document is JSON");
        assert_eq!(document["keys"]["p256dh"], "legacy-p256dh");
        assert_eq!(document["user_agent"], "Legacy Browser");

        // And the next write persists the migrated shape without losing it.
        store
            .enroll(&scope, request("fresh"))
            .await
            .expect("enroll alongside the legacy record");
        let listed = store.list(&scope).await.expect("list");
        assert_eq!(listed.len(), 2);
        let migrated = listed
            .iter()
            .find(|record| record.registration_id == migrated_id)
            .expect("the legacy record survives the rewrite");
        assert!(migrated.document.contains("legacy-p256dh"));

        // Refresh preserves the pre-existing host id rather than replacing it
        // with endpoint-derived identity, and remove addresses that id.
        let refreshed = store
            .enroll(&scope, request("legacy"))
            .await
            .expect("refresh the migrated endpoint");
        assert_eq!(refreshed.registration_id, migrated_id);
        assert_eq!(store.list(&scope).await.expect("list").len(), 2);
        assert!(store.remove(&scope, migrated_id).await.expect("remove"));
        assert_eq!(store.list(&scope).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn writer_keeps_generic_and_legacy_registration_documents_rollback_readable() {
        #[derive(Deserialize)]
        struct LegacyDocument {
            subscriptions: Vec<LegacySubscription>,
        }

        #[derive(Deserialize)]
        struct LegacySubscription {
            subscription_id: String,
            endpoint: String,
            keys: serde_json::Value,
            user_agent: Option<String>,
        }

        let store = store();
        let scope = scope("user1");
        let enrolled = store
            .enroll(&scope, request("rollback"))
            .await
            .expect("enroll");
        let entry = store
            .filesystem
            .get(
                &resource_scope(&scope),
                &ScopedPath::new("/registrations/channel-one.json").expect("path"),
            )
            .await
            .expect("read persisted document")
            .expect("persisted document");
        let value: serde_json::Value =
            serde_json::from_slice(&entry.entry.body).expect("generic document json");
        assert_eq!(
            value["records"][0]["registration_id"],
            enrolled.registration_id
        );

        let legacy: LegacyDocument =
            serde_json::from_value(value).expect("previous Web Push reader can decode the write");
        assert_eq!(legacy.subscriptions.len(), 1);
        assert_eq!(
            legacy.subscriptions[0].subscription_id,
            enrolled.registration_id
        );
        assert_eq!(
            legacy.subscriptions[0].endpoint,
            "https://push.alpha.example/send/rollback"
        );
        assert_eq!(legacy.subscriptions[0].keys["p256dh"], "a");
        assert_eq!(legacy.subscriptions[0].user_agent, None);
    }

    /// The reserved-key strip is the rollback-SSRF guard: the enrollment
    /// document is client-supplied and only bounded, never parsed, so a
    /// document that itself smuggles `"endpoint": "https://evil.example/x"`
    /// (or a forged id/timestamp) must NOT be able to rewrite the endpoint a
    /// rolled-back Web Push reader would post to. The admitted, host-checked
    /// values always win.
    #[tokio::test]
    async fn legacy_projection_strips_document_smuggled_reserved_keys() {
        let store = store();
        let scope = scope("user1");
        let enrolled = store
            .enroll(
                &scope,
                DeliveryRegistrationRequest {
                    endpoint: "https://push.alpha.example/send/honest".to_string(),
                    document: r#"{
                        "keys": {"p256dh": "a", "auth": "b"},
                        "endpoint": "https://evil.example/exfiltrate",
                        "subscription_id": "forged-id",
                        "registration_id": "forged-id",
                        "created_at": "1999-01-01T00:00:00Z",
                        "document": "nested-noise"
                    }"#
                    .to_string(),
                },
            )
            .await
            .expect("enroll");
        let entry = store
            .filesystem
            .get(
                &resource_scope(&scope),
                &ScopedPath::new("/registrations/channel-one.json").expect("path"),
            )
            .await
            .expect("read persisted document")
            .expect("persisted document");
        let value: serde_json::Value =
            serde_json::from_slice(&entry.entry.body).expect("document json");
        let subscription = &value["subscriptions"][0];
        assert_eq!(
            subscription["endpoint"], "https://push.alpha.example/send/honest",
            "the admitted endpoint wins over the smuggled one: {subscription}"
        );
        assert_eq!(
            subscription["subscription_id"], enrolled.registration_id,
            "the host-minted id wins over the forged one"
        );
        assert_ne!(subscription["created_at"], "1999-01-01T00:00:00Z");
        assert_eq!(subscription["document"], serde_json::Value::Null);
        // The generic `records` half retains the opaque document verbatim —
        // inert stored bytes, never routed on. The projection a rolled-back
        // reader ROUTES on is `subscriptions`, and no smuggled value may
        // survive there.
        assert!(
            !value["subscriptions"].to_string().contains("evil.example"),
            "no smuggled endpoint survives in the routable legacy projection: {value}"
        );
    }

    #[tokio::test]
    async fn an_unmapped_channel_fails_closed() {
        let store = store();
        let mut scope = scope("user1");
        scope.extension_id = ExtensionId::new("unmapped").expect("extension");
        assert!(matches!(
            store.list(&scope).await,
            Err(DeliveryRegistrationError::Rejected { .. })
        ));
    }

    /// Two clients enrolling at once against the same empty document must
    /// BOTH land: `cas_update` re-reads and re-applies on conflict.
    #[tokio::test]
    async fn concurrent_enrollments_both_persist() {
        let store = Arc::new(store());
        let scope = scope("user1");
        let first = {
            let store = Arc::clone(&store);
            let scope = scope.clone();
            tokio::spawn(async move { store.enroll(&scope, request("alpha")).await })
        };
        let second = {
            let store = Arc::clone(&store);
            let scope = scope.clone();
            tokio::spawn(async move { store.enroll(&scope, request("beta")).await })
        };
        first.await.expect("join alpha").expect("enroll alpha");
        second.await.expect("join beta").expect("enroll beta");
        assert_eq!(
            store.list(&scope).await.expect("list").len(),
            2,
            "a lost enrollment means the CAS retry clobbered instead of re-applying"
        );
    }
}
