use std::fmt;

use serde::{Deserialize, Serialize};

use ironclaw_host_api::{AgentId, ProjectId, UserId};

/// How this turn run was initiated. Generic — no product/channel specifics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOriginKind {
    WebUi,
    Inbound,
    ScheduledTrigger,
}

/// The conversation surface a turn arrived on / replies to. Generic dm-vs-channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSurfaceType {
    Direct,
    Channel,
}

/// Maximum byte length for a [`RunOriginAdapter`] value. Mirrors `AdapterKind`'s
/// validation bound in `ironclaw_conversations` so that any valid `AdapterKind`
/// always converts without narrowing. If `AdapterKind`'s limit changes, update
/// this constant to match.
const MAX_RUN_ORIGIN_ADAPTER_BYTES: usize = 512;

/// Generic adapter identity carried into the turn context. Bounded validated string;
/// callers convert their rich adapter id (e.g. `ProductAdapterId`, `AdapterKind`) into this.
///
/// Serializes as a plain string. Deserialization validates via `TryFrom<String>` so
/// persisted payloads with empty or oversized values are rejected at the boundary.
///
/// The byte-length cap matches `AdapterKind`'s validation bound (512 bytes) so that
/// any valid `AdapterKind` always converts into a `RunOriginAdapter` without silent
/// narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RunOriginAdapter(String);

impl RunOriginAdapter {
    fn validate(s: &str) -> Result<(), crate::TurnError> {
        if s.is_empty() || s.len() > MAX_RUN_ORIGIN_ADAPTER_BYTES {
            return Err(crate::TurnError::InvalidRunOriginAdapter);
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, crate::TurnError> {
        let s = value.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RunOriginAdapter {
    type Error = crate::TurnError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for RunOriginAdapter {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunOriginAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RunOriginAdapter> for String {
    fn from(a: RunOriginAdapter) -> Self {
        a.0
    }
}

/// Who owns this turn, for delivery-preference scoping and slice rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnOwner {
    Personal {
        user: UserId,
    },
    SharedAgent {
        agent: AgentId,
        project: Option<ProjectId>,
    },
}

/// Generic, persisted product context for one turn. Resolved once at ingress by
/// `ironclaw_product_context`; rendered into the model-visible runtime context.
///
/// **Intended mint points** are the resolver functions in `ironclaw_product_context`:
/// `resolve_inbound` (for all inbound/trigger paths), `resolve_web_ui` (for the
/// WebUI gateway), and `resolve_cli` (for local CLI chat). Those resolvers call
/// `ProductTurnContext::new` or `ProductTurnContext::new_with_source_channel`
/// internally; callers outside that crate should not call constructors directly.
/// `#[non_exhaustive]` blocks struct-literal construction from external crates.
///
/// `new` is a low-level constructor and is deliberately *not* a hard cross-crate seal —
/// Rust has no friend-crate visibility, so a type that must live here (it is carried on
/// `SubmitTurnRequest`/`TurnRunState`) cannot restrict construction to one other crate.
/// The enforced trust boundary is upstream, not on this constructor: a `ScheduledTrigger`
/// origin is only produced when ingress enters through the trusted-trigger submit seam,
/// which carries trigger-ness as a typed value rather than re-deriving it from the
/// adapter-kind string (see `ironclaw_conversations` `TrustedInboundKind` and
/// `ironclaw_product_context::resolve_inbound`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductTurnContext {
    pub origin: TurnOriginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_type: Option<TurnSurfaceType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<RunOriginAdapter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<RunOriginAdapter>,
    pub owner: TurnOwner,
}

impl ProductTurnContext {
    pub fn new(
        origin: TurnOriginKind,
        surface_type: Option<TurnSurfaceType>,
        adapter: Option<RunOriginAdapter>,
        owner: TurnOwner,
    ) -> Self {
        let source_channel = adapter.clone();
        Self::new_with_source_channel(origin, surface_type, adapter, source_channel, owner)
    }

    pub fn new_with_source_channel(
        origin: TurnOriginKind,
        surface_type: Option<TurnSurfaceType>,
        adapter: Option<RunOriginAdapter>,
        source_channel: Option<RunOriginAdapter>,
        owner: TurnOwner,
    ) -> Self {
        Self {
            origin,
            surface_type,
            adapter,
            source_channel,
            owner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_turn_context_round_trips_through_json() {
        let ctx = ProductTurnContext::new(
            TurnOriginKind::Inbound,
            Some(TurnSurfaceType::Channel),
            Some(RunOriginAdapter::new("telegram").unwrap()),
            TurnOwner::Personal {
                user: ironclaw_host_api::UserId::new("u1").unwrap(),
            },
        );
        let json = serde_json::to_string(&ctx).unwrap();
        let back: ProductTurnContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, back);
        assert_eq!(
            back.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some("telegram")
        );
    }

    #[test]
    fn product_turn_context_can_stamp_source_channel_without_adapter() {
        let ctx = ProductTurnContext::new_with_source_channel(
            TurnOriginKind::WebUi,
            None,
            None,
            Some(RunOriginAdapter::new("webui").unwrap()),
            TurnOwner::Personal {
                user: ironclaw_host_api::UserId::new("u1").unwrap(),
            },
        );
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(
            json.contains("\"source_channel\":\"webui\""),
            "source channel must serialize independently of adapter: {json}"
        );
        let back: ProductTurnContext = serde_json::from_str(&json).unwrap();
        assert!(back.adapter.is_none());
        assert_eq!(
            back.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some("webui")
        );
    }

    #[test]
    fn run_origin_adapter_rejects_empty() {
        assert!(RunOriginAdapter::new("").is_err());
    }

    #[test]
    fn run_origin_adapter_accepts_at_max_bytes() {
        // Exactly at the limit must succeed — mirrors AdapterKind's 512-byte cap.
        let at_limit = "a".repeat(MAX_RUN_ORIGIN_ADAPTER_BYTES);
        assert!(
            RunOriginAdapter::new(at_limit).is_ok(),
            "adapter at exactly {MAX_RUN_ORIGIN_ADAPTER_BYTES} bytes must be accepted"
        );
    }

    #[test]
    fn run_origin_adapter_rejects_over_512_bytes() {
        let overlong = "a".repeat(MAX_RUN_ORIGIN_ADAPTER_BYTES + 1);
        assert!(
            RunOriginAdapter::new(overlong).is_err(),
            "adapter exceeding {MAX_RUN_ORIGIN_ADAPTER_BYTES} bytes must be rejected"
        );
    }

    #[test]
    fn deserialize_rejects_empty_adapter_in_product_turn_context() {
        // The try_from serde gate must reject persisted payloads with an empty
        // adapter string — the same invariant that new() enforces.
        let json = r#"{
            "origin": "inbound",
            "adapter": "",
            "owner": {"kind": "personal", "user": "u1"}
        }"#;
        assert!(
            serde_json::from_str::<ProductTurnContext>(json).is_err(),
            "empty adapter must fail deserialization via try_from"
        );
    }

    #[test]
    fn deserialize_rejects_empty_source_channel_in_product_turn_context() {
        let json = r#"{
            "origin": "web_ui",
            "source_channel": "",
            "owner": {"kind": "personal", "user": "u1"}
        }"#;
        assert!(
            serde_json::from_str::<ProductTurnContext>(json).is_err(),
            "empty source_channel must fail deserialization via try_from"
        );
    }

    #[test]
    fn deserialize_rejects_overlong_run_origin_adapter() {
        // The try_from serde gate must also reject persisted payloads whose adapter
        // exceeds the max — the >512 branch that the direct constructor test covers
        // but the serde boundary did not.
        let overlong = "a".repeat(MAX_RUN_ORIGIN_ADAPTER_BYTES + 1);
        let json = format!(
            r#"{{"origin":"inbound","adapter":"{overlong}","owner":{{"kind":"personal","user":"u1"}}}}"#
        );
        assert!(
            serde_json::from_str::<ProductTurnContext>(&json).is_err(),
            "adapter exceeding {MAX_RUN_ORIGIN_ADAPTER_BYTES} bytes must fail deserialization via try_from"
        );
    }
}
