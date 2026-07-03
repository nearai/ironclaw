//! Single owner of turn-origin/surface/owner classification at ingress.

use ironclaw_turns::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
};

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
    let origin = match classification {
        InboundClassification::TrustedTrigger => TurnOriginKind::ScheduledTrigger,
        InboundClassification::TrustedOther | InboundClassification::Untrusted => {
            TurnOriginKind::Inbound
        }
    };
    ProductTurnContext::new(origin, surface_type, Some(adapter), owner)
}

/// Resolve a WebUI submission. Always `WebUi`, no adapter/surface.
pub fn resolve_web_ui(owner: TurnOwner) -> ProductTurnContext {
    ProductTurnContext::new(TurnOriginKind::WebUi, None, None, owner)
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
    }

    #[test]
    fn web_ui_yields_web_ui_origin_no_adapter() {
        let ctx = resolve_web_ui(owner());
        assert_eq!(ctx.origin, TurnOriginKind::WebUi);
        assert!(ctx.adapter.is_none());
    }
}
