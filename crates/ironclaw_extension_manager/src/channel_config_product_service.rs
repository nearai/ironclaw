//! The `[channel.config]` product service — the manager-side face of
//! [`ironclaw_extension_host::ChannelConfigService`].
//!
//! The host owns the *service core*: validation against the installed
//! manifest, the durable/secret write split, and the §6.5 reactivate cycle.
//! What lives here is only the product port over it — the projection the
//! WebUI setup service and the lifecycle configure action see, and the
//! `ChannelConfigError` → `ProductSurfaceError` status table that decides
//! what those callers are told. Splitting them is the point of §6.8.3: the
//! host keeps the authority, the manager keeps the UX semantics.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::{ChannelConfigError, ChannelConfigService};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_product_contracts::channel_config::ChannelConfigProductService;
use ironclaw_product_contracts::package_lifecycle::ChannelConfigField;
use ironclaw_product_contracts::surface::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};

/// The production [`ChannelConfigProductService`] port
/// over [`ChannelConfigService`] — the surface the WebUI setup service and
/// the lifecycle configure action route through.
pub struct RebornChannelConfigProductService {
    service: Arc<ChannelConfigService>,
}

impl RebornChannelConfigProductService {
    pub fn new(service: Arc<ChannelConfigService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl ChannelConfigProductService for RebornChannelConfigProductService {
    async fn field_status(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ChannelConfigField>, ProductSurfaceError> {
        match self
            .service
            .declares_admin_configuration(extension_id)
            .await
        {
            // The extension is administrator-configured: its fields are the
            // admin surface's, not this projection's.
            Ok(true) => return Ok(Vec::new()),
            Ok(false) => {}
            // Neither installed nor in the available catalog — nothing to
            // configure. `status()` answers the same way for the same reason
            // below, so fall through rather than deciding it twice.
            Err(ChannelConfigError::NotInstalled { .. }) => {}
            // Anything else is a real read failure. Projecting it as "no
            // fields" would tell the WebUI an extension has nothing to
            // configure because the host could not read whether it does.
            Err(error) => return Err(map_channel_config_error(error)),
        }
        match self.service.status(extension_id).await {
            Ok(statuses) => Ok(statuses
                .into_iter()
                .map(|status| ChannelConfigField {
                    name: status.handle,
                    label: status.label,
                    secret: status.secret,
                    provided: status.provided,
                })
                .collect()),
            // A not-yet-installed extension has nothing to configure; the
            // setup view renders for it, so this projection stays empty
            // rather than erroring.
            Err(ChannelConfigError::NotInstalled { .. }) => Ok(Vec::new()),
            Err(error) => Err(map_channel_config_error(error)),
        }
    }

    async fn save_values(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), ProductSurfaceError> {
        self.service
            .save(extension_id, values)
            .await
            .map_err(map_channel_config_error)
    }
}

fn map_channel_config_error(error: ChannelConfigError) -> ProductSurfaceError {
    match error {
        ChannelConfigError::NotInstalled { .. } => ProductSurfaceError {
            code: ProductSurfaceErrorCode::NotFound,
            kind: ProductSurfaceErrorKind::NotFound,
            status_code: 404,
            retryable: false,
            field: None,
            validation_code: None,
        },
        ChannelConfigError::UnknownField { .. } => ProductSurfaceError {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: None,
        },
        ChannelConfigError::Storage { .. } => ProductSurfaceError {
            code: ProductSurfaceErrorCode::Unavailable,
            kind: ProductSurfaceErrorKind::ServiceUnavailable,
            status_code: 503,
            retryable: true,
            field: None,
            validation_code: None,
        },
        // The save persisted but the §6.5 reactivate cycle failed: the host
        // record is left per §6.1 with the typed reason; the operator fixes
        // the value and saves again.
        ChannelConfigError::Reactivation { .. } => ProductSurfaceError {
            code: ProductSurfaceErrorCode::Conflict,
            kind: ProductSurfaceErrorKind::Conflict,
            status_code: 409,
            retryable: false,
            field: None,
            validation_code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_extension_host::{ChannelConfigReactivation, ChannelConfigReactivationError};
    use ironclaw_extensions::{
        ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationId,
        ExtensionInstallationStore, ExtensionInstallationStorePort, ExtensionManifestRecord,
        ExtensionManifestRef, MembershipDeactivation,
    };
    use ironclaw_filesystem::{
        Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, RootFilesystem,
    };
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        path::VirtualPath,
        resource::ResourceScope,
    };
    use ironclaw_secrets::{SecretStore, SecretStorePort};

    use super::*;

    /// The §6.5 reactivate cycle is not what these cases exercise; the
    /// service still requires the port, so this is the inert one.
    struct NeverReactivates;

    #[async_trait]
    impl ChannelConfigReactivation for NeverReactivates {
        async fn reactivate_if_active(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), ChannelConfigReactivationError> {
            Ok(())
        }
    }

    /// A manifest-read fault reaches the caller instead of reading as
    /// "nothing to configure".
    ///
    /// `field_status` asks the host twice: once for
    /// `declares_admin_configuration` (the admin-configuration suppression
    /// check) and once for `status()`. Swallowing the first read's error
    /// collapses a storage fault into the suppression check's "no" answer, and
    /// the `status()` call that follows then answers `NotInstalled` for the
    /// very same extension — so the WebUI renders an empty field list for an
    /// extension whose fields could not be read at all. The two reads have
    /// different fallbacks (`resolved_manifest` consults the available
    /// catalog, `status()`'s completeness path does not), so they do not fail
    /// together by construction and the swallowed error is observable today,
    /// not only after some future cache lands.
    ///
    /// The fault is injected *below* the real installation store, so the
    /// production read path and its `FilesystemError` ->
    /// `ExtensionInstallationError` -> `ChannelConfigError::Storage` mapping
    /// all run for real rather than being asserted by a whole-trait fake.
    #[tokio::test]
    async fn a_manifest_read_fault_is_not_projected_as_an_empty_field_list() {
        let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
        let installation_store = ExtensionInstallationStore::load_at(
            Arc::clone(&backend) as Arc<dyn RootFilesystem>,
            VirtualPath::new("/system/extensions/.installations/test").expect("valid test path"),
            ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"),
            ironclaw_host_runtime::default_host_api_contract_registry().expect("host contracts"),
        )
        .await
        .expect("filesystem extension installation store");
        // Fail only the first record query after load — the one
        // `declares_admin_configuration` issues. The `status()` query that
        // follows succeeds and reports "not installed": exactly the shape
        // that turns a swallowed fault into an empty field list.
        backend.add_fault(
            Fault::on(FilesystemOperation::Query)
                .nth(1)
                .backend("installation store offline"),
        );

        let service = RebornChannelConfigProductService::new(Arc::new(ChannelConfigService::new(
            Arc::new(installation_store),
            Arc::new(SecretStore::ephemeral()),
            ResourceScope::local_default(
                UserId::new("channel-config-owner").expect("user id"),
                InvocationId::new(),
            )
            .expect("resource scope"),
            Arc::new(NeverReactivates),
        )));

        let error = service
            .field_status(&ExtensionId::new("acmechat").expect("extension id"))
            .await
            .expect_err("a manifest read fault must not read as 'nothing to configure'");
        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.kind, ProductSurfaceErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable, "a storage fault is worth retrying");
    }

    /// The field projection itself: `ChannelConfigFieldStatus` (host) ->
    /// `ChannelConfigField` (product wire). It renames `handle` to `name` and
    /// carries `label`/`secret`/`provided` across, and nothing had ever run it
    /// -- every other case here drives an empty list, which cannot tell a
    /// correct mapping from one that transposed two fields or dropped
    /// `provided`.
    ///
    /// Reaching it needs the `NotInstalled` fall-through at the suppression
    /// check, which is exactly what that arm exists for: `field_status` asks
    /// the host twice against a shared durable store, so an extension that
    /// finishes installing between the two reads answers "not installed" to
    /// the first and hands the second a manifest with fields. The comment on
    /// that arm says it falls through "rather than deciding it twice"; this is
    /// the case where the second decision differs.
    ///
    /// Non-vacuous by construction: the double is keyed on `extension_id` and
    /// defers only `telegram`'s first read, so `slack` -- installed in the same
    /// store, visible to both of its reads -- takes the suppression path and
    /// projects nothing. Same service, same call, two answers.
    #[tokio::test]
    async fn manifest_field_status_is_projected_onto_the_product_wire_shape() {
        let telegram = ExtensionId::new("telegram").expect("extension id");
        let slack = ExtensionId::new("slack").expect("extension id");
        let scope = ResourceScope::local_default(
            UserId::new("channel-config-owner").expect("user id"),
            InvocationId::new(),
        )
        .expect("resource scope");
        let installation_store =
            installed_store(&[(TELEGRAM_MANIFEST, "telegram"), (SLACK_MANIFEST, "slack")]).await;
        let secrets = Arc::new(SecretStore::ephemeral());
        // One field already has stored material, so `provided` cannot pass by
        // being uniformly false.
        secrets
            .put(
                scope.clone(),
                ironclaw_host_api::ids::SecretHandle::new("telegram_bot_token")
                    .expect("secret handle"),
                ironclaw_secrets::SecretMaterial::from("bot-token".to_string()),
                None,
            )
            .await
            .expect("seed the stored bot token");

        let service = RebornChannelConfigProductService::new(Arc::new(ChannelConfigService::new(
            Arc::new(DeferredFirstManifestRead {
                inner: installation_store,
                deferred: telegram.clone(),
                reads: AtomicUsize::new(0),
            }),
            secrets,
            scope,
            Arc::new(NeverReactivates),
        )));

        let fields = service
            .field_status(&telegram)
            .await
            .expect("an installed manifest's fields project onto the wire shape");
        assert_eq!(
            fields,
            vec![
                ChannelConfigField {
                    name: "telegram_bot_token".to_string(),
                    label: "Bot token".to_string(),
                    secret: true,
                    provided: true,
                },
                ChannelConfigField {
                    name: "telegram_webhook_secret".to_string(),
                    label: "Webhook secret token".to_string(),
                    secret: true,
                    provided: false,
                },
                ChannelConfigField {
                    name: "telegram_webhook_url".to_string(),
                    label: "Public webhook URL".to_string(),
                    secret: false,
                    provided: false,
                },
                ChannelConfigField {
                    name: "bot_username".to_string(),
                    label: "Bot username".to_string(),
                    secret: false,
                    provided: false,
                },
            ],
            "the handle becomes `name`, and label/secret/provided cross unchanged"
        );

        assert_eq!(
            service
                .field_status(&slack)
                .await
                .expect("an administrator-configured extension still answers"),
            Vec::new(),
            "an extension whose manifest is visible to both reads is suppressed, \
             so the projection above is keyed on the extension asked about"
        );
    }

    /// An installation store that answers one named extension's *first*
    /// manifest read as "not installed" and everything after -- and every
    /// other extension, always -- from the real store. Models the install that
    /// lands between `field_status`'s two reads.
    struct DeferredFirstManifestRead {
        inner: Arc<ExtensionInstallationStore>,
        deferred: ExtensionId,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl ExtensionInstallationStorePort for DeferredFirstManifestRead {
        async fn list_manifests(
            &self,
        ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
            self.inner.list_manifests().await
        }

        async fn get_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
            if extension_id == &self.deferred && self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(None);
            }
            self.inner.get_manifest(extension_id).await
        }

        async fn persist_removal_tombstone(
            &self,
            manifest: ExtensionManifestRecord,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.persist_removal_tombstone(manifest).await
        }

        async fn upsert_manifest_and_installation(
            &self,
            manifest: ExtensionManifestRecord,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner
                .upsert_manifest_and_installation(manifest, installation)
                .await
        }

        async fn list_installations(
            &self,
        ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.list_installations().await
        }

        async fn get_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
            self.inner.get_installation(installation_id).await
        }

        async fn upsert_installation(
            &self,
            installation: ExtensionInstallation,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.upsert_installation(installation).await
        }

        async fn activate_membership(
            &self,
            installation_id: &ExtensionInstallationId,
            user_id: &UserId,
        ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
            self.inner
                .activate_membership(installation_id, user_id)
                .await
        }

        async fn deactivate_membership(
            &self,
            installation_id: &ExtensionInstallationId,
            user_id: &UserId,
        ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
            self.inner
                .deactivate_membership(installation_id, user_id)
                .await
        }

        async fn delete_installation(
            &self,
            installation_id: &ExtensionInstallationId,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.delete_installation(installation_id).await
        }

        async fn delete_manifest(
            &self,
            extension_id: &ExtensionId,
        ) -> Result<(), ExtensionInstallationError> {
            self.inner.delete_manifest(extension_id).await
        }
    }

    /// Real first-party manifests, so the projected field set is the shipped
    /// one rather than a fixture that can drift from it.
    const TELEGRAM_MANIFEST: &str =
        include_str!("../../extensions/packages/telegram/manifest.toml");
    const SLACK_MANIFEST: &str = include_str!("../../extensions/packages/slack/manifest.toml");

    async fn installed_store(manifests: &[(&str, &str)]) -> Arc<ExtensionInstallationStore> {
        let store = Arc::new(
            ExtensionInstallationStore::load_at(
                Arc::new(InMemoryBackend::new()),
                VirtualPath::new("/system/extensions/.installations/test")
                    .expect("valid test path"),
                ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"),
                ironclaw_host_runtime::default_host_api_contract_registry()
                    .expect("host contracts"),
            )
            .await
            .expect("filesystem extension installation store"),
        );
        for (manifest_toml, id) in manifests {
            let record = ExtensionManifestRecord::from_toml(
                *manifest_toml,
                ironclaw_extensions::ManifestSource::HostBundled,
                &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
                None,
                &ironclaw_host_runtime::default_host_api_contract_registry().expect("contracts"),
                None,
            )
            .expect("fixture manifest parses");
            let extension_id = ExtensionId::new(*id).expect("extension id");
            store
                .upsert_manifest_and_installation(
                    record,
                    ExtensionInstallation::new(
                        ExtensionInstallationId::new(id.to_string()).expect("installation id"),
                        extension_id.clone(),
                        ExtensionManifestRef::new(extension_id, None),
                        Vec::new(),
                        chrono::Utc::now(),
                        ironclaw_extensions::InstallationOwner::Tenant,
                    )
                    .expect("installation"),
                )
                .await
                .expect("persist install");
        }
        store
    }

    /// Every arm of the status table, in one table-driven case.
    ///
    /// The projection is the only thing that decides what a WebUI caller is
    /// told when a channel-config write fails, and the four answers are
    /// deliberately different: a missing extension is a 404, a bad field name
    /// blames the caller with a 400, a storage fault is a *retryable* 503, and
    /// a failed reactivation is a 409 — the save landed, the §6.5 cycle did
    /// not, so retrying the same request would not help and the operator has
    /// to fix the value. Collapsing any pair would tell an operator to retry
    /// something that cannot succeed, or to give up on something that can.
    #[test]
    fn the_status_table_answers_each_failure_with_its_own_code() {
        let cases = [
            (
                ChannelConfigError::NotInstalled {
                    extension_id: "acme".to_string(),
                },
                ProductSurfaceErrorCode::NotFound,
                ProductSurfaceErrorKind::NotFound,
                404,
                false,
            ),
            (
                ChannelConfigError::UnknownField {
                    handle: "nope".to_string(),
                },
                ProductSurfaceErrorCode::InvalidRequest,
                ProductSurfaceErrorKind::Validation,
                400,
                false,
            ),
            (
                ChannelConfigError::Storage {
                    reason: "disk".to_string(),
                },
                ProductSurfaceErrorCode::Unavailable,
                ProductSurfaceErrorKind::ServiceUnavailable,
                503,
                true,
            ),
            (
                ChannelConfigError::Reactivation {
                    reason: "adapter refused".to_string(),
                },
                ProductSurfaceErrorCode::Conflict,
                ProductSurfaceErrorKind::Conflict,
                409,
                false,
            ),
        ];

        for (error, code, kind, status, retryable) in cases {
            let projected = map_channel_config_error(error.clone());
            assert_eq!(projected.code, code, "code for {error:?}");
            assert_eq!(projected.kind, kind, "kind for {error:?}");
            assert_eq!(projected.status_code, status, "status for {error:?}");
            assert_eq!(projected.retryable, retryable, "retryable for {error:?}");
            assert!(
                projected.field.is_none() && projected.validation_code.is_none(),
                "the projection must not leak a field name or a validation code \
                 out of the host's error text for {error:?}"
            );
        }
    }

    /// Storage is the only retryable answer. Pinned separately because
    /// "retryable" is what a WebUI client loops on: marking `Reactivation`
    /// retryable would spin a client against a value only a human can fix.
    #[test]
    fn only_a_storage_fault_is_retryable() {
        let retryable = [
            ChannelConfigError::NotInstalled {
                extension_id: "acme".to_string(),
            },
            ChannelConfigError::UnknownField {
                handle: "nope".to_string(),
            },
            ChannelConfigError::Storage {
                reason: "disk".to_string(),
            },
            ChannelConfigError::Reactivation {
                reason: "adapter refused".to_string(),
            },
        ]
        .into_iter()
        .filter(|error| map_channel_config_error(error.clone()).retryable)
        .count();
        assert_eq!(retryable, 1, "exactly one arm may tell a client to retry");
    }

    /// Composition holds this as `Arc<dyn ChannelConfigProductService>`.
    ///
    /// The coercion is the assertion — it is what fails to compile if the
    /// impl is dropped or the trait stops being object-safe, and it is the
    /// exact shape the wiring site uses. Nothing is constructed: taking the
    /// `Arc` by value and returning it as the trait object exercises the
    /// unsize coercion without needing a real `ChannelConfigService`, whose
    /// filesystem and secret-store substrates belong to the host's own tests.
    #[test]
    fn the_concrete_service_coerces_to_the_port_composition_stores() {
        fn coerce(
            service: Arc<RebornChannelConfigProductService>,
        ) -> Arc<dyn ChannelConfigProductService> {
            service
        }
        let _ = coerce;
    }
}
