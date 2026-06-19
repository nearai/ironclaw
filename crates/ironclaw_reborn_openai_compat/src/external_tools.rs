//! Client-supplied ("external") tool ports and DTO parsing for the Responses
//! surface.
//!
//! The route crate owns only the API contract: it parses the request `tools`
//! into [`OpenAiCompatExternalToolSpec`]s and drives two host-wired ports —
//! [`OpenAiCompatExternalToolStore`] (register specs at submit, submit client
//! outputs on resume) and [`OpenAiCompatExternalToolResume`] (resume a parked
//! `BlockedExternalTool` run). The composition layer adapts both ports to the
//! engine's run-scoped external-tool catalog and turn coordinator; this crate
//! never touches the engine directly (see the crate boundary in `CLAUDE.md`).

use async_trait::async_trait;
use serde::Deserialize;

use crate::{OpenAiCompatActorScope, OpenAiCompatHttpError, OpenAiCompatTurnRunRef};

/// Maximum external tools accepted on one Responses create request. Mirrors the
/// engine catalog's per-run cap so an over-large `tools` array is rejected at
/// the parse boundary with a stable `400` rather than deep in composition.
const MAX_RESPONSES_EXTERNAL_TOOLS: usize = 256;

/// A client-declared function tool parsed from a Responses `tools` entry. The
/// model may call it; the host never executes it — a call parks the run and the
/// call is handed back to the API client as a `function_call` output item.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiCompatExternalToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
}

/// Host-wired store for a run's client-supplied tool specs and the outputs the
/// client submits to resolve parked calls. Wired by composition over the engine
/// external-tool catalog; absent (`None` on the workflow) means the Responses
/// surface rejects `tools`/`function_call_output` with a stable `400`.
#[async_trait]
pub trait OpenAiCompatExternalToolStore: Send + Sync {
    /// Register (replacing any prior set) the external tools for a run, so the
    /// model is offered them on its next planning step. Called immediately after
    /// the create submit returns the run ref.
    async fn register_tools(
        &self,
        run_ref: OpenAiCompatTurnRunRef,
        specs: Vec<OpenAiCompatExternalToolSpec>,
    ) -> Result<(), OpenAiCompatHttpError>;

    /// Record a client-submitted output for a parked external tool call, keyed by
    /// the `call_id` surfaced in the earlier `function_call` output item.
    async fn submit_tool_output(
        &self,
        run_ref: OpenAiCompatTurnRunRef,
        call_id: String,
        output: serde_json::Value,
    ) -> Result<(), OpenAiCompatHttpError>;
}

/// The data composition needs to resume a parked external-tool run. The run id
/// and thread id locate the parked run; composition reads the run's current gate
/// and binding refs from the coordinator to build the engine resume request.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiCompatExternalToolResumeRequest {
    pub actor_scope: OpenAiCompatActorScope,
    pub run_ref: OpenAiCompatTurnRunRef,
    /// Canonical thread id the run belongs to (decoded from the response's bound
    /// projection ref). Required to authorize the coordinator run-state read.
    pub thread_id: String,
}

/// Host-wired resume for a parked `BlockedExternalTool` run after its client tool
/// outputs were submitted through [`OpenAiCompatExternalToolStore`]. Wired by
/// composition over the engine turn coordinator's external-tool resume.
#[async_trait]
pub trait OpenAiCompatExternalToolResume: Send + Sync {
    async fn resume_external_tool_run(
        &self,
        request: OpenAiCompatExternalToolResumeRequest,
    ) -> Result<(), OpenAiCompatHttpError>;
}

/// Wire shape of a single Responses `tools` entry. Unknown fields are tolerated
/// (DTO policy) so newer optional tool attributes do not fail parsing.
#[derive(Deserialize)]
struct ExternalToolWire {
    #[serde(rename = "type")]
    tool_type: String,
    name: Option<String>,
    description: Option<String>,
    parameters: Option<serde_json::Value>,
}

/// Parse the request `tools` array into validated external tool specs. Only
/// `type: "function"` entries are supported on this surface; any other tool type
/// (e.g. `web_search_preview`) returns a stable `400` naming `tools`. An empty
/// array is treated as "no tools" by the caller before this is invoked.
pub(crate) fn parse_external_tools(
    tools: &[serde_json::Value],
) -> Result<Vec<OpenAiCompatExternalToolSpec>, OpenAiCompatHttpError> {
    if tools.len() > MAX_RESPONSES_EXTERNAL_TOOLS {
        return Err(invalid_tools());
    }
    let mut specs = Vec::with_capacity(tools.len());
    for tool in tools {
        let wire: ExternalToolWire =
            serde_json::from_value(tool.clone()).map_err(|_| invalid_tools())?;
        if wire.tool_type != "function" {
            return Err(invalid_tools());
        }
        let name = wire
            .name
            .filter(|name| !name.is_empty())
            .ok_or_else(invalid_tools)?;
        specs.push(OpenAiCompatExternalToolSpec {
            name,
            description: wire.description.unwrap_or_default(),
            parameters_schema: wire
                .parameters
                .unwrap_or_else(|| serde_json::json!({"type": "object"})),
        });
    }
    Ok(specs)
}

fn invalid_tools() -> OpenAiCompatHttpError {
    OpenAiCompatHttpError::invalid_request(Some("tools".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function_tools_with_defaults() {
        let specs = parse_external_tools(&[serde_json::json!({
            "type": "function",
            "name": "get_weather",
            "description": "Look up weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
        })])
        .expect("parse");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "get_weather");
        assert_eq!(specs[0].description, "Look up weather");

        // Missing description/parameters default rather than failing.
        let specs = parse_external_tools(&[serde_json::json!({
            "type": "function",
            "name": "ping"
        })])
        .expect("parse defaults");
        assert_eq!(specs[0].description, "");
        assert_eq!(
            specs[0].parameters_schema,
            serde_json::json!({"type": "object"})
        );
    }

    #[test]
    fn rejects_non_function_and_nameless_tools() {
        assert!(
            parse_external_tools(&[serde_json::json!({"type": "web_search_preview"})]).is_err()
        );
        assert!(parse_external_tools(&[serde_json::json!({"type": "function"})]).is_err());
        assert!(
            parse_external_tools(&[serde_json::json!({"type": "function", "name": ""})]).is_err()
        );
    }
}
