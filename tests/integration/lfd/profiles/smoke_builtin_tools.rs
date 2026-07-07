//! `smoke_builtin_tools` — the minimal working profile: builtin tools
//! (`builtin.http`/`echo`/`time`/`json`/`shell` via the real first-party
//! runtime over recording egress), scripted model from the case's
//! `llm_script`, plain text turns from `inbound`. The template for
//! per-feature profile files.

use async_trait::async_trait;

use super::{LfdProfile, ProfileError};
use crate::case::{Case, ScriptStep};
use crate::reborn_support::builder::RebornIntegrationHarness;
use crate::reborn_support::http_matcher::ScriptedHttpResponse;
use crate::reborn_support::reply::RebornScriptedReply;

pub const NAME: &str = "smoke_builtin_tools";

pub struct SmokeBuiltinTools;

#[async_trait]
impl LfdProfile for SmokeBuiltinTools {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn assemble(&self, case: &Case) -> Result<RebornIntegrationHarness, ProfileError> {
        // Fail unsupported (never silently ignore) on every setup axis this
        // minimal profile does not wire.
        if !case.setup.extensions.is_empty() {
            return Err(ProfileError::Unsupported(
                "smoke_builtin_tools does not install extensions".to_string(),
            ));
        }
        if !case.setup.memory_docs.is_empty() {
            return Err(ProfileError::Unsupported(
                "smoke_builtin_tools does not seed memory docs".to_string(),
            ));
        }
        if !case.setup.triggers.is_empty() {
            return Err(ProfileError::Unsupported(
                "smoke_builtin_tools does not seed triggers".to_string(),
            ));
        }
        if case.setup.has_profile_extra() {
            return Err(ProfileError::Unsupported(
                "smoke_builtin_tools takes no profile_extra".to_string(),
            ));
        }
        // `setup.secrets` are accepted but NOT injected into any store — this
        // profile has no credential surface. Their values still feed the
        // pinned leak scan.
        let mut builder = RebornIntegrationHarness::builder(format!("conv-lfd-{}", case.case_id))
            .with_builtin_http_tools()
            .with_turn_event_sink()
            .script(script_replies(&case.llm_script));
        if !case.setup.http_stubs.is_empty() {
            let responses: Result<Vec<ScriptedHttpResponse>, ProfileError> = case
                .setup
                .http_stubs
                .iter()
                .map(|stub| {
                    let body = serde_json::to_vec(&stub.body).map_err(|error| {
                        ProfileError::Harness(format!(
                            "http stub {:?} body does not serialize: {error}",
                            stub.key
                        ))
                    })?;
                    Ok(ScriptedHttpResponse::for_url(stub.key.clone(), body)
                        .with_status(stub.status))
                })
                .collect();
            builder = builder.with_keyed_http_responses(responses?);
        }
        builder
            .build()
            .await
            .map_err(|error| ProfileError::Harness(format!("harness build failed: {error}")))
    }
}

/// Flatten the case's `llm_script` into the harness's FIFO reply script — one
/// `RebornScriptedReply` per step, in turn order.
fn script_replies(script: &[crate::case::ScriptTurn]) -> Vec<RebornScriptedReply> {
    script
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .map(|step| match step {
            ScriptStep::Tool { tool, params } => {
                RebornScriptedReply::tool_call(tool, params.clone())
            }
            ScriptStep::Text { text } => RebornScriptedReply::text(text.clone()),
        })
        .collect()
}
