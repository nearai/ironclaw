//! Finish-reason conformance for the rig bridge.
//!
//! Split out of `rig_adapter.rs` (#6284 item 8, contract clause (e)): the
//! matrix and its provider fixtures run to hundreds of lines of payload, and
//! reading the adapter's production conversion logic should not mean
//! scrolling past them. This is a child of `rig_adapter`'s `tests` module, so
//! `super::*` still reaches both the adapter's items and the shared test
//! helpers.

use super::*;

/// A terminal frame shaped like Ollama's — the one provider whose rig-core
/// `StreamingResponse` still carries the finish reason.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct OllamaShapedStreamingResponse {
    done_reason: Option<String>,
}

impl GetTokenUsage for OllamaShapedStreamingResponse {
    fn token_usage(&self) -> Option<RigUsage> {
        Some(RigUsage::new())
    }
}

/// A streaming model whose script the test controls: whether a tool call
/// is emitted, and whether the provider ever sends a terminal frame.
#[derive(Clone)]
struct ScriptedStreamingModel {
    emit_tool_call: bool,
    terminal_frame: Option<OllamaShapedStreamingResponse>,
}

impl CompletionModel for ScriptedStreamingModel {
    type Response = serde_json::Value;
    type StreamingResponse = OllamaShapedStreamingResponse;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        unimplemented!("constructed directly in tests")
    }

    async fn completion(
        &self,
        _request: RigRequest,
    ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
        Err(CompletionError::ProviderError(
            "non-streaming path must not be used".to_string(),
        ))
    }

    async fn stream(
        &self,
        _request: RigRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let mut frames = vec![Ok(RawStreamingChoice::Message("partial".to_string()))];
        if self.emit_tool_call {
            frames.push(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                "call-1".to_string(),
                "search".to_string(),
                serde_json::json!({"query": "iron"}),
            ))));
        }
        if let Some(frame) = self.terminal_frame.clone() {
            frames.push(Ok(RawStreamingChoice::FinalResponse(frame)));
        }
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(frames),
        )))
    }
}

fn search_tool_request() -> ToolCompletionRequest {
    ToolCompletionRequest::new(
        vec![ChatMessage::user("search")],
        vec![IronToolDefinition {
            name: "search".to_string(),
            description: "Search the index".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }],
    )
}

fn discarding_sink() -> Arc<dyn CompletionStreamSink> {
    let (tx, _rx) = mpsc::unbounded_channel();
    Arc::new(RecordingCompletionStreamSink { sender: tx })
}

/// #6284 item 8, streaming half: `drain_streaming_response`'s only exit was
/// `stream.next() == None`. A stream that ran dry without the provider ever
/// yielding its terminal frame produced the same `Stop` as a provider that
/// finished, so a truncated run was reported as a clean success.
///
/// A missing terminal frame is the crate's existing incomplete-stream
/// condition (`codex_chatgpt`'s "stream ended before response.completed"),
/// so it surfaces the same way: a retryable `LlmError::StreamInterrupted`,
/// raised *before* a response object is constructed, so retry and failover
/// see a failure rather than a completed call.
///
/// **Scope, honestly:** in rig-core 0.36 only Ollama can reach this. Its
/// stream yields `FinalResponse` solely inside `if response.done`. OpenAI,
/// Anthropic, Gemini, OpenRouter and DeepSeek yield their usage-only
/// `FinalResponse` unconditionally after the SSE loop — including after a
/// plain `Err(StreamEnded)` break — so for those five a server that closes
/// mid-answer still sets `stream.response` and this condition never fires.
/// Detecting that needs a terminal-observed signal rig-core does not carry;
/// see the PR's "Known limitation".
#[tokio::test]
async fn streaming_without_terminal_frame_is_a_retryable_incomplete_stream() {
    let adapter = with_native_streaming(RigAdapter::new(
        ScriptedStreamingModel {
            emit_tool_call: false,
            terminal_frame: None,
        },
        "streaming-truncated",
    ));

    let error = adapter
        .complete_streaming(
            CompletionRequest::new(vec![ChatMessage::user("hello")]),
            discarding_sink(),
        )
        .await
        .expect_err("a stream that never terminated is not a completed call");

    match error {
        LlmError::StreamInterrupted { reason, .. } => assert!(
            reason.contains("terminal frame"),
            "reason must name the missing terminal frame, got: {reason}"
        ),
        other => panic!("expected a retryable StreamInterrupted, got {other:?}"),
    }
}

/// The same seam on the tool-capable path. A half-streamed tool call must
/// never reach a caller at all — the arguments may be cut off.
#[tokio::test]
async fn streaming_tool_call_without_terminal_frame_is_a_retryable_incomplete_stream() {
    let adapter = with_native_streaming(RigAdapter::new(
        ScriptedStreamingModel {
            emit_tool_call: true,
            terminal_frame: None,
        },
        "streaming-truncated",
    ));

    let error = adapter
        .complete_with_tools_streaming(search_tool_request(), discarding_sink())
        .await
        .expect_err("a half-streamed tool call is not a completed call");

    assert!(
        matches!(error, LlmError::StreamInterrupted { .. }),
        "expected a retryable StreamInterrupted, got {error:?}"
    );
}

/// When the provider *does* carry a finish reason on its terminal frame
/// (rig-core 0.36: Ollama only), the adapter reports it — including over
/// the tool-call shape.
#[tokio::test]
async fn streaming_reads_the_terminal_frames_finish_reason() {
    let adapter = with_native_streaming(RigAdapter::new(
        ScriptedStreamingModel {
            emit_tool_call: true,
            terminal_frame: Some(OllamaShapedStreamingResponse {
                done_reason: Some("length".to_string()),
            }),
        },
        "streaming-ollama",
    ));

    let response = adapter
        .complete_with_tools_streaming(search_tool_request(), discarding_sink())
        .await
        .expect("streaming completion succeeds");

    assert_eq!(response.finish_reason, FinishReason::Length);
}

/// A terminal frame that carries no finish reason at all is the common
/// case: rig-core 0.36 drops it for OpenAI, Anthropic, Gemini, OpenRouter
/// and DeepSeek, whose `StreamingResponse` types hold usage only. Those
/// five also yield that frame unconditionally, so its presence proves
/// nothing about whether the server actually finished — shape inference
/// stays the documented fallback there, and a mid-answer disconnect on
/// those providers is *not* detected by this adapter. Reporting `Unknown`
/// for every streamed OpenAI turn would fail runs that actually succeeded.
#[tokio::test]
async fn streaming_terminal_frame_without_a_finish_reason_falls_back_to_shape() {
    let adapter = with_native_streaming(RigAdapter::new(
        ScriptedStreamingModel {
            emit_tool_call: true,
            terminal_frame: Some(OllamaShapedStreamingResponse { done_reason: None }),
        },
        "streaming-openai-like",
    ));

    let response = adapter
        .complete_with_tools_streaming(search_tool_request(), discarding_sink())
        .await
        .expect("streaming completion succeeds");

    assert_eq!(response.finish_reason, FinishReason::ToolUse);
}

/// Conformance matrix for the rig bridge (#6284 item 8, contract clause
/// (e): "no non-success may be reported as success").
///
/// Every provider rig fronts states *its own* reason for stopping in its
/// own vocabulary. Before this pin, `extract_response` guessed from the
/// response shape — tool calls present meant `ToolUse`, everything else
/// meant `Stop` — so a `max_tokens` truncation and a content-filter refusal
/// were indistinguishable from a clean answer, and `FinishReason::Length`
/// and `FinishReason::ContentFilter` were unreachable for every rig-backed
/// provider.
///
/// Each row is one provider token in the provider's own spelling, taken
/// from the raw response JSON path that provider actually emits.
#[test]
fn provider_finish_reason_conformance_matrix() {
    fn openai_shaped(finish: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "logprobs": null,
                "finish_reason": finish,
            }],
        })
    }
    fn anthropic_shaped(stop: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_1",
            "model": "claude-sonnet-4",
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": stop,
            "stop_sequence": null,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })
    }
    fn ollama_shaped(done: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "model": "llama3.2",
            "created_at": "2026-01-01T00:00:00Z",
            "message": {"role": "assistant", "content": "hi"},
            "done": true,
            "done_reason": done,
        })
    }
    fn gemini_shaped(finish: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "responseId": "resp_1",
            "candidates": [{"content": null, "finishReason": finish}],
        })
    }
    // Gemini's *other* block mechanism. rig's own docs on
    // `GenerateContentResponse`: "Returns no candidates at all only if
    // there was something wrong with the prompt (check promptFeedback)."
    fn gemini_prompt_blocked(block: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "responseId": "resp_1",
            "candidates": [],
            "promptFeedback": {"blockReason": block},
        })
    }

    // (adapter family, provider's own token, expected FinishReason)
    let cases: Vec<(&str, serde_json::Value, Option<FinishReason>)> = vec![
        // -- OpenAI-shaped: openai, openai_compatible, tinfoil, azure,
        //    deepseek, openrouter. Path: choices[0].finish_reason.
        (
            "openai/stop",
            openai_shaped("stop".into()),
            Some(FinishReason::Stop),
        ),
        (
            "openai/length",
            openai_shaped("length".into()),
            Some(FinishReason::Length),
        ),
        (
            "openai/tool_calls",
            openai_shaped("tool_calls".into()),
            Some(FinishReason::ToolUse),
        ),
        (
            "openai/function_call",
            openai_shaped("function_call".into()),
            Some(FinishReason::ToolUse),
        ),
        (
            "openai/content_filter",
            openai_shaped("content_filter".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "openai/unknown-token",
            openai_shaped("teapot".into()),
            Some(FinishReason::Unknown),
        ),
        (
            "openai/absent-null",
            openai_shaped(serde_json::Value::Null),
            None,
        ),
        ("openai/absent-empty", openai_shaped("".into()), None),
        // -- Anthropic-by-key. Path: stop_reason.
        (
            "anthropic/end_turn",
            anthropic_shaped("end_turn".into()),
            Some(FinishReason::Stop),
        ),
        (
            "anthropic/stop_sequence",
            anthropic_shaped("stop_sequence".into()),
            Some(FinishReason::Stop),
        ),
        (
            "anthropic/max_tokens",
            anthropic_shaped("max_tokens".into()),
            Some(FinishReason::Length),
        ),
        (
            "anthropic/tool_use",
            anthropic_shaped("tool_use".into()),
            Some(FinishReason::ToolUse),
        ),
        (
            "anthropic/refusal",
            anthropic_shaped("refusal".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "anthropic/unknown-token",
            anthropic_shaped("pause_turn".into()),
            Some(FinishReason::Unknown),
        ),
        (
            "anthropic/absent",
            anthropic_shaped(serde_json::Value::Null),
            None,
        ),
        // -- Ollama. Path: done_reason.
        (
            "ollama/stop",
            ollama_shaped("stop".into()),
            Some(FinishReason::Stop),
        ),
        (
            "ollama/length",
            ollama_shaped("length".into()),
            Some(FinishReason::Length),
        ),
        (
            "ollama/unknown-token",
            ollama_shaped("unload".into()),
            Some(FinishReason::Unknown),
        ),
        (
            "ollama/absent",
            ollama_shaped(serde_json::Value::Null),
            None,
        ),
        // -- Gemini by API key. Path: candidates[0].finishReason.
        (
            "gemini/STOP",
            gemini_shaped("STOP".into()),
            Some(FinishReason::Stop),
        ),
        (
            "gemini/MAX_TOKENS",
            gemini_shaped("MAX_TOKENS".into()),
            Some(FinishReason::Length),
        ),
        (
            "gemini/SAFETY",
            gemini_shaped("SAFETY".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/RECITATION",
            gemini_shaped("RECITATION".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/PROHIBITED_CONTENT",
            gemini_shaped("PROHIBITED_CONTENT".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/BLOCKLIST",
            gemini_shaped("BLOCKLIST".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/SPII",
            gemini_shaped("SPII".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            // Gemini's token for a response blocked by Model Armor.
            "gemini/MODEL_ARMOR",
            gemini_shaped("MODEL_ARMOR".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/MALFORMED_FUNCTION_CALL",
            gemini_shaped("MALFORMED_FUNCTION_CALL".into()),
            Some(FinishReason::Unknown),
        ),
        (
            "gemini/OTHER",
            gemini_shaped("OTHER".into()),
            Some(FinishReason::Unknown),
        ),
        (
            "gemini/absent",
            gemini_shaped(serde_json::Value::Null),
            None,
        ),
        // -- Gemini by API key, prompt-level block. The input was
        //    rejected before generation, so there are no candidates at
        //    all and the reason lives at promptFeedback.blockReason.
        //    Falling through to shape inference reported this as a clean
        //    `Stop`.
        (
            "gemini/promptFeedback-SAFETY",
            gemini_prompt_blocked("SAFETY".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            "gemini/promptFeedback-PROHIBITED_CONTENT",
            gemini_prompt_blocked("PROHIBITED_CONTENT".into()),
            Some(FinishReason::ContentFilter),
        ),
        (
            // Recognized as a block we cannot classify — never a clean stop.
            "gemini/promptFeedback-OTHER",
            gemini_prompt_blocked("OTHER".into()),
            Some(FinishReason::Unknown),
        ),
        (
            // A candidate's own reason still wins when both are present.
            "gemini/candidate-outranks-promptFeedback",
            serde_json::json!({
                "responseId": "resp_1",
                "candidates": [{"content": null, "finishReason": "STOP"}],
                "promptFeedback": {"blockReason": "SAFETY"},
            }),
            Some(FinishReason::Stop),
        ),
        (
            // Preserved: promptFeedback carrying only safety ratings
            // blocks nothing, so the provider still said nothing.
            "gemini/promptFeedback-without-blockReason",
            serde_json::json!({
                "responseId": "resp_1",
                "candidates": [],
                "promptFeedback": {"safetyRatings": []},
            }),
            None,
        ),
        (
            // Preserved: no candidates and nothing blocked stays "the
            // provider said nothing", not a filter.
            "gemini/no-candidates-unblocked",
            serde_json::json!({"responseId": "resp_1", "candidates": []}),
            None,
        ),
        // -- A provider shape we do not recognize at all.
        (
            "unrecognized-shape",
            serde_json::json!({"output": "hi"}),
            None,
        ),
    ];

    for (label, raw, expected) in cases {
        assert_eq!(
            extract_finish_reason(Some(&raw)),
            expected,
            "{label}: adapter must report the provider's own finish reason",
        );
    }
}

/// The matrix above feeds `extract_finish_reason` hand-written JSON. This
/// pins that the JSON paths are the ones rig-core's own serde derives
/// actually produce: each fixture is parsed into the concrete rig response
/// type the adapter receives, then handed to the extractor exactly as
/// `complete`/`complete_with_tools` do. If rig renames or moves a field,
/// this fails instead of silently falling back to shape inference.
#[test]
fn finish_reason_paths_match_real_rig_response_types() {
    let openai: rig::providers::openai::completion::CompletionResponse =
        serde_json::from_value(serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "system_fingerprint": null,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "partial"},
                "logprobs": null,
                "finish_reason": "length",
            }],
            "usage": null,
        }))
        .expect("openai fixture must parse into rig's CompletionResponse");
    assert_eq!(
        extract_finish_reason(serialize_raw_response(&openai).as_ref()),
        Some(FinishReason::Length),
        "rig's OpenAI response must still expose choices[0].finish_reason",
    );

    let anthropic: rig::providers::anthropic::completion::CompletionResponse =
        serde_json::from_value(serde_json::json!({
            "id": "msg_1",
            "model": "claude-sonnet-4",
            "role": "assistant",
            "content": [{"type": "text", "text": "partial"}],
            "stop_reason": "max_tokens",
            "stop_sequence": null,
            "usage": {"input_tokens": 1, "output_tokens": 2},
        }))
        .expect("anthropic fixture must parse into rig's CompletionResponse");
    assert_eq!(
        extract_finish_reason(serialize_raw_response(&anthropic).as_ref()),
        Some(FinishReason::Length),
        "rig's Anthropic response must still expose stop_reason",
    );

    let ollama: rig::providers::ollama::CompletionResponse =
        serde_json::from_value(serde_json::json!({
            "model": "llama3.2",
            "created_at": "2026-01-01T00:00:00Z",
            "message": {"role": "assistant", "content": "partial"},
            "done": true,
            "done_reason": "length",
        }))
        .expect("ollama fixture must parse into rig's CompletionResponse");
    assert_eq!(
        extract_finish_reason(serialize_raw_response(&ollama).as_ref()),
        Some(FinishReason::Length),
        "rig's Ollama response must still expose done_reason",
    );

    let gemini: rig::providers::gemini::completion::gemini_api_types::GenerateContentResponse =
        serde_json::from_value(serde_json::json!({
            "responseId": "resp_1",
            "candidates": [{"content": null, "finishReason": "SAFETY"}],
        }))
        .expect("gemini fixture must parse into rig's GenerateContentResponse");
    assert_eq!(
        extract_finish_reason(serialize_raw_response(&gemini).as_ref()),
        Some(FinishReason::ContentFilter),
        "rig's Gemini response must still expose candidates[].finishReason",
    );

    // The prompt-block path, pinned against rig's own serde derives:
    // `GenerateContentResponse` and `PromptFeedback` are both
    // `rename_all = "camelCase"` and `BlockReason` is
    // `SCREAMING_SNAKE_CASE`, so the reason reaches the extractor at
    // `promptFeedback.blockReason` in the vocabulary the shared table
    // already speaks. If rig renames either, this fails instead of
    // silently reporting a blocked prompt as a clean stop.
    let gemini_blocked: rig::providers::gemini::completion::gemini_api_types::GenerateContentResponse =
        serde_json::from_value(serde_json::json!({
            "responseId": "resp_1",
            "candidates": [],
            "promptFeedback": {"blockReason": "SAFETY"},
        }))
        .expect("gemini prompt-block fixture must parse into rig's GenerateContentResponse");
    assert_eq!(
        extract_finish_reason(serialize_raw_response(&gemini_blocked).as_ref()),
        Some(FinishReason::ContentFilter),
        "rig's Gemini response must still expose promptFeedback.blockReason",
    );
}

/// Precedence at the caller: what the provider reported outranks what the
/// body looks like. A truncated or filtered response that *also* carried
/// tool calls must not be laundered into `ToolUse` — the tool arguments
/// may be cut off mid-JSON, and `ironclaw_turn_runner`'s model gateway only
/// accepts provider tool calls when the finish reason is `ToolUse`/`Stop`.
#[test]
fn provider_finish_reason_outranks_response_shape() {
    let with_tool_call = OneOrMany::one(AssistantContent::tool_call(
        "call_1",
        "search",
        serde_json::json!({"q": "test"}),
    ));
    let text_only = OneOrMany::one(AssistantContent::text("hello"));
    let usage = RigUsage::new();

    let finish_of = |choice: &OneOrMany<AssistantContent>, provider| {
        let (_t, _c, finish, _r, _rd) = extract_response(choice, &usage, provider);
        finish
    };

    // Truncation and filtering win over the tool-call shape.
    assert_eq!(
        finish_of(&with_tool_call, Some(FinishReason::Length)),
        FinishReason::Length,
    );
    assert_eq!(
        finish_of(&with_tool_call, Some(FinishReason::ContentFilter)),
        FinishReason::ContentFilter,
    );
    // A provider that says "tool calls" is believed even without any.
    assert_eq!(
        finish_of(&text_only, Some(FinishReason::ToolUse)),
        FinishReason::ToolUse,
    );
    // `stop` alongside tool calls is refined to ToolUse: some proxies
    // report `stop` on tool-call turns, and the tool arguments are intact.
    assert_eq!(
        finish_of(&with_tool_call, Some(FinishReason::Stop)),
        FinishReason::ToolUse,
    );
    // An unclassifiable token is an explicit provider failure and is never
    // refined, however complete the parsed tool calls look: Gemini's
    // MALFORMED_FUNCTION_CALL maps here and does carry function-call parts.
    assert_eq!(
        finish_of(&with_tool_call, Some(FinishReason::Unknown)),
        FinishReason::Unknown,
    );
    // An unclassifiable token with no tool calls stays honest.
    assert_eq!(
        finish_of(&text_only, Some(FinishReason::Unknown)),
        FinishReason::Unknown,
    );
    // Documented fallback when the provider states nothing at all: today's
    // shape inference, and only then.
    assert_eq!(finish_of(&text_only, None), FinishReason::Stop);
    assert_eq!(finish_of(&with_tool_call, None), FinishReason::ToolUse);
}
