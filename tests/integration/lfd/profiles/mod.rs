//! Profile registry — maps a Case's `profile` string to the harness assembly
//! for that feature (SCHEMA.md §1/§6).
//!
//! Adding a per-feature profile = ONE new file in this directory implementing
//! [`LfdProfile`] plus one arm in [`resolve`]. Profiles assemble harnesses and
//! interpret profile-owned case fields (`setup.profile_extra`, inbound payload
//! shape, profile-specific state-query kinds); outcome EXTRACTION stays in the
//! pinned `extract` module — a profile cannot fabricate outcomes through this
//! API.

use async_trait::async_trait;

use crate::case::{Case, InboundEntry};
use crate::reborn_support::builder::RebornIntegrationHarness;

pub mod slack_channel;
pub mod smoke_builtin_tools;

/// How a profile step failed, mapping 1:1 onto the two non-`ran` Outcome
/// statuses (SCHEMA.md §2).
#[derive(Debug)]
pub enum ProfileError {
    /// The profile cannot execute this case (→ `status: "unsupported"`).
    Unsupported(String),
    /// The harness raised (→ `status: "error"`).
    Harness(String),
}

/// One feature's harness assembly. Object-safe (`async_trait`) so the runner
/// resolves profiles by the case's `profile` string at runtime.
#[async_trait]
pub trait LfdProfile: Send + Sync {
    /// The registry key — the value a Case's `profile` field must carry.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    /// Map the case's `setup` + `llm_script` to a BUILT
    /// [`RebornIntegrationHarness`]. Return [`ProfileError::Unsupported`] for
    /// any setup axis this profile does not wire (never silently ignore one).
    async fn assemble(&self, case: &Case) -> Result<RebornIntegrationHarness, ProfileError>;

    /// The text submitted for one inbound entry's turn. Default: the payload
    /// must carry a string `"text"` field (or be a bare JSON string).
    fn inbound_text(&self, inbound: &InboundEntry) -> Result<String, ProfileError> {
        if let Some(text) = inbound.payload.as_str() {
            return Ok(text.to_string());
        }
        inbound
            .payload
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ProfileError::Unsupported(
                    "inbound payload has no string \"text\" field".to_string(),
                )
            })
    }

    /// Submit one inbound entry through the profile's ingress. Most profiles
    /// use plain text synthetic turns; product-channel profiles can override
    /// this to parse raw channel payloads, skip no-ops, or apply protocol-level
    /// idempotency before a turn is admitted.
    async fn submit_inbound(
        &self,
        harness: &RebornIntegrationHarness,
        inbound: &InboundEntry,
    ) -> Result<(), ProfileError> {
        let text = self.inbound_text(inbound)?;
        harness
            .submit_turn(&text)
            .await
            .map_err(|error| ProfileError::Harness(format!("turn failed: {error}")))?;
        Ok(())
    }

    /// Profile-specific state-query kinds, consulted only AFTER the pinned
    /// dispatcher's built-in kinds (`extract::run_state_queries`). Default:
    /// none supported.
    async fn state_query(
        &self,
        _harness: &RebornIntegrationHarness,
        kind: &str,
        _params: &serde_json::Value,
    ) -> Result<serde_json::Value, ProfileError> {
        Err(ProfileError::Unsupported(format!(
            "unsupported state query kind {kind:?}"
        )))
    }
}

/// Look up the profile named by a case. `None` → `status: "unsupported"`.
pub fn resolve(profile: &str) -> Option<Box<dyn LfdProfile>> {
    match profile {
        slack_channel::NAME => Some(Box::new(slack_channel::SlackChannel)),
        smoke_builtin_tools::NAME => Some(Box::new(smoke_builtin_tools::SmokeBuiltinTools)),
        _ => None,
    }
}
