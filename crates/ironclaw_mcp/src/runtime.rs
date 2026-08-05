//! Resource-governed execution of a manifest-declared MCP capability.
//!
//! This module is the lane's authority boundary: it admits the descriptor
//! against the package the caller projected, reserves against the host budget,
//! calls the configured [`McpClient`], and reconciles or releases — never both.
//! It also assembles the manifest credential context an authentication failure
//! reports back, which is the only place the lane reads
//! `RuntimeCredentialRequirement`. It speaks no protocol and sends no bytes.

use async_trait::async_trait;
use ironclaw_extension_contracts::runtime::ExtensionRuntime;
use ironclaw_host_api::{
    capability::{RuntimeCredentialRequirement, RuntimeCredentialRequirementSource},
    decision::RuntimeCredentialAuthRequirement,
    ids::{ExtensionId, ResourceReservationId, SecretHandle},
    resource::{
        CapabilityHostResult, ResourceEstimate, ResourceReservation, ResourceScope,
        RuntimeResourceBudget, RuntimeResourceError,
    },
    runtime::RuntimeKind,
};

use crate::contract::{
    McpClient, McpClientError, McpClientRequest, McpError, McpExecutionRequest, McpExecutionResult,
    McpExecutor, McpRuntimeConfig,
};
use crate::egress::requires_host_http_egress;

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpAuthContext {
    required_secrets: Vec<SecretHandle>,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
}

#[derive(Debug)]
struct PreparedMcpClientRequest {
    request: McpClientRequest,
    auth_context: McpAuthContext,
}

/// Runtime for executing manifest-declared MCP capabilities through a host adapter.
#[derive(Debug, Clone)]
pub struct McpRuntime<C> {
    config: McpRuntimeConfig,
    client: C,
}

impl<C> McpRuntime<C>
where
    C: McpClient,
{
    pub fn new(config: McpRuntimeConfig, client: C) -> Self {
        Self { config, client }
    }

    pub fn config(&self) -> &McpRuntimeConfig {
        &self.config
    }

    pub async fn execute_extension_json<Budget>(
        &self,
        budget: &Budget,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError>
    where
        Budget: RuntimeResourceBudget + ?Sized,
    {
        let client_request = self.prepare_client_request(&request)?;
        let auth_context = client_request.auth_context;
        let client_request = client_request.request;
        let transport = client_request.transport.clone();
        if requires_host_http_egress(&transport) && !self.client.uses_host_mediated_http_egress() {
            return Err(McpError::HostHttpEgressRequired { transport });
        }
        let reservation = reserve_or_use_existing(
            budget,
            request.scope.clone(),
            request.estimate.clone(),
            request.resource_reservation.clone(),
        )?;

        let output = match self.client.call_tool(client_request).await {
            Ok(output) => output,
            Err(error) => {
                return Err(release_after_failure(
                    budget,
                    reservation.id,
                    mcp_error_from_client_error(error, auth_context),
                ));
            }
        };

        let serialized_len = serde_json::to_vec(&output.output)
            .map_err(|error| {
                release_after_failure(
                    budget,
                    reservation.id,
                    McpError::InvalidInvocation {
                        reason: error.to_string(),
                    },
                )
            })?
            .len() as u64;
        let output_bytes = output
            .output_bytes
            .unwrap_or(serialized_len)
            .max(serialized_len);
        if output_bytes > self.config.max_output_bytes {
            return Err(release_after_failure(
                budget,
                reservation.id,
                McpError::OutputLimitExceeded {
                    limit: self.config.max_output_bytes,
                    actual: output_bytes,
                },
            ));
        }

        let mut usage = output.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        if transport == "stdio" {
            usage.process_count = usage.process_count.max(1);
        }
        let receipt = budget.reconcile(reservation.id, usage.clone())?;
        Ok(McpExecutionResult {
            result: CapabilityHostResult {
                output: output.output,
                reservation_id: reservation.id,
                usage,
                output_bytes,
            },
            receipt,
        })
    }

    fn prepare_client_request(
        &self,
        request: &McpExecutionRequest<'_>,
    ) -> Result<PreparedMcpClientRequest, McpError> {
        let descriptor = request
            .capabilities
            .iter()
            .find(|descriptor| &descriptor.id == request.capability_id)
            .cloned()
            .ok_or_else(|| McpError::CapabilityNotDeclared {
                capability: request.capability_id.clone(),
            })?;

        if descriptor.runtime != RuntimeKind::Mcp {
            return Err(McpError::ExtensionRuntimeMismatch {
                extension: request.extension.clone(),
                actual: descriptor.runtime,
            });
        }
        if descriptor.provider != *request.extension {
            return Err(McpError::DescriptorMismatch {
                reason: format!(
                    "descriptor {} provider {} does not match package {}",
                    descriptor.id, descriptor.provider, *request.extension
                ),
            });
        }

        let (transport, command, args, url) = match request.runtime {
            ExtensionRuntime::Mcp {
                transport,
                command,
                args,
                url,
            } => (transport, command, args, url),
            other => {
                return Err(McpError::ExtensionRuntimeMismatch {
                    extension: request.extension.clone(),
                    actual: other.kind(),
                });
            }
        };

        if transport == "stdio" {
            return Err(McpError::ExternalStdioTransportUnsupported);
        }
        if !matches!(transport.as_str(), "http" | "sse") {
            return Err(McpError::UnsupportedTransport {
                transport: transport.clone(),
            });
        }
        if matches!(transport.as_str(), "http" | "sse") && url.is_none() {
            return Err(McpError::InvalidInvocation {
                reason: format!("{transport} MCP transport requires a manifest url"),
            });
        }

        let auth_context = mcp_auth_context(&descriptor.provider, &descriptor.runtime_credentials);

        Ok(PreparedMcpClientRequest {
            request: McpClientRequest {
                provider: request.extension.clone(),
                capability_id: request.capability_id.clone(),
                scope: request.scope.clone(),
                transport: transport.clone(),
                command: command.clone(),
                args: args.clone(),
                url: url.clone(),
                input: request.invocation.input.clone(),
                max_output_bytes: self.config.max_output_bytes,
            },
            auth_context,
        })
    }
}

fn mcp_error_from_client_error(error: McpClientError, auth_context: McpAuthContext) -> McpError {
    match error {
        McpClientError::Client { reason } => McpError::Client { reason },
        McpClientError::InvalidToolCatalog { reason } => McpError::InvalidToolCatalog { reason },
        McpClientError::AuthRequired | McpClientError::AuthChallenge { .. } => {
            McpError::AuthRequired {
                required_secrets: auth_context.required_secrets,
                credential_requirements: auth_context.credential_requirements,
            }
        }
    }
}

fn mcp_auth_context(
    requester_extension: &ExtensionId,
    credentials: &[RuntimeCredentialRequirement],
) -> McpAuthContext {
    let mut required_secrets = Vec::new();
    let mut credential_requirements = Vec::new();
    for credential in credentials.iter().filter(|credential| credential.required) {
        match &credential.source {
            RuntimeCredentialRequirementSource::SecretHandle => {
                required_secrets.push(credential.handle.clone());
            }
            RuntimeCredentialRequirementSource::ProductAuthAccount { .. } => {
                if let Some(requirement) =
                    credential.product_auth_requirement_for(requester_extension.clone())
                {
                    credential_requirements.push(requirement);
                }
            }
        }
    }
    McpAuthContext {
        required_secrets,
        credential_requirements,
    }
}

#[async_trait]
impl<C> McpExecutor for McpRuntime<C>
where
    C: McpClient,
{
    async fn execute_extension_json(
        &self,
        budget: &dyn RuntimeResourceBudget,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        McpRuntime::execute_extension_json(self, budget, request).await
    }
}

fn reserve_or_use_existing<Budget>(
    budget: &Budget,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation: Option<ResourceReservation>,
) -> Result<ResourceReservation, McpError>
where
    Budget: RuntimeResourceBudget + ?Sized,
{
    if let Some(reservation) = reservation {
        if reservation.scope != scope || reservation.estimate != estimate {
            return Err(McpError::Resource(
                RuntimeResourceError::reservation_mismatch(reservation.id),
            ));
        }
        return Ok(reservation);
    }
    budget.reserve(scope, estimate).map_err(McpError::from)
}

fn release_after_failure<Budget>(
    budget: &Budget,
    reservation_id: ResourceReservationId,
    original: McpError,
) -> McpError
where
    Budget: RuntimeResourceBudget + ?Sized,
{
    let _ = budget.release(reservation_id);
    original
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_auth_context_preserves_product_auth_oauth_setup() {
        let scopes = vec!["https://www.googleapis.com/auth/drive.readonly".to_string()];
        let credential = RuntimeCredentialRequirement {
            handle: SecretHandle::new("google-drive-access").unwrap(),
            source: RuntimeCredentialRequirementSource::ProductAuthAccount {
                provider: ironclaw_host_api::ids::VendorId::new("google").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes: scopes.clone(),
                },
            },
            provider_scopes: scopes.clone(),
            audience: ironclaw_host_api::action::NetworkTargetPattern {
                scheme: None,
                host_pattern: "*".to_string(),
                port: None,
            },
            target: ironclaw_host_api::http::RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        };

        let context = mcp_auth_context(&ExtensionId::new("google-drive").unwrap(), &[credential]);

        assert!(context.required_secrets.is_empty());
        assert_eq!(
            context.credential_requirements,
            vec![RuntimeCredentialAuthRequirement {
                provider: ironclaw_host_api::ids::VendorId::new("google").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes
                },
                requester_extension: ExtensionId::new("google-drive").unwrap(),
                provider_scopes: vec!["https://www.googleapis.com/auth/drive.readonly".to_string()],
            }]
        );
    }
}
