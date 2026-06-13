//! Single owner of turn-origin/surface/owner classification at ingress.

use ironclaw_turns::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
};

/// Ingress trust level. Callers map their policy (e.g. `BindingResolutionPolicy`) onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    Trusted,
    Untrusted,
}

/// Resolve an inbound submission into a generic product context.
///
/// `ScheduledTrigger` is minted ONLY when `trust == Trusted && is_trigger_adapter`.
/// Any other combination is `Inbound` — an untrusted caller cannot mint a trigger origin.
pub fn resolve_inbound(
    trust: TrustLevel,
    is_trigger_adapter: bool,
    adapter: RunOriginAdapter,
    surface_type: Option<TurnSurfaceType>,
    owner: TurnOwner,
) -> ProductTurnContext {
    let origin = if trust == TrustLevel::Trusted && is_trigger_adapter {
        TurnOriginKind::ScheduledTrigger
    } else {
        TurnOriginKind::Inbound
    };
    ProductTurnContext {
        origin,
        surface_type,
        adapter: Some(adapter),
        owner,
    }
}

/// Resolve a WebUI submission. Always `WebUi`, no adapter/surface.
pub fn resolve_web_ui(owner: TurnOwner) -> ProductTurnContext {
    ProductTurnContext {
        origin: TurnOriginKind::WebUi,
        surface_type: None,
        adapter: None,
        owner,
    }
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
        let ctx = resolve_inbound(TrustLevel::Trusted, true, adapter(), None, owner());
        assert_eq!(ctx.origin, TurnOriginKind::ScheduledTrigger);
    }

    #[test]
    fn untrusted_trigger_adapter_yields_inbound_not_trigger() {
        let ctx = resolve_inbound(TrustLevel::Untrusted, true, adapter(), None, owner());
        assert_eq!(ctx.origin, TurnOriginKind::Inbound);
    }

    #[test]
    fn trusted_non_trigger_adapter_yields_inbound() {
        let a = RunOriginAdapter::new("telegram").unwrap();
        let ctx = resolve_inbound(
            TrustLevel::Trusted,
            false,
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
