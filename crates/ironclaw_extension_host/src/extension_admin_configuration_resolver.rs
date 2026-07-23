//! Manifest-indexed runtime reads over administrator configuration.
//!
//! Runtime consumers know an extension id and field handle (or, for OAuth
//! client material, only the globally declared handle). The underlying
//! [`AdminConfigurationService`] deliberately reads by group id. This
//! resolver performs that generic manifest-to-group translation once and
//! keeps persistence, secrets, and lifecycle policy in their owning services.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::{AdminConfigurationGroupId, ResolvedExtensionManifest};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use ironclaw_secrets::{SecretMaterial, SecretStore};

#[cfg(any(test, feature = "test-support"))]
use crate::{AdminConfigurationIdempotencyKey, AdminConfigurationSubmittedValue};
use crate::{AdminConfigurationService, AdminConfigurationServiceError};

/// Runtime lookup failure. Never contains administrator-provided material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionAdminConfigurationResolverError {
    #[error("extension `{extension_id}` is not available")]
    UnknownExtension { extension_id: String },
    #[error("administrator configuration handle `{handle}` is ambiguous")]
    AmbiguousFieldHandle { handle: String },
    #[error("administrator configuration handle is invalid")]
    InvalidFieldHandle,
    #[error("administrator configuration is unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldLocation {
    group_id: AdminConfigurationGroupId,
    secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IndexedField {
    Unique(FieldLocation),
    Ambiguous,
}

/// Concrete, manifest-indexed read adapter over administrator configuration.
///
/// There is intentionally no resolver trait or composition-local DTO. The
/// existing filesystem and secret-store ports provide the required runtime
/// polymorphism; this type only translates manifest identity into the service's
/// typed group lookup.
pub struct ExtensionAdminConfigurationResolver<F, S>
where
    F: RootFilesystem + ?Sized,
    S: SecretStore + ?Sized,
{
    service: Arc<AdminConfigurationService<F, S>>,
    scope: ResourceScope,
    by_extension: BTreeMap<ExtensionId, BTreeMap<SecretHandle, IndexedField>>,
    by_handle: BTreeMap<SecretHandle, IndexedField>,
}

impl<F, S> ExtensionAdminConfigurationResolver<F, S>
where
    F: RootFilesystem + ?Sized,
    S: SecretStore + ?Sized,
{
    pub fn new(
        service: Arc<AdminConfigurationService<F, S>>,
        scope: ResourceScope,
        manifests: impl IntoIterator<Item = Arc<ResolvedExtensionManifest>>,
    ) -> Self {
        let mut by_extension = BTreeMap::new();
        let mut by_handle = BTreeMap::new();
        for manifest in manifests {
            let extension_fields = by_extension.entry(manifest.id.clone()).or_default();
            for descriptor in &manifest.admin_configuration {
                for field in &descriptor.fields {
                    let location = FieldLocation {
                        group_id: descriptor.group_id.clone(),
                        secret: field.secret,
                    };
                    index_field(extension_fields, field.handle.clone(), location.clone());
                    index_field(&mut by_handle, field.handle.clone(), location);
                }
            }
        }
        Self {
            service,
            scope,
            by_extension,
            by_handle,
        }
    }

    /// Resolve one secret declared by an extension's administrator schema.
    pub async fn secret_material(
        &self,
        extension_id: &ExtensionId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ExtensionAdminConfigurationResolverError> {
        let Some(location) = self.extension_field(extension_id, handle)? else {
            return Ok(None);
        };
        if !location.secret {
            return Ok(None);
        }
        self.service
            .secret_material(&self.scope, &location.group_id, handle)
            .await
            .map_err(map_service_error)
    }

    /// Resolve one non-secret declared by an extension's administrator schema.
    pub async fn non_secret_value(
        &self,
        extension_id: &ExtensionId,
        handle: &str,
    ) -> Result<Option<String>, ExtensionAdminConfigurationResolverError> {
        let handle = SecretHandle::new(handle).map_err(|error| {
            tracing::debug!(
                %error,
                "invalid administrator configuration handle"
            );
            ExtensionAdminConfigurationResolverError::InvalidFieldHandle
        })?;
        let Some(location) = self.extension_field(extension_id, &handle)? else {
            return Ok(None);
        };
        if location.secret {
            return Ok(None);
        }
        self.service
            .non_secret_value(&self.scope, &location.group_id, &handle)
            .await
            .map_err(map_service_error)
    }

    /// Resolve globally addressed client material such as an OAuth recipe's
    /// client-id handle. Equal declarations shared by multiple manifests and
    /// the same administrator group deduplicate. A handle that names different
    /// groups or field kinds fails closed.
    pub async fn credential_handle_value(
        &self,
        handle: &str,
    ) -> Result<Option<secrecy::SecretString>, ExtensionAdminConfigurationResolverError> {
        use secrecy::ExposeSecret as _;

        let handle = SecretHandle::new(handle).map_err(|error| {
            tracing::debug!(
                %error,
                "invalid administrator configuration handle"
            );
            ExtensionAdminConfigurationResolverError::InvalidFieldHandle
        })?;
        let Some(location) = indexed_location(self.by_handle.get(&handle), &handle)? else {
            return Ok(None);
        };
        if location.secret {
            return self
                .service
                .secret_material(&self.scope, &location.group_id, &handle)
                .await
                .map_err(map_service_error)
                .map(|value| {
                    value.map(|material| {
                        secrecy::SecretString::from(material.expose_secret().to_string())
                    })
                });
        }
        self.service
            .non_secret_value(&self.scope, &location.group_id, &handle)
            .await
            .map_err(map_service_error)
            .map(|value| value.map(secrecy::SecretString::from))
    }

    /// Resolve every non-secret administrator value supplied to one
    /// extension runtime during activation.
    pub async fn effective_non_secret_config(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<(String, String)>, ExtensionAdminConfigurationResolverError> {
        // An extension with no manifest-declared `[admin_configuration]` (or
        // one imported after the boot-time index was built) has no
        // administrator values to supply — that is a normal lifecycle state,
        // not an error: activation and boot restore publish it with an empty
        // config. Credential-handle lookups (`extension_field`) stay strict.
        let Some(fields) = self.by_extension.get(extension_id) else {
            return Ok(Vec::new());
        };
        let mut values = Vec::new();
        for (handle, indexed) in fields {
            let Some(location) = indexed_location(Some(indexed), handle)? else {
                continue;
            };
            if location.secret {
                continue;
            }
            if let Some(value) = self
                .service
                .non_secret_value(&self.scope, &location.group_id, handle)
                .await
                .map_err(map_service_error)?
            {
                values.push((handle.as_str().to_string(), value));
            }
        }
        Ok(values)
    }

    /// Test-support seam for journeys that configure deployment state before
    /// exercising a non-WebUI channel path. Production writes remain behind
    /// the operator-authorized administrator capability.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn configure_admin_group_for_test(
        &self,
        group_id: &str,
        values: Vec<(String, String)>,
    ) -> Result<(), String> {
        let group_id =
            AdminConfigurationGroupId::new(group_id).map_err(|error| error.to_string())?;
        let current = self
            .service
            .get(&self.scope, &group_id)
            .await
            .map_err(|error| error.to_string())?;
        let current_revision = current.revision;
        let mut effective = current
            .fields
            .into_iter()
            .filter_map(|field| field.value.map(|value| (field.handle.to_string(), value)))
            .collect::<BTreeMap<_, _>>();
        effective.extend(values);
        let submitted = effective
            .into_iter()
            .map(|(handle, value)| {
                Ok(AdminConfigurationSubmittedValue {
                    handle: SecretHandle::new(handle).map_err(|error| error.to_string())?,
                    value: SecretMaterial::from(value),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let idempotency_key = AdminConfigurationIdempotencyKey::new(format!(
            "resolver-test-{}-{current_revision}",
            group_id.as_str()
        ))
        .map_err(|error| error.to_string())?;
        self.service
            .replace(
                &self.scope,
                &group_id,
                &idempotency_key,
                current_revision,
                submitted,
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn extension_field(
        &self,
        extension_id: &ExtensionId,
        handle: &SecretHandle,
    ) -> Result<Option<&FieldLocation>, ExtensionAdminConfigurationResolverError> {
        let fields = self.by_extension.get(extension_id).ok_or_else(|| {
            ExtensionAdminConfigurationResolverError::UnknownExtension {
                extension_id: extension_id.as_str().to_string(),
            }
        })?;
        indexed_location(fields.get(handle), handle)
    }
}

fn index_field(
    index: &mut BTreeMap<SecretHandle, IndexedField>,
    handle: SecretHandle,
    location: FieldLocation,
) {
    match index.get_mut(&handle) {
        None => {
            index.insert(handle, IndexedField::Unique(location));
        }
        Some(IndexedField::Unique(existing)) if existing == &location => {}
        Some(entry) => *entry = IndexedField::Ambiguous,
    }
}

fn indexed_location<'a>(
    indexed: Option<&'a IndexedField>,
    handle: &SecretHandle,
) -> Result<Option<&'a FieldLocation>, ExtensionAdminConfigurationResolverError> {
    match indexed {
        None => Ok(None),
        Some(IndexedField::Unique(location)) => Ok(Some(location)),
        Some(IndexedField::Ambiguous) => Err(
            ExtensionAdminConfigurationResolverError::AmbiguousFieldHandle {
                handle: handle.as_str().to_string(),
            },
        ),
    }
}

fn map_service_error(
    error: AdminConfigurationServiceError,
) -> ExtensionAdminConfigurationResolverError {
    tracing::warn!(error = %error, "administrator configuration resolution failed");
    ExtensionAdminConfigurationResolverError::Unavailable
}

#[cfg(test)]
mod tests {
    use ironclaw_extensions::{
        AdminConfigurationField, ExtensionAdminConfigurationDescriptor, ExtensionRuntimeV2,
        MANIFEST_SCHEMA_VERSION_V3, ResolvedExtensionManifest,
    };
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, RequestedTrustClass,
        TenantId, UserId, VirtualPath,
    };
    use ironclaw_secrets::FilesystemSecretStore;

    use super::*;
    use crate::FilesystemAdminConfigurationStore;

    #[tokio::test]
    async fn non_channel_oauth_credentials_resolve_from_manifest_admin_configuration() {
        let descriptor = descriptor("provider.example", "oauth_client_id", "oauth_client_secret");
        let service = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(scoped_admin_fs()),
                Arc::new(FilesystemSecretStore::ephemeral()),
                [descriptor.clone()],
            )
            .expect("descriptor catalog"),
        );
        let scope = sample_scope();
        service
            .replace(
                &scope,
                &descriptor.group_id,
                &AdminConfigurationIdempotencyKey::new("oauth-config").unwrap(),
                0,
                vec![
                    submitted("oauth_client_id", "client-id"),
                    submitted("oauth_client_secret", "client-secret"),
                ],
            )
            .await
            .unwrap();
        let manifest = manifest("non-channel-tools", descriptor);
        assert!(manifest.channel.is_none());
        let resolver = ExtensionAdminConfigurationResolver::new(service, scope, [manifest]);

        for (handle, expected) in [
            ("oauth_client_id", "client-id"),
            ("oauth_client_secret", "client-secret"),
        ] {
            let value = resolver
                .credential_handle_value(handle)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(secrecy::ExposeSecret::expose_secret(&value), expected);
        }
    }

    #[tokio::test]
    async fn globally_ambiguous_credential_handle_fails_closed() {
        let first = descriptor("provider.one", "shared_client_id", "first_secret");
        let second = descriptor("provider.two", "shared_client_id", "second_secret");
        let service = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(scoped_admin_fs()),
                Arc::new(FilesystemSecretStore::ephemeral()),
                [first.clone(), second.clone()],
            )
            .expect("descriptor catalog"),
        );
        let resolver = ExtensionAdminConfigurationResolver::new(
            service,
            sample_scope(),
            [manifest("first", first), manifest("second", second)],
        );

        let error = resolver
            .credential_handle_value("shared_client_id")
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ExtensionAdminConfigurationResolverError::AmbiguousFieldHandle {
                handle: "shared_client_id".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn extension_scoped_reads_filter_field_kinds_and_missing_values() {
        let descriptor = descriptor("provider.example", "oauth_client_id", "oauth_client_secret");
        let service = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(scoped_admin_fs()),
                Arc::new(FilesystemSecretStore::ephemeral()),
                [descriptor.clone()],
            )
            .expect("descriptor catalog"),
        );
        let scope = sample_scope();
        service
            .replace(
                &scope,
                &descriptor.group_id,
                &AdminConfigurationIdempotencyKey::new("scoped-read-config").unwrap(),
                0,
                vec![
                    submitted("oauth_client_id", "client-id"),
                    submitted("oauth_client_secret", "client-secret"),
                ],
            )
            .await
            .unwrap();
        let extension_id = ExtensionId::new("scoped-reads").unwrap();
        let resolver = ExtensionAdminConfigurationResolver::new(
            service,
            scope,
            [manifest(extension_id.as_str(), descriptor)],
        );
        let client_id_handle = SecretHandle::new("oauth_client_id").unwrap();
        let client_secret_handle = SecretHandle::new("oauth_client_secret").unwrap();
        let missing_handle = SecretHandle::new("missing_handle").unwrap();

        let secret = resolver
            .secret_material(&extension_id, &client_secret_handle)
            .await
            .unwrap()
            .expect("configured secret");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&secret),
            "client-secret"
        );
        assert!(
            resolver
                .secret_material(&extension_id, &client_id_handle)
                .await
                .unwrap()
                .is_none(),
            "a non-secret field must not resolve through the secret read path"
        );
        assert!(
            resolver
                .secret_material(&extension_id, &missing_handle)
                .await
                .unwrap()
                .is_none(),
            "an undeclared field must not resolve"
        );

        assert_eq!(
            resolver
                .non_secret_value(&extension_id, "oauth_client_id")
                .await
                .unwrap(),
            Some("client-id".to_string())
        );
        assert!(
            resolver
                .non_secret_value(&extension_id, "oauth_client_secret")
                .await
                .unwrap()
                .is_none(),
            "a secret field must not resolve through the non-secret read path"
        );
        assert!(
            resolver
                .non_secret_value(&extension_id, "missing_handle")
                .await
                .unwrap()
                .is_none(),
            "an undeclared field must not resolve"
        );
        assert_eq!(
            resolver
                .non_secret_value(&extension_id, "")
                .await
                .unwrap_err(),
            ExtensionAdminConfigurationResolverError::InvalidFieldHandle
        );

        assert_eq!(
            resolver
                .effective_non_secret_config(&extension_id)
                .await
                .unwrap(),
            vec![("oauth_client_id".to_string(), "client-id".to_string())],
            "effective runtime config includes configured non-secrets only"
        );
        // #6520 lifecycle: an extension outside the admin-configuration index
        // (nothing declared, or imported post-boot) resolves to an EMPTY
        // effective config — install/restore publish it rather than failing.
        let unknown_extension = ExtensionId::new("unknown-extension").unwrap();
        assert_eq!(
            resolver
                .effective_non_secret_config(&unknown_extension)
                .await
                .expect("undeclared extension resolves to empty config"),
            Vec::<(String, String)>::new()
        );
    }

    #[tokio::test]
    async fn extension_scoped_effective_config_rejects_ambiguous_handle() {
        let first = descriptor("provider.one", "shared_client_id", "first_secret");
        let second = descriptor("provider.two", "shared_client_id", "second_secret");
        let service = Arc::new(
            AdminConfigurationService::new(
                FilesystemAdminConfigurationStore::new(scoped_admin_fs()),
                Arc::new(FilesystemSecretStore::ephemeral()),
                [first.clone(), second.clone()],
            )
            .expect("descriptor catalog"),
        );
        let extension_id = ExtensionId::new("ambiguous-extension").unwrap();
        let resolver = ExtensionAdminConfigurationResolver::new(
            service,
            sample_scope(),
            [manifest_with_descriptors(
                extension_id.as_str(),
                vec![first, second],
            )],
        );

        assert_eq!(
            resolver
                .effective_non_secret_config(&extension_id)
                .await
                .unwrap_err(),
            ExtensionAdminConfigurationResolverError::AmbiguousFieldHandle {
                handle: "shared_client_id".to_string(),
            }
        );
    }

    fn descriptor(
        group_id: &str,
        client_id_handle: &str,
        client_secret_handle: &str,
    ) -> ExtensionAdminConfigurationDescriptor {
        ExtensionAdminConfigurationDescriptor {
            group_id: AdminConfigurationGroupId::new(group_id).unwrap(),
            display_name: format!("{group_id} deployment configuration"),
            description: String::new(),
            fields: vec![
                AdminConfigurationField {
                    handle: SecretHandle::new(client_id_handle).unwrap(),
                    label: "OAuth client ID".to_string(),
                    secret: false,
                    required: true,
                },
                AdminConfigurationField {
                    handle: SecretHandle::new(client_secret_handle).unwrap(),
                    label: "OAuth client secret".to_string(),
                    secret: true,
                    required: true,
                },
            ],
        }
    }

    fn manifest(
        id: &str,
        descriptor: ExtensionAdminConfigurationDescriptor,
    ) -> Arc<ResolvedExtensionManifest> {
        manifest_with_descriptors(id, vec![descriptor])
    }

    fn manifest_with_descriptors(
        id: &str,
        descriptors: Vec<ExtensionAdminConfigurationDescriptor>,
    ) -> Arc<ResolvedExtensionManifest> {
        Arc::new(ResolvedExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION_V3.to_string(),
            id: ExtensionId::new(id).unwrap(),
            name: id.to_string(),
            version: "0.1.0".to_string(),
            description: "resolver fixture".to_string(),
            requested_trust: RequestedTrustClass::ThirdParty,
            runtime: ExtensionRuntimeV2::FirstParty {
                service: format!("{id}.extension/v1"),
            },
            mcp: None,
            tools: Vec::new(),
            channel: None,
            admin_configuration: descriptors,
            auth: Vec::new(),
            host_apis: Vec::new(),
            section_surfaces: Vec::new(),
            hooks: Vec::new(),
        })
    }

    fn submitted(handle: &str, value: &str) -> AdminConfigurationSubmittedValue {
        AdminConfigurationSubmittedValue {
            handle: SecretHandle::new(handle).unwrap(),
            value: SecretMaterial::from(value.to_string()),
        }
    }

    fn scoped_admin_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/extension-admin-configuration").unwrap(),
            VirtualPath::new("/engine/tenants/test/admin-configuration").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        ))
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("operator-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}
