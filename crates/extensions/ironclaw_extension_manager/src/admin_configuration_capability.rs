//! Authorized first-party mutation for manifest-declared administrator configuration.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_assistant::ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID;
use ironclaw_extension_host::{
    AdminConfigurationGroupState, AdminConfigurationIdempotencyKey, AdminConfigurationServiceError,
    AdminConfigurationSubmittedValue, ChannelConfigReactivation,
};
use ironclaw_extension_registry::{
    AdminConfigurationGroupId, CapabilityManifest, CapabilityVisibility, ExtensionError,
    ExtensionPackage,
};
use ironclaw_host_api::{
    capability::{EffectKind, OriginGateMatrix, OriginGatePolicy, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::{CapabilityId, ExtensionId, SecretHandle, UserId},
    resource::{ResourceEstimate, ResourceProfile, ResourceUsage},
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_secrets::SecretMaterial;
use serde::Deserialize;

use crate::admin_configuration::ComposedAdminConfigurationService;

pub fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    let root = package
        .materialized_root()
        .map_err(|error| ExtensionError::InvalidManifest {
            reason: format!("built-in package requires a materialized root: {error}"),
        })?
        .clone();
    ExtensionPackage::from_manifest(package.manifest, root)
}

pub fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    service: Arc<ComposedAdminConfigurationService>,
    operator_user_id: UserId,
    reactivation: Arc<dyn ChannelConfigReactivation>,
    consumers: BTreeMap<AdminConfigurationGroupId, BTreeSet<ExtensionId>>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID)?,
        Arc::new(AdminConfigurationReplaceHandler {
            service,
            operator_user_id,
            reactivation,
            consumers: Arc::new(consumers),
        }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID)?,
        description: "Replace one manifest-declared tenant administrator configuration group through an authenticated operator gesture.".to_string(),
        effects: vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::UseSecret,
        ],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        standard_op: None,
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
    reactivation: Arc<dyn ChannelConfigReactivation>,
    consumers: Arc<BTreeMap<AdminConfigurationGroupId, BTreeSet<ExtensionId>>>,
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

        let input: ReplaceInput = serde_json::from_value(request.input)
            .map_err(|error| rejected_input(started, "input", error))?;
        let group_id = AdminConfigurationGroupId::new(input.group_id)
            .map_err(|error| rejected_input(started, "group_id", error))?;
        let idempotency_key =
            AdminConfigurationIdempotencyKey::new(request.scope.invocation_id.to_string())
                .map_err(|error| rejected_input(started, "idempotency_key", error))?;
        let submitted = input
            .values
            .into_iter()
            .map(|value| {
                Ok(AdminConfigurationSubmittedValue {
                    handle: SecretHandle::new(value.handle)
                        .map_err(|error| rejected_input(started, "values[].handle", error))?,
                    value: SecretMaterial::from(value.value),
                })
            })
            .collect::<Result<Vec<_>, FirstPartyCapabilityError>>()?;
        let state = self
            .service
            .replace(
                &request.scope,
                &group_id,
                &idempotency_key,
                input.expected_revision,
                submitted,
            )
            .await
            .map_err(|error| map_service_error(error, started))?;
        self.reactivate_consumers(&group_id, started).await?;
        let output = render_state(state);
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

impl AdminConfigurationReplaceHandler {
    /// Refresh every installed extension that consumes the group.
    ///
    /// **This runs after the replacement is already durable.** `dispatch`
    /// awaits `service.replace(..)` first, so a failure here reports
    /// `OperationFailed` over a group whose new values *are* committed — the
    /// caller must re-read rather than assume the write was rolled back.
    ///
    /// That asymmetry is deliberate, not an unhandled persist-then-reload gap.
    /// Neither escape hatch fits: the replacement cannot be pre-validated
    /// (whether an extension reactivates is only knowable by attempting it),
    /// and rolling back would mean re-writing the previous secret values — a
    /// second mutation of the same revision-guarded group, which is a worse
    /// failure than a stale-but-live extension. What makes the exposure
    /// bounded is that the group is revision- and idempotency-guarded, so the
    /// caller's retry re-runs this refresh without re-applying the write.
    async fn reactivate_consumers(
        &self,
        group_id: &AdminConfigurationGroupId,
        started: Instant,
    ) -> Result<(), FirstPartyCapabilityError> {
        let Some(consumers) = self.consumers.get(group_id) else {
            return Ok(());
        };
        for extension_id in consumers {
            self.reactivation
                .reactivate_if_active(extension_id)
                .await
                .map_err(|error| {
                    tracing::warn!(
                        %error,
                        extension_id = %extension_id,
                        group_id = %group_id,
                        "admin-configuration replacement could not refresh active extension"
                    );
                    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                        .with_usage(resource_usage(started))
                })?;
        }
        Ok(())
    }
}

/// Reject one malformed piece of the replace input without discarding why.
///
/// `RuntimeDispatchErrorKind::InputEncode` carries no reason, and the operator
/// gets nothing but that kind — so the parse/validation cause has nowhere to go
/// except a log, and dropping the binding (`map_err(|_| …)`) would lose it for
/// good. `debug!`, not `warn!`/`info!`: those corrupt the REPL/TUI, and a
/// malformed payload is an internal diagnostic rather than operator-facing
/// status.
///
/// Safe to record: the three validation causes name an identifier
/// (`group_id`, the invocation-derived idempotency key, a field `handle`) and
/// `serde_json`'s message names *fields* — `missing field \`group_id\``,
/// `unknown field \`x\``. It never echoes a well-typed string, so a submitted
/// secret value (always a JSON string here, and always routed to
/// `SecretMaterial`) cannot reach this line.
fn rejected_input(
    started: Instant,
    field: &'static str,
    error: impl std::fmt::Display,
) -> FirstPartyCapabilityError {
    tracing::debug!(
        error = %error,
        field,
        "admin-configuration replacement rejected malformed input"
    );
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
        .with_usage(resource_usage(started))
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
    use ironclaw_extension_host::{
        AdminConfigurationFieldState, AdminConfigurationGroupState, ChannelConfigReactivationError,
        FilesystemAdminConfigurationStore,
    };
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{
        ids::{AgentId, InvocationId, TenantId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use ironclaw_secrets::{SecretStore, SecretStorePort};

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

    #[derive(Clone, Default)]
    struct SharedLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    struct SharedLogWriterGuard(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriterGuard(std::sync::Arc::clone(&self.0))
        }
    }

    impl SharedLogWriter {
        fn contents(&self) -> String {
            String::from_utf8(
                self.0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            )
            .expect("tracing output is UTF-8")
        }
    }

    struct NoopReactivation;

    #[async_trait]
    impl ChannelConfigReactivation for NoopReactivation {
        async fn reactivate_if_active(
            &self,
            _extension_id: &ExtensionId,
        ) -> Result<(), ChannelConfigReactivationError> {
            Ok(())
        }
    }

    /// A handler whose service declares no groups. Every case below is
    /// rejected while parsing the payload, so the service is never reached —
    /// which is the point: input rejection must not depend on backend state.
    fn replace_handler(operator_user_id: UserId) -> AdminConfigurationReplaceHandler {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::ephemeral());
        let service = ComposedAdminConfigurationService::new(
            FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                filesystem,
                |_scope| {
                    MountView::new(vec![MountGrant::new(
                        MountAlias::new("/extension-admin-configuration").expect("mount alias"),
                        VirtualPath::new("/tenants/test/shared/admin-configuration")
                            .expect("virtual path"),
                        MountPermissions::read_write_list_delete(),
                    )])
                },
            ))),
            secrets,
            Vec::new(),
        )
        .expect("admin configuration service");
        AdminConfigurationReplaceHandler {
            service: Arc::new(service),
            operator_user_id,
            reactivation: Arc::new(NoopReactivation),
            consumers: Arc::new(BTreeMap::new()),
        }
    }

    /// Each malformed shape is refused as `InputEncode`, and the reason it was
    /// refused survives in the log instead of being dropped on the floor.
    ///
    /// The dispatch error carries no reason field, so a `map_err(|_| …)` here
    /// destroys the only copy of the cause — the operator sees "input encode"
    /// and nothing, anywhere, says which field or why. That is why the log
    /// half is asserted, and why a subscriber has to be installed for the
    /// assertion to mean anything: `tracing` short-circuits on the null
    /// dispatcher, so without one the macro body never runs and this could not
    /// tell "recorded it" from "discarded it".
    ///
    /// The third case carries a secret sentinel behind an invalid handle, so
    /// the same test also pins that recording the *cause* never starts
    /// recording the submitted *value*.
    #[test]
    fn every_malformed_input_is_refused_as_input_encode_with_its_cause_recorded() {
        let secret_sentinel = "submitted-secret-never-logged";
        let operator = UserId::new("operator").expect("user id");
        let handler = replace_handler(operator.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        // `idempotency_key` has no case: it is derived from the scope's own
        // `InvocationId`, which is always well-formed, so that arm is
        // unreachable from any caller-supplied payload and defensive only.
        let cases = [
            (
                "input",
                "missing field",
                serde_json::json!({ "group_id": "extension.fixture" }),
            ),
            (
                "group_id",
                "not a dotted identifier",
                serde_json::json!({
                    "group_id": "Not A Group",
                    "expected_revision": 1,
                    "values": [],
                }),
            ),
            (
                "values[].handle",
                "not a valid secret handle",
                serde_json::json!({
                    "group_id": "extension.fixture",
                    "expected_revision": 1,
                    "values": [{ "handle": "Not A Handle!", "value": secret_sentinel }],
                }),
            ),
        ];

        for (field, why, input) in cases {
            let logs = SharedLogWriter::default();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_target(false)
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(logs.clone())
                .finish();
            let mut request = FirstPartyCapabilityRequest::request_for_test(
                CapabilityId::new(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID)
                    .expect("capability id"),
                ResourceScope {
                    tenant_id: TenantId::new("tenant").expect("tenant"),
                    user_id: operator.clone(),
                    agent_id: None,
                    project_id: None,
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                input,
                None,
            );
            request.authenticated_actor_user_id = Some(operator.clone());

            let error = tracing::subscriber::with_default(subscriber, || {
                runtime.block_on(handler.dispatch(request))
            })
            .err()
            .unwrap_or_else(|| panic!("{field} ({why}) must be refused"));
            assert_eq!(
                error.kind(),
                Some(RuntimeDispatchErrorKind::InputEncode),
                "{field} ({why}) must stay an input-encode refusal"
            );

            let logged = logs.contents();
            assert!(
                logged.contains(field),
                "{field} ({why}) must name the field it refused, got {logged:?}"
            );
            assert!(
                logged.contains("rejected malformed input"),
                "{field} ({why}) must keep the stable diagnostic, got {logged:?}"
            );
            assert!(
                !logged.contains(secret_sentinel),
                "a submitted value must never reach the diagnostic, got {logged:?}"
            );
        }
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
                description: String::new(),
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
