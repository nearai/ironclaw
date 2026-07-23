//! Single owner of turn-origin/surface/owner classification at ingress.

use ironclaw_turns::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
};

pub const WEBUI_SOURCE_CHANNEL: &str = "webui";
pub const CLI_SOURCE_CHANNEL: &str = "cli";

/// Ingress classification. Callers collapse their (trust policy, trigger-adapter) signal
/// into one value, so the resolver cannot receive a contradictory pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundClassification {
    /// Trusted ingress whose adapter is the trusted-trigger adapter.
    TrustedTrigger,
    /// Trusted ingress, non-trigger adapter.
    TrustedOther,
    /// Untrusted ingress (adapter identity is irrelevant — never a trigger).
    Untrusted,
}

/// Resolve an inbound submission into a generic product context.
///
/// `ScheduledTrigger` is minted ONLY when `classification == TrustedTrigger`.
/// Any other combination yields `Inbound` — an untrusted caller cannot mint a trigger origin.
pub fn resolve_inbound(
    classification: InboundClassification,
    adapter: RunOriginAdapter,
    surface_type: Option<TurnSurfaceType>,
    owner: TurnOwner,
) -> ProductTurnContext {
    resolve_inbound_with_source_channel(classification, adapter, None, surface_type, owner)
}

/// Resolve an inbound submission with an explicit product source channel.
/// When omitted, the source channel defaults to the adapter identity.
pub fn resolve_inbound_with_source_channel(
    classification: InboundClassification,
    adapter: RunOriginAdapter,
    source_channel: Option<RunOriginAdapter>,
    surface_type: Option<TurnSurfaceType>,
    owner: TurnOwner,
) -> ProductTurnContext {
    let origin = match classification {
        InboundClassification::TrustedTrigger => TurnOriginKind::ScheduledTrigger,
        InboundClassification::TrustedOther | InboundClassification::Untrusted => {
            TurnOriginKind::Inbound
        }
    };
    let source_channel = source_channel.or_else(|| Some(adapter.clone()));
    ProductTurnContext::new_with_source_channel(
        origin,
        surface_type,
        Some(adapter),
        source_channel,
        owner,
    )
}

/// Resolve a WebUI submission. Always `WebUi`, no adapter/surface, source channel `webui`.
pub fn resolve_web_ui(owner: TurnOwner) -> ProductTurnContext {
    ProductTurnContext::new_with_source_channel(
        TurnOriginKind::WebUi,
        None,
        None,
        source_channel(WEBUI_SOURCE_CHANNEL),
        owner,
    )
}

/// Resolve a local CLI submission. It follows the first-party chat origin path
/// while preserving `cli` as the source channel for downstream rendering and
/// product-surface accounting.
pub fn resolve_cli(owner: TurnOwner) -> ProductTurnContext {
    ProductTurnContext::new_with_source_channel(
        TurnOriginKind::WebUi,
        None,
        None,
        source_channel(CLI_SOURCE_CHANNEL),
        owner,
    )
}

fn source_channel(value: &'static str) -> Option<RunOriginAdapter> {
    RunOriginAdapter::new(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::UserId;

    fn owner() -> TurnOwner {
        TurnOwner::Personal {
            user: UserId::new("u1").unwrap(),
        }
    }
    fn adapter() -> RunOriginAdapter {
        RunOriginAdapter::new("trigger").unwrap()
    }

    #[test]
    fn trusted_trigger_adapter_yields_scheduled_trigger() {
        let ctx = resolve_inbound(
            InboundClassification::TrustedTrigger,
            adapter(),
            None,
            owner(),
        );
        assert_eq!(ctx.origin, TurnOriginKind::ScheduledTrigger);
        assert_eq!(
            ctx.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some("trigger")
        );
    }

    #[test]
    fn untrusted_trigger_adapter_yields_inbound_not_trigger() {
        let ctx = resolve_inbound(InboundClassification::Untrusted, adapter(), None, owner());
        assert_eq!(ctx.origin, TurnOriginKind::Inbound);
    }

    #[test]
    fn trusted_non_trigger_adapter_yields_inbound() {
        let a = RunOriginAdapter::new("telegram").unwrap();
        let ctx = resolve_inbound(
            InboundClassification::TrustedOther,
            a,
            Some(TurnSurfaceType::Channel),
            owner(),
        );
        assert_eq!(ctx.origin, TurnOriginKind::Inbound);
        assert_eq!(ctx.surface_type, Some(TurnSurfaceType::Channel));
        assert_eq!(
            ctx.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some("telegram")
        );
    }

    #[test]
    fn web_ui_yields_web_ui_origin_no_adapter_with_source_channel() {
        let ctx = resolve_web_ui(owner());
        assert_eq!(ctx.origin, TurnOriginKind::WebUi);
        assert!(ctx.adapter.is_none());
        assert_eq!(
            ctx.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some(WEBUI_SOURCE_CHANNEL)
        );
    }

    #[test]
    fn cli_yields_web_ui_origin_no_adapter_with_cli_source_channel() {
        let ctx = resolve_cli(owner());
        assert_eq!(ctx.origin, TurnOriginKind::WebUi);
        assert!(ctx.adapter.is_none());
        assert_eq!(
            ctx.source_channel.as_ref().map(RunOriginAdapter::as_str),
            Some(CLI_SOURCE_CHANNEL)
        );
    }
}
