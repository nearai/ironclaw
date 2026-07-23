//! Authorized first-party mutation for manifest-declared administrator configuration.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_extension_host::{
    AdminConfigurationGroupState, AdminConfigurationIdempotencyKey, AdminConfigurationServiceError,
    AdminConfigurationSubmittedValue, reconcile_admin_configuration_consumers,
};
use ironclaw_extensions::{
    AdminConfigurationGroupId, CapabilityManifest, CapabilityVisibility, ExtensionError,
    ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId, HostApiError,
    OriginGateMatrix, OriginGatePolicy, PermissionMode, ResourceEstimate, ResourceProfile,
    ResourceUsage, RuntimeDispatchErrorKind, SecretHandle, UserId,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID;
use ironclaw_secrets::SecretMaterial;
use serde::Deserialize;

use crate::extension_host::admin_configuration::ComposedAdminConfigurationService;
use crate::extension_host::extension_lifecycle::ExtensionManagementPort;

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    service: Arc<ComposedAdminConfigurationService>,
    operator_user_id: UserId,
    affected_extensions: BTreeMap<AdminConfigurationGroupId, BTreeSet<ExtensionId>>,
    extension_management: Arc<ExtensionManagementPort>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID)?,
        Arc::new(AdminConfigurationReplaceHandler {
            service,
            operator_user_id,
            affected_extensions,
            extension_management,
        }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID)?,
        implements: Vec::new(),
        description: "Replace one manifest-declared tenant administrator configuration group through an authenticated operator gesture.".to_string(),
        effects: vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::UseSecret,
        ],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/admin_configuration_replace.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/admin_configuration_replace.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(64 * 1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(OriginGateMatrix {
            loop_run: OriginGatePolicy::Forbidden,
            product: OriginGatePolicy::ConsentSufficient,
            automation: OriginGatePolicy::Forbidden,
        }),
    })
}

struct AdminConfigurationReplaceHandler {
    service: Arc<ComposedAdminConfigurationService>,
    operator_user_id: UserId,
    affected_extensions: BTreeMap<AdminConfigurationGroupId, BTreeSet<ExtensionId>>,
    extension_management: Arc<ExtensionManagementPort>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceInput {
    group_id: String,
    expected_revision: u64,
    values: Vec<SubmittedValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedValue {
    handle: String,
    value: String,
}

#[async_trait]
impl FirstPartyCapabilityHandler for AdminConfigurationReplaceHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        if !is_operator_request(&request, &self.operator_user_id) {
            return Err(
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::PolicyDenied)
                    .with_usage(resource_usage(started)),
            );
        }
        if request.capability_id.as_str() != ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )
            .with_usage(resource_usage(started)));
        }

        let input: ReplaceInput = serde_json::from_value(request.input).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
                .with_usage(resource_usage(started))
        })?;
        let group_id = AdminConfigurationGroupId::new(input.group_id).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
                .with_usage(resource_usage(started))
        })?;
        let idempotency_key =
            AdminConfigurationIdempotencyKey::new(request.scope.invocation_id.to_string())
                .map_err(|_| {
                    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
                        .with_usage(resource_usage(started))
                })?;
        let submitted = input
            .values
            .into_iter()
            .map(|value| {
                Ok(AdminConfigurationSubmittedValue {
                    handle: SecretHandle::new(value.handle).map_err(|_| {
                        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
                            .with_usage(resource_usage(started))
                    })?,
                    value: SecretMaterial::from(value.value),
                })
            })
            .collect::<Result<Vec<_>, FirstPartyCapabilityError>>()?;
        let reconcile = || async {
            let Some(extension_ids) = self.affected_extensions.get(&group_id) else {
                return Ok(());
            };
            reconcile_admin_configuration_consumers(&group_id, extension_ids, |extension_id| {
                let extension_management = Arc::clone(&self.extension_management);
                async move {
                    extension_management
                        .reconcile_runtime_after_admin_configuration(&extension_id)
                        .await
                }
            })
            .await
        };
        let state = self
            .service
            .replace_with_reconcile(
                &request.scope,
                &group_id,
                &idempotency_key,
                input.expected_revision,
                submitted,
                reconcile,
            )
            .await
            .map_err(|error| map_service_error(error, started))?;
        let output = render_state(state);
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

fn is_operator_request(request: &FirstPartyCapabilityRequest, operator_user_id: &UserId) -> bool {
    request.authenticated_actor_user_id.as_ref() == Some(operator_user_id)
        && &request.scope.user_id == operator_user_id
}

fn render_state(state: AdminConfigurationGroupState) -> serde_json::Value {
    serde_json::json!({
        "group_id": state.group_id.as_str(),
        "revision": state.revision,
        "complete": state.complete,
        "fields": state.fields.into_iter().map(|field| {
            let value = if field.secret { None } else { field.value };
            serde_json::json!({
                "handle": field.handle.as_str(),
                "secret": field.secret,
                "required": field.required,
                "provided": field.provided,
                "value": value,
            })
        }).collect::<Vec<_>>(),
    })
}

fn map_service_error(
    error: AdminConfigurationServiceError,
    started: Instant,
) -> FirstPartyCapabilityError {
    let kind = match error {
        AdminConfigurationServiceError::UnknownGroup
        | AdminConfigurationServiceError::UnknownField
        | AdminConfigurationServiceError::DuplicateField
        | AdminConfigurationServiceError::MissingRequiredField
        | AdminConfigurationServiceError::ValueTooLarge => RuntimeDispatchErrorKind::InputEncode,
        AdminConfigurationServiceError::RevisionConflict { .. }
        | AdminConfigurationServiceError::IdempotencyConflict => {
            RuntimeDispatchErrorKind::OperationFailed
        }
        AdminConfigurationServiceError::RuntimeReconciliationFailed
        | AdminConfigurationServiceError::RuntimeRollbackFailed => {
            RuntimeDispatchErrorKind::OperationFailed
        }
        AdminConfigurationServiceError::InvalidDescriptor
        | AdminConfigurationServiceError::DescriptorConflict => RuntimeDispatchErrorKind::Manifest,
        AdminConfigurationServiceError::Unavailable => RuntimeDispatchErrorKind::Backend,
    };
    tracing::warn!(error = %error, "admin-configuration replacement failed");
    FirstPartyCapabilityError::new(kind).with_usage(resource_usage(started))
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use ironclaw_extension_host::{AdminConfigurationFieldState, AdminConfigurationGroupState};
    use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId};

    use super::*;

    #[test]
    fn capability_is_api_only_and_operator_gated() {
        assert_eq!(manifest().unwrap().visibility, CapabilityVisibility::Api);

        let operator = UserId::new("operator").unwrap();
        let member = UserId::new("member").unwrap();
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant").unwrap(),
            user_id: operator.clone(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID).unwrap(),
            scope,
            serde_json::json!({}),
            None,
        );
        request.authenticated_actor_user_id = Some(member);
        assert!(!is_operator_request(&request, &operator));
        request.authenticated_actor_user_id = Some(operator.clone());
        assert!(is_operator_request(&request, &operator));
    }

    #[test]
    fn capability_output_never_serializes_secret_field_values() {
        let sentinel = "secret-sentinel-never-serialize";
        let output = render_state(AdminConfigurationGroupState {
            group_id: AdminConfigurationGroupId::new("extension.fixture").unwrap(),
            display_name: "Fixture".to_string(),
            description: String::new(),
            revision: 1,
            complete: true,
            fields: vec![AdminConfigurationFieldState {
                handle: SecretHandle::new("fixture_token").unwrap(),
                label: "Token".to_string(),
                secret: true,
                required: true,
                provided: true,
                value: Some(sentinel.to_string()),
            }],
        });
        assert!(!output.to_string().contains(sentinel));
        assert!(output["fields"][0]["value"].is_null());
    }
}
