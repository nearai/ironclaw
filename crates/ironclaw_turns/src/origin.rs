use serde::{Deserialize, Serialize};

use ironclaw_host_api::turn::{RunOriginAdapter, TurnOwner};

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

/// Generic, persisted product context for one turn. Resolved once at ingress by
/// `ironclaw_turns::product_context`; rendered into the model-visible runtime context.
///
/// **Intended mint points** are the resolver functions in `ironclaw_turns::product_context`:
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
/// `ironclaw_turns::product_context::resolve_inbound`).
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
    use ironclaw_host_api::turn::MAX_RUN_ORIGIN_ADAPTER_BYTES;

    #[test]
    fn product_turn_context_round_trips_through_json() {
        let ctx = ProductTurnContext::new(
            TurnOriginKind::Inbound,
            Some(TurnSurfaceType::Channel),
            Some(RunOriginAdapter::new("telegram").unwrap()),
            TurnOwner::Personal {
                user: ironclaw_host_api::ids::UserId::new("u1").unwrap(),
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
                user: ironclaw_host_api::ids::UserId::new("u1").unwrap(),
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
