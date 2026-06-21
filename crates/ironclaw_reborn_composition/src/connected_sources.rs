use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, HostApiError, ResourceUsage, RuntimeDispatchErrorKind};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product_workflow::{
    ConnectorReadError, ConnectorReadPort, RebornConnectorReadRequest,
};
use serde::Deserialize;
use serde_json::{Map, Value};

const CONNECTED_SOURCES_READ_CAPABILITY_ID: &str = "connected-sources.read";

pub(crate) fn register_bundled_connected_sources_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    connector_port: Arc<dyn ConnectorReadPort>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(CONNECTED_SOURCES_READ_CAPABILITY_ID)?,
        Arc::new(ConnectedSourcesReadHandler { connector_port }),
    );
    Ok(())
}

struct ConnectedSourcesReadHandler {
    connector_port: Arc<dyn ConnectorReadPort>,
}

#[derive(Debug, Deserialize)]
struct ConnectedSourcesReadInput {
    toolkit: String,
    tool: String,
    #[serde(default = "empty_object")]
    arguments: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[async_trait]
impl FirstPartyCapabilityHandler for ConnectedSourcesReadHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        if request.capability_id.as_str() != CONNECTED_SOURCES_READ_CAPABILITY_ID {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            ));
        }
        let input = serde_json::from_value::<ConnectedSourcesReadInput>(request.input).map_err(
            |error| {
                FirstPartyCapabilityError::with_safe_summary(
                    RuntimeDispatchErrorKind::InputEncode,
                    bounded_summary("invalid connected sources read input", &error.to_string()),
                )
            },
        )?;
        let response = self
            .connector_port
            .read(RebornConnectorReadRequest {
                toolkit: input.toolkit,
                tool: input.tool,
                arguments: input.arguments,
            })
            .await
            .map_err(connector_error_to_first_party)?;
        let output = serde_json::to_value(response)
            .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            ResourceUsage::default(),
        ))
    }
}

fn connector_error_to_first_party(error: ConnectorReadError) -> FirstPartyCapabilityError {
    match error {
        ConnectorReadError::InvalidRequest { reason } => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::Client,
                bounded_summary("connected sources rejected the read request", &reason),
            )
        }
        ConnectorReadError::Unavailable { retryable: false } => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::SecretDenied,
                "connected sources are not configured for this profile",
            )
        }
        ConnectorReadError::Unavailable { retryable: true } => {
            FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::Backend,
                "connected sources are temporarily unavailable",
            )
        }
        ConnectorReadError::Upstream { message } => FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Backend,
            bounded_summary("connected source provider returned an error", &message),
        ),
        ConnectorReadError::Internal => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
    }
}

fn bounded_summary(prefix: &str, detail: &str) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return prefix.to_string();
    }
    let mut summary = format!("{prefix}: {trimmed}");
    summary.truncate(240);
    summary
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use ironclaw_product_workflow::{
        RebornConnectedAccountsResponse, RebornConnectorReadResponse, RebornConnectorWriteRequest,
    };
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct RecordingConnector {
        reads: Mutex<Vec<RebornConnectorReadRequest>>,
        response: Mutex<Option<Result<RebornConnectorReadResponse, ConnectorReadError>>>,
    }

    #[async_trait]
    impl ConnectorReadPort for RecordingConnector {
        async fn connected(&self) -> Result<RebornConnectedAccountsResponse, ConnectorReadError> {
            Ok(RebornConnectedAccountsResponse {
                accounts: Vec::new(),
            })
        }

        async fn read(
            &self,
            request: RebornConnectorReadRequest,
        ) -> Result<RebornConnectorReadResponse, ConnectorReadError> {
            self.reads.lock().expect("reads lock").push(request);
            self.response
                .lock()
                .expect("response lock")
                .clone()
                .unwrap_or_else(|| {
                    Ok(RebornConnectorReadResponse {
                        successful: true,
                        data: json!({ "messages": [{ "subject": "Redacted" }] }),
                        error: None,
                    })
                })
        }

        async fn write(
            &self,
            _request: RebornConnectorWriteRequest,
        ) -> Result<RebornConnectorReadResponse, ConnectorReadError> {
            Err(ConnectorReadError::InvalidRequest {
                reason: "write path must not be reachable".to_string(),
            })
        }

        async fn configure_secrets(
            &self,
            _secrets: HashMap<String, String>,
        ) -> Result<(), ConnectorReadError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn handler_delegates_to_read_only_connector_port() {
        let connector = Arc::new(RecordingConnector::default());
        let connector_port: Arc<dyn ConnectorReadPort> = connector.clone();
        let mut registry = FirstPartyCapabilityRegistry::new();
        register_bundled_connected_sources_first_party_handlers(&mut registry, connector_port)
            .expect("handler registers");
        let capability_id = CapabilityId::new(CONNECTED_SOURCES_READ_CAPABILITY_ID).unwrap();
        let handler = registry.get(&capability_id).expect("handler installed");

        let result = handler
            .dispatch(FirstPartyCapabilityRequest::request_for_test(
                capability_id,
                ironclaw_host_api::ResourceScope::system(),
                json!({
                    "toolkit": "gmail",
                    "tool": "GMAIL_FETCH_EMAILS",
                    "arguments": { "max_results": 1 }
                }),
                None,
            ))
            .await
            .expect("dispatch succeeds");

        assert_eq!(result.output["successful"], true);
        let reads = connector.reads.lock().expect("reads lock");
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].toolkit, "gmail");
        assert_eq!(reads[0].tool, "GMAIL_FETCH_EMAILS");
        assert_eq!(reads[0].arguments, json!({ "max_results": 1 }));
    }

    #[tokio::test]
    async fn handler_maps_connector_rejections_to_client_errors() {
        let connector = Arc::new(RecordingConnector::default());
        *connector.response.lock().expect("response lock") =
            Some(Err(ConnectorReadError::InvalidRequest {
                reason: "tool 'GMAIL_SEND_EMAIL' is not on the read-only allowlist".to_string(),
            }));
        let connector_port: Arc<dyn ConnectorReadPort> = connector;
        let mut registry = FirstPartyCapabilityRegistry::new();
        register_bundled_connected_sources_first_party_handlers(&mut registry, connector_port)
            .expect("handler registers");
        let capability_id = CapabilityId::new(CONNECTED_SOURCES_READ_CAPABILITY_ID).unwrap();
        let handler = registry.get(&capability_id).expect("handler installed");

        let error = handler
            .dispatch(FirstPartyCapabilityRequest::request_for_test(
                capability_id,
                ironclaw_host_api::ResourceScope::system(),
                json!({
                    "toolkit": "gmail",
                    "tool": "GMAIL_SEND_EMAIL",
                    "arguments": {}
                }),
                None,
            ))
            .await
            .expect_err("write-like read is rejected");

        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Client));
        assert!(
            error
                .safe_summary()
                .expect("safe summary")
                .contains("read-only allowlist")
        );
    }
}
