//! Bounded loop-ref newtypes and the shared trait-impl macros that back them.

use ironclaw_host_api::INPUT_ENCODE_HUMAN_SUMMARY;
use serde::{Deserialize, Deserializer, Serialize};

use super::run_context::LoopRunContext;
use super::validate::{
    validate_bounded_loop_string, validate_loop_inline_message_body, validate_loop_opaque_token,
    validate_loop_safe_identifier, validate_loop_safe_summary, validate_prefixed_loop_ref,
    validate_prefixed_path_safe_loop_ref,
};

/// Emit the `AsRef<str>` / `Display` / validated-`Deserialize` trait impls
/// shared by every bounded loop-ref newtype. Requires the type to expose an
/// inherent `new(impl Into<String>) -> Result<Self, String>` and `as_str()`.
/// The validated `Deserialize` routes through `new` so wire values get the same
/// bounds/charset check as explicit construction. Used both by
/// [`bounded_loop_ref!`] and by the newtypes whose `new` needs a custom
/// validator (so their trait surface stays identical rather than re-typed).
macro_rules! impl_bounded_ref_traits {
    ($name:ident) => {
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! bounded_loop_ref {
    ($name:ident, $label:literal, $prefix:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                validate_prefixed_loop_ref($label, $prefix, $max, value.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl_bounded_ref_traits!($name);
    };
}

bounded_loop_ref!(CapabilityInputRef, "capability input ref", "input:", 256);
bounded_loop_ref!(
    LoopInputCursorToken,
    "loop input cursor token",
    "input-cursor:",
    256
);
bounded_loop_ref!(LoopInputAckToken, "loop input ack token", "input-ack:", 256);
bounded_loop_ref!(LoopProcessRef, "loop process ref", "process:", 256);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopCheckpointStateRef(String);

impl LoopCheckpointStateRef {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_prefixed_path_safe_loop_ref(
            "loop checkpoint state ref",
            "checkpoint:",
            256,
            value.into(),
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_bounded_ref_traits!(LoopCheckpointStateRef);

impl LoopCheckpointStateRef {
    pub(crate) fn legacy_unknown() -> Self {
        Self("checkpoint:unknown".to_string())
    }

    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop checkpoint state token", 96)?;
        Self::new(format!("checkpoint:{}:{token}", context.run_id))
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        let Some(token) = self
            .0
            .strip_prefix(&format!("checkpoint:{}:", context.run_id))
        else {
            return false;
        };
        validate_loop_opaque_token(token.to_string(), "loop checkpoint state token", 96).is_ok()
    }
}

/// Opaque reference to a host-built prompt bundle for one loop run.
///
/// Serialized refs use `prompt:{run_id}:{opaque_token}`. Consumers must treat
/// the token as opaque metadata and must not infer or persist raw prompt text
/// from this value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopPromptBundleRef(String);

impl LoopPromptBundleRef {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value =
            validate_prefixed_loop_ref("loop prompt bundle ref", "prompt:", 256, value.into())?;
        let suffix = value
            .strip_prefix("prompt:")
            .ok_or_else(|| "loop prompt bundle ref must start with `prompt:`".to_string())?;
        let (run_id, token) = suffix.split_once(':').ok_or_else(|| {
            "loop prompt bundle ref must include scoped run id and opaque token".to_string()
        })?;
        uuid::Uuid::parse_str(run_id)
            .map_err(|_| "loop prompt bundle ref run id must be a UUID".to_string())?;
        validate_loop_opaque_token(token.to_string(), "loop prompt bundle token", 96)?;
        Ok(Self(value))
    }

    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop prompt bundle token", 96)?;
        Self::new(format!("prompt:{}:{token}", context.run_id))
    }

    pub(crate) fn fresh_for_run(context: &LoopRunContext) -> Self {
        Self(format!(
            "prompt:{}:{}",
            context.run_id,
            uuid::Uuid::new_v4()
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.0.starts_with(&format!("prompt:{}:", context.run_id))
    }
}

impl_bounded_ref_traits!(LoopPromptBundleRef);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopSafeSummary(String);

impl LoopSafeSummary {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_summary(value.into()).map(Self)
    }

    /// Build a display-safe capability failure summary, replacing unsafe input
    /// with a fixed redaction marker.
    pub fn capability_failure_summary(value: impl Into<String>) -> Self {
        Self::new(value).unwrap_or_else(|_| Self::tool_failure_details_redacted())
    }

    /// Fixed summary used when a capability failure detail was intentionally
    /// redacted before reaching a user-visible or model-visible boundary.
    pub fn tool_failure_details_redacted() -> Self {
        Self("the tool failure details were redacted".to_string())
    }

    /// Fixed fallback for input-encoding failures when no narrower safe detail
    /// is available.
    pub fn tool_input_could_not_be_encoded() -> Self {
        Self(INPUT_ENCODE_HUMAN_SUMMARY.to_string())
    }

    pub fn model_gateway_failed() -> Self {
        Self("model gateway failed".to_string())
    }

    /// Sanitized summary for a primary model call that exceeded its timeout.
    /// Infallible because the literal is known to satisfy
    /// [`validate_loop_safe_summary`].
    pub fn model_gateway_timed_out() -> Self {
        Self("model gateway timed out".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_bounded_ref_traits!(LoopSafeSummary);

/// Validated body for host-approved inline prompt messages.
///
/// Unlike [`LoopSafeSummary`], this preserves model-visible prompt formatting
/// and uses the generic model-content validation budget.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct LoopInlineMessageBody(String);

impl LoopInlineMessageBody {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for LoopInlineMessageBody {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopInlineMessageBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for LoopInlineMessageBody {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_loop_inline_message_body(value).map(Self)
    }
}

// `LoopSafeSummary`'s AsRef/Display/Deserialize come from
// `impl_bounded_ref_traits!` at its definition above.

pub(crate) fn origin_input_cursor_token() -> LoopInputCursorToken {
    LoopInputCursorToken("input-cursor:origin".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CapabilitySurfaceVersion(String);

impl CapabilitySurfaceVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability surface version", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl_bounded_ref_traits!(CapabilitySurfaceVersion);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CapabilityResumeToken(String);

impl CapabilityResumeToken {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_bounded_loop_string(value.into(), "capability resume token", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl_bounded_ref_traits!(CapabilityResumeToken);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_failure_summary_redacts_secret_like_tokens() {
        for raw_summary in [
            "provider returned AKIAIOSFODNN7EXAMPLE",
            "provider returned gcp-live-secret",
            "provider returned sk-ant-live-secret",
            "provider returned ghp_live_secret",
            "provider returned github_pat_live_secret",
        ] {
            let summary = LoopSafeSummary::capability_failure_summary(raw_summary);
            assert_eq!(
                summary.as_str(),
                "the tool failure details were redacted",
                "summary must be redacted: {raw_summary}"
            );
        }
    }

    #[test]
    fn loop_safe_summary_accepts_fixed_input_encode_summary() {
        let summary = LoopSafeSummary::new(INPUT_ENCODE_HUMAN_SUMMARY)
            .expect("fixed host-authored input encode summary is safe");
        assert_eq!(summary.as_str(), INPUT_ENCODE_HUMAN_SUMMARY);
    }
}
