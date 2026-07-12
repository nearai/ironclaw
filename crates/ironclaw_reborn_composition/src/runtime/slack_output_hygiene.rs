use std::sync::Arc;

use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, HostManagedModelStreamSink,
};
use ironclaw_turns::{
    TurnScope,
    run_profile::{LoopCapabilityPort, ParentLoopOutput},
};

pub(super) const SLACK_IDENTIFIER_REDACTION: &str = "[Slack identifier redacted]";

#[derive(Clone)]
pub(super) struct SlackOutputHygieneGateway {
    inner: Arc<dyn HostManagedModelGateway>,
}

impl SlackOutputHygieneGateway {
    pub(super) fn new(inner: Arc<dyn HostManagedModelGateway>) -> Self {
        Self { inner }
    }
}

struct SlackOutputHygieneSink {
    inner: Arc<dyn HostManagedModelStreamSink>,
}

#[async_trait::async_trait]
impl HostManagedModelStreamSink for SlackOutputHygieneSink {
    async fn safe_text_update(&self, safe_text: String) {
        self.inner
            .safe_text_update(redact_slack_identifiers(&safe_text))
            .await;
    }
}

#[async_trait::async_trait]
impl HostManagedModelGateway for SlackOutputHygieneGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let slack_context = request_has_slack_context(&request);
        self.inner
            .stream_model(request)
            .await
            .map(|response| sanitize_response(response, slack_context))
    }

    async fn stream_model_with_progress(
        &self,
        request: HostManagedModelRequest,
        sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let slack_context = request_has_slack_context(&request);
        let sink = hygiene_sink(sink, slack_context);
        self.inner
            .stream_model_with_progress(request, sink)
            .await
            .map(|response| sanitize_response(response, slack_context))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let slack_context = request_has_slack_context(&request)
            || capabilities_have_slack_context(capabilities.as_ref());
        self.inner
            .stream_model_with_capabilities(request, capabilities)
            .await
            .map(|response| sanitize_response(response, slack_context))
    }

    async fn stream_model_with_capabilities_and_progress(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
        sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let slack_context = request_has_slack_context(&request)
            || capabilities_have_slack_context(capabilities.as_ref());
        let sink = hygiene_sink(sink, slack_context);
        self.inner
            .stream_model_with_capabilities_and_progress(request, capabilities, sink)
            .await
            .map(|response| sanitize_response(response, slack_context))
    }

    fn resolve_for_scope(&self, scope: &TurnScope) -> Option<Arc<dyn HostManagedModelGateway>> {
        self.inner
            .resolve_for_scope(scope)
            .map(|inner| Arc::new(Self::new(inner)) as Arc<dyn HostManagedModelGateway>)
    }
}

fn request_has_slack_context(request: &HostManagedModelRequest) -> bool {
    request.messages.iter().any(|message| {
        message
            .tool_result_provider_call
            .as_ref()
            .is_some_and(|provider_call| is_slack_capability(provider_call.capability_id.as_str()))
    })
}

fn capabilities_have_slack_context(capabilities: &dyn LoopCapabilityPort) -> bool {
    match capabilities.tool_definitions() {
        Ok(definitions) => definitions
            .iter()
            .any(|definition| is_slack_capability(definition.capability_id.as_str())),
        Err(_) => false,
    }
}

fn is_slack_capability(capability_id: &str) -> bool {
    capability_id.starts_with("slack.")
}

fn hygiene_sink(
    sink: Arc<dyn HostManagedModelStreamSink>,
    slack_context: bool,
) -> Arc<dyn HostManagedModelStreamSink> {
    if slack_context {
        Arc::new(SlackOutputHygieneSink { inner: sink })
    } else {
        sink
    }
}

fn sanitize_response(
    mut response: HostManagedModelResponse,
    slack_context: bool,
) -> HostManagedModelResponse {
    if !slack_context {
        return response;
    }
    response.safe_text_deltas = response
        .safe_text_deltas
        .into_iter()
        .map(|delta| redact_slack_identifiers(&delta))
        .collect();
    response.safe_reasoning_deltas = response
        .safe_reasoning_deltas
        .into_iter()
        .map(|delta| redact_slack_identifiers(&delta))
        .collect();
    if let ParentLoopOutput::AssistantReply(reply) = &mut response.output {
        reply.content = redact_slack_identifiers(&reply.content);
    }
    response
}

fn redact_slack_identifiers(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut redacted = String::with_capacity(text.len());
    let mut copy_from = 0;
    let mut cursor = 0;

    while cursor < bytes.len() {
        let mention = bytes[cursor] == b'<' && bytes.get(cursor + 1) == Some(&b'@');
        let identifier_start = if mention { cursor + 2 } else { cursor };
        let Some(identifier_end) = slack_identifier_end(bytes, identifier_start) else {
            cursor += 1;
            continue;
        };
        let replacement_end = if mention && bytes.get(identifier_end) == Some(&b'>') {
            identifier_end + 1
        } else {
            identifier_end
        };

        redacted.push_str(&text[copy_from..cursor]);
        redacted.push_str(SLACK_IDENTIFIER_REDACTION);
        copy_from = replacement_end;
        cursor = replacement_end;
    }

    redacted.push_str(&text[copy_from..]);
    redacted
}

fn slack_identifier_end(bytes: &[u8], start: usize) -> Option<usize> {
    let prefix = *bytes.get(start)?;
    if !matches!(prefix, b'U' | b'W')
        || start
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .is_some_and(|byte| is_ascii_token_byte(*byte))
    {
        return None;
    }

    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        end += 1;
    }
    let suffix_len = end - start - 1;
    if suffix_len < 8
        || bytes
            .get(end)
            .is_some_and(|byte| is_ascii_token_byte(*byte))
    {
        return None;
    }
    Some(end)
}

fn is_ascii_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ironclaw_host_api::{CapabilityId, ProviderToolName, TenantId, ThreadId};
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelMessage,
        HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
        HostManagedModelStreamSink,
    };
    use ironclaw_threads::ProviderToolCallReferenceEnvelope;
    use ironclaw_turns::run_profile::{
        AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
        CapabilityCallCandidate, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        CapabilitySurfaceVersion, LoopCapabilityPort, LoopModelUsage, ModelProfileId,
        ParentLoopOutput, ProviderToolCallReplay, ProviderToolDefinition, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    };
    use ironclaw_turns::{LoopMessageRef, TurnId, TurnRunId, TurnScope};

    use super::{SlackOutputHygieneGateway, redact_slack_identifiers};

    #[test]
    fn slack_output_hygiene_redacts_bounded_user_identifiers_and_mentions() {
        assert_eq!(
            redact_slack_identifiers("user U0123ABCDE"),
            "user [Slack identifier redacted]"
        );
        assert_eq!(
            redact_slack_identifiers("mention <@U0123ABCDE>"),
            "mention [Slack identifier redacted]"
        );
        assert_eq!(
            redact_slack_identifiers("legacy W0123ABCDE."),
            "legacy [Slack identifier redacted]."
        );
    }

    #[test]
    fn slack_output_hygiene_preserves_short_and_embedded_text_and_is_idempotent() {
        assert_eq!(
            redact_slack_identifiers("short U123 and word BUILDING"),
            "short U123 and word BUILDING"
        );
        let once = redact_slack_identifiers("U0123ABCDE");
        assert_eq!(redact_slack_identifiers(&once), once);
    }

    #[derive(Clone)]
    struct RecordingGateway {
        response: HostManagedModelResponse,
        progress: Vec<String>,
        requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
        methods: Arc<Mutex<Vec<&'static str>>>,
        resolved: Option<Arc<dyn HostManagedModelGateway>>,
    }

    impl RecordingGateway {
        fn new(response: HostManagedModelResponse) -> Self {
            Self {
                response,
                progress: Vec::new(),
                requests: Arc::new(Mutex::new(Vec::new())),
                methods: Arc::new(Mutex::new(Vec::new())),
                resolved: None,
            }
        }

        fn with_progress(mut self, progress: Vec<&str>) -> Self {
            self.progress = progress.into_iter().map(str::to_string).collect();
            self
        }

        fn with_resolved(mut self, resolved: Arc<dyn HostManagedModelGateway>) -> Self {
            self.resolved = Some(resolved);
            self
        }

        fn record(&self, method: &'static str, request: HostManagedModelRequest) {
            self.methods
                .lock()
                .expect("recording gateway methods lock")
                .push(method);
            self.requests
                .lock()
                .expect("recording gateway requests lock")
                .push(request);
        }

        async fn emit_progress(&self, sink: &dyn HostManagedModelStreamSink) {
            for update in &self.progress {
                sink.safe_text_update(update.clone()).await;
            }
        }
    }

    #[async_trait::async_trait]
    impl HostManagedModelGateway for RecordingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.record("stream_model", request);
            Ok(self.response.clone())
        }

        async fn stream_model_with_progress(
            &self,
            request: HostManagedModelRequest,
            sink: Arc<dyn HostManagedModelStreamSink>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.record("stream_model_with_progress", request);
            self.emit_progress(sink.as_ref()).await;
            Ok(self.response.clone())
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            _capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.record("stream_model_with_capabilities", request);
            Ok(self.response.clone())
        }

        async fn stream_model_with_capabilities_and_progress(
            &self,
            request: HostManagedModelRequest,
            _capabilities: Arc<dyn LoopCapabilityPort>,
            sink: Arc<dyn HostManagedModelStreamSink>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.record("stream_model_with_capabilities_and_progress", request);
            self.emit_progress(sink.as_ref()).await;
            Ok(self.response.clone())
        }

        fn resolve_for_scope(
            &self,
            _scope: &TurnScope,
        ) -> Option<Arc<dyn HostManagedModelGateway>> {
            self.resolved.clone()
        }
    }

    #[derive(Default)]
    struct RecordingStreamSink {
        updates: Mutex<Vec<String>>,
    }

    impl RecordingStreamSink {
        fn updates(&self) -> Vec<String> {
            self.updates
                .lock()
                .expect("recording stream sink lock")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl HostManagedModelStreamSink for RecordingStreamSink {
        async fn safe_text_update(&self, safe_text: String) {
            self.updates
                .lock()
                .expect("recording stream sink lock")
                .push(safe_text);
        }
    }

    struct StaticToolDefinitions {
        definitions: Vec<ProviderToolDefinition>,
    }

    #[async_trait::async_trait]
    impl LoopCapabilityPort for StaticToolDefinitions {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self.definitions.clone())
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface:test").expect("surface version"),
                descriptors: Vec::new(),
                callable_capability_ids: None,
            })
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            panic!("output hygiene tests do not invoke capabilities")
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            panic!("output hygiene tests do not invoke capability batches")
        }
    }

    fn model_request(prior_capability_id: Option<&str>) -> HostManagedModelRequest {
        let messages = prior_capability_id
            .map(|capability_id| HostManagedModelMessage {
                role: HostManagedModelMessageRole::ToolResult,
                content: "hydrated result for U0123ABCDE".to_string(),
                content_ref: LoopMessageRef::new("msg:slack-output-hygiene").expect("message ref"),
                tool_result_provider_call: Some(ProviderToolCallReferenceEnvelope {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "test-turn".to_string(),
                    provider_call_id: "test-call".to_string(),
                    provider_tool_name: ProviderToolName::new("slack_search_messages")
                        .expect("provider tool name"),
                    capability_id: CapabilityId::new(capability_id).expect("capability id"),
                    arguments: serde_json::json!({"query": "U0123ABCDE"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                }),
                tool_result_content: None,
                image_parts: Vec::new(),
            })
            .into_iter()
            .collect();
        HostManagedModelRequest {
            model_profile_id: ModelProfileId::new("interactive_model").expect("model profile"),
            messages,
            surface_version: None,
            resolved_model_route: None,
            run_id: TurnRunId::new(),
            turn_id: TurnId::new(),
        }
    }

    fn tool_definitions(capability_id: &str) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(StaticToolDefinitions {
            definitions: vec![
                ProviderToolDefinition::from_parts(
                    CapabilityId::new(capability_id).expect("capability id"),
                    capability_id.replace('.', "_"),
                    "synthetic capability for output hygiene tests",
                    serde_json::json!({"type": "object"}),
                )
                .expect("provider tool definition"),
            ],
        })
    }

    fn slack_send_call(arguments: serde_json::Value) -> CapabilityCallCandidate {
        let capability_id = CapabilityId::new("slack.send_message").expect("capability id");
        CapabilityCallCandidate {
            activity_id: Default::default(),
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("surface version"),
            capability_id: capability_id.clone(),
            input_ref: CapabilityInputRef::new("input:slack-send").expect("input ref"),
            effective_capability_ids: vec![capability_id],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                provider_turn_id: "test-turn".to_string(),
                provider_call_id: "test-call".to_string(),
                provider_tool_name: ProviderToolName::new("slack_send_message")
                    .expect("provider tool name"),
                arguments,
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        }
    }

    #[tokio::test]
    async fn slack_output_hygiene_gateway_redacts_assistant_text_and_reasoning_from_slack_history()
    {
        let request = model_request(Some("slack.search_messages"));
        let usage = LoopModelUsage {
            input_tokens: 23,
            output_tokens: 11,
        };
        let inner = Arc::new(RecordingGateway::new(
            HostManagedModelResponse::assistant_reply_with_reasoning(
                "user U0123ABCDE",
                Some("mention <@U0123ABCDE>".to_string()),
            )
            .with_usage(usage),
        ));
        let gateway = SlackOutputHygieneGateway::new(inner.clone());

        let response = gateway
            .stream_model(request.clone())
            .await
            .expect("decorated gateway response");

        assert_eq!(
            inner.methods.lock().expect("methods lock").as_slice(),
            ["stream_model"]
        );
        assert_eq!(
            inner.requests.lock().expect("requests lock").as_slice(),
            [request],
            "hydrated request content and provider metadata must be forwarded unchanged"
        );
        assert_eq!(
            response.safe_text_deltas,
            ["user [Slack identifier redacted]"]
        );
        assert_eq!(
            response.safe_reasoning_deltas,
            ["mention [Slack identifier redacted]"]
        );
        assert_eq!(
            response.output,
            ParentLoopOutput::AssistantReply(ironclaw_turns::run_profile::AssistantReply {
                content: "user [Slack identifier redacted]".to_string(),
            })
        );
        assert_eq!(response.usage, Some(usage));
    }

    #[tokio::test]
    async fn slack_output_hygiene_gateway_redacts_cumulative_progress_for_slack_history() {
        let inner = Arc::new(
            RecordingGateway::new(HostManagedModelResponse::assistant_reply("complete"))
                .with_progress(vec![
                    "checking U0123ABCDE",
                    "checking U0123ABCDE and W0123ABCDE",
                ]),
        );
        let gateway = SlackOutputHygieneGateway::new(inner.clone());
        let sink = Arc::new(RecordingStreamSink::default());

        gateway
            .stream_model_with_progress(model_request(Some("slack.search_messages")), sink.clone())
            .await
            .expect("decorated progress response");

        assert_eq!(
            sink.updates(),
            vec![
                "checking [Slack identifier redacted]",
                "checking [Slack identifier redacted] and [Slack identifier redacted]",
            ]
        );
        assert_eq!(
            inner.methods.lock().expect("methods lock").as_slice(),
            ["stream_model_with_progress"]
        );
    }

    #[tokio::test]
    async fn slack_output_hygiene_gateway_detects_visible_slack_tools_without_mutating_capability_calls()
     {
        let arguments = serde_json::json!({
            "user_id": "U0123ABCDE",
            "text": "<@U0123ABCDE>"
        });
        let raw_response = HostManagedModelResponse::capability_calls_with_reasoning(
            vec![slack_send_call(arguments.clone())],
            "sending for U0123ABCDE",
            Some("send to <@U0123ABCDE>".to_string()),
        );
        let expected_output = raw_response.output.clone();
        let inner = Arc::new(RecordingGateway::new(raw_response));
        let gateway = SlackOutputHygieneGateway::new(inner.clone());

        let response = gateway
            .stream_model_with_capabilities(
                model_request(None),
                tool_definitions("slack.send_message"),
            )
            .await
            .expect("decorated capability response");

        assert_eq!(
            response.safe_text_deltas,
            ["sending for [Slack identifier redacted]"]
        );
        assert_eq!(
            response.safe_reasoning_deltas,
            ["send to [Slack identifier redacted]"]
        );
        assert_eq!(response.output, expected_output);
        assert_eq!(
            inner.methods.lock().expect("methods lock").as_slice(),
            ["stream_model_with_capabilities"]
        );
        let ParentLoopOutput::CapabilityCalls(calls) = response.output else {
            panic!("capability calls must remain capability calls")
        };
        assert_eq!(
            calls[0]
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .arguments,
            arguments,
            "Slack send_message arguments must remain byte-for-byte unchanged"
        );
    }

    #[tokio::test]
    async fn slack_output_hygiene_gateway_preserves_non_slack_response_and_progress() {
        let request = model_request(None);
        let raw_response = HostManagedModelResponse::assistant_reply_with_reasoning(
            "user U0123ABCDE",
            Some("mention <@U0123ABCDE>".to_string()),
        );
        let inner = Arc::new(
            RecordingGateway::new(raw_response.clone()).with_progress(vec!["checking U0123ABCDE"]),
        );
        let gateway = SlackOutputHygieneGateway::new(inner.clone());
        let sink = Arc::new(RecordingStreamSink::default());

        let response = gateway
            .stream_model_with_capabilities_and_progress(
                request.clone(),
                tool_definitions("builtin.echo"),
                sink.clone(),
            )
            .await
            .expect("non-Slack response");

        assert_eq!(response, raw_response);
        assert_eq!(sink.updates(), vec!["checking U0123ABCDE"]);
        assert_eq!(
            inner.methods.lock().expect("methods lock").as_slice(),
            ["stream_model_with_capabilities_and_progress"]
        );
        assert_eq!(
            inner.requests.lock().expect("requests lock").as_slice(),
            [request]
        );
    }

    #[tokio::test]
    async fn slack_output_hygiene_gateway_wraps_scope_resolved_inner_gateway() {
        let resolved_inner = Arc::new(RecordingGateway::new(
            HostManagedModelResponse::assistant_reply("user U0123ABCDE"),
        ));
        let root_inner = Arc::new(
            RecordingGateway::new(HostManagedModelResponse::assistant_reply("fallback"))
                .with_resolved(resolved_inner),
        );
        let gateway = SlackOutputHygieneGateway::new(root_inner);
        let scope = TurnScope::new(
            TenantId::new("tenant-output-hygiene").expect("tenant id"),
            None,
            None,
            ThreadId::new("thread-output-hygiene").expect("thread id"),
        );
        let resolved = gateway
            .resolve_for_scope(&scope)
            .expect("resolved gateway must stay decorated");

        let response = resolved
            .stream_model(model_request(Some("slack.search_messages")))
            .await
            .expect("resolved decorated response");

        assert_eq!(
            response.output,
            ParentLoopOutput::AssistantReply(ironclaw_turns::run_profile::AssistantReply {
                content: "user [Slack identifier redacted]".to_string(),
            })
        );
    }
}
