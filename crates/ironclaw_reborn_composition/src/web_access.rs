use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_first_party_extensions::{
    WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID, WebAccessDispatchError,
    WebAccessDispatchRequest, WebAccessExecutor,
};
use ironclaw_host_api::{CapabilityId, HostApiError};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub(crate) fn register_bundled_web_access_first_party_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    let handler = Arc::new(WebAccessFirstPartyHandler {
        executor: WebAccessExecutor::default(),
    });
    registry.insert_handler(
        CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    registry.insert_handler(
        CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    Ok(())
}

struct WebAccessFirstPartyHandler {
    executor: WebAccessExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for WebAccessFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(WebAccessDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(web_access_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn web_access_error(error: WebAccessDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    if let Some(usage) = error.usage().cloned() {
        mapped.with_usage(usage)
    } else {
        mapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        InvocationId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, UserId,
    };
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    struct RecordingEgress {
        responses: Mutex<VecDeque<RuntimeHttpEgressResponse>>,
        requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
    }

    impl RecordingEgress {
        fn for_mcp_fetch() -> Self {
            Self {
                responses: Mutex::new(
                    [
                        Self::json_response(json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {"protocolVersion": "2024-11-05", "capabilities": {}}
                        })),
                        RuntimeHttpEgressResponse {
                            status: 202,
                            headers: Vec::new(),
                            body: Vec::new(),
                            saved_body: None,
                            request_bytes: 5,
                            response_bytes: 0,
                            redaction_applied: false,
                        },
                        Self::json_response(json!({
                            "result": {"content": [{"type": "text", "text": "# Example Domain\nURL: https://example.com\n\nExample body"}]}
                        })),
                    ]
                    .into_iter()
                    .collect(),
                ),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn json_response(body: Value) -> RuntimeHttpEgressResponse {
            let body = serde_json::to_vec(&body).unwrap();
            RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body,
                saved_body: None,
                request_bytes: 10,
                response_bytes: 20,
                redaction_applied: false,
            }
        }

        fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeHttpEgress for RecordingEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.requests.lock().unwrap().push(request);
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("RecordingEgress: no more responses queued"))
        }
    }

    fn scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .unwrap()
    }

    #[tokio::test]
    async fn bundled_web_access_get_content_handler_forwards_runtime_egress() {
        let mut registry = FirstPartyCapabilityRegistry::default();
        register_bundled_web_access_first_party_handlers(&mut registry).unwrap();
        let capability_id = CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID).unwrap();
        let egress = Arc::new(RecordingEgress::for_mcp_fetch());
        let egress_port: Arc<dyn RuntimeHttpEgress> = egress.clone();
        let handler = registry.get(&capability_id).expect("handler registered");

        let result = handler
            .dispatch(FirstPartyCapabilityRequest::request_for_test(
                capability_id.clone(),
                scope(),
                json!({"url": "https://example.com"}),
                Some(egress_port),
            ))
            .await
            .unwrap();

        assert_eq!(result.output["provider_used"], "exa_mcp");
        assert_eq!(result.output["url"], "https://example.com");
        let requests = egress.requests();
        assert_eq!(requests.len(), 3);
        assert!(
            requests
                .iter()
                .all(|request| request.capability_id == capability_id)
        );
        let tools_call = serde_json::from_slice::<Value>(&requests[2].body).unwrap();
        assert_eq!(tools_call["params"]["name"], "web_fetch_exa");
    }
}
