//! In-process `OutboundPreferencesProductFacade` double for the C-SYNTH seam
//! (`ironclaw_reborn_composition::runtime::local_dev::outbound_delivery`). Fixed
//! in-memory inventory: succeeds for a known target, `NotFound` otherwise — one
//! double drives both the happy path and the reject route without per-test
//! config. Distinct from `delivery::RecordingOutboundDeliverySink` (the
//! final-reply delivery sink; this is the delivery-*preference* facade).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};
use ironclaw_product::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesResponse, RebornSetOutboundPreferencesRequest,
};

/// Bundled behind ONE mutex (not two) so a reader never observes `set_calls`
/// and `last_accepted` out of sync.
#[derive(Default)]
struct FakeOutboundState {
    set_calls: Vec<RebornOutboundDeliveryTargetId>,
    last_accepted: Option<RebornOutboundDeliveryTargetSummary>,
}

/// Fixed in-memory `OutboundPreferencesProductFacade` double. Stateful:
/// `set_outbound_preferences` updates `last_accepted`, and
/// `get_outbound_preferences` reads it back — proves a `set` persisted via a
/// different facade method, not just an echo.
pub(crate) struct FakeOutboundPreferencesFacade {
    targets: Vec<RebornOutboundDeliveryTargetOption>,
    state: Mutex<FakeOutboundState>,
}

impl FakeOutboundPreferencesFacade {
    /// Seed a double whose inventory carries two Slack targets plus the
    /// host-owned WebApp destination the production registry always exposes
    /// (`host_owned_outbound_delivery_target_registry` in composition — the
    /// double must mirror that contract or web_app routing tests diverge from
    /// production). A `target_set` call for a listed id resolves; any other id
    /// surfaces as `NotFound`.
    pub(crate) fn with_default_targets() -> Arc<Self> {
        Arc::new(Self {
            targets: vec![
                target_option("slack:dm:alpha", "Slack DM Alpha"),
                target_option("slack:channel:beta", "Slack Channel Beta"),
                ironclaw_product::web_app_outbound_delivery_target_option()
                    .expect("host-owned web_app target option"),
            ],
            state: Mutex::new(FakeOutboundState::default()),
        })
    }

    /// Target ids passed to `set_outbound_preferences`, in call order — proves a
    /// `Completed` outcome reached the facade (a no-op set would leave this empty).
    pub(crate) fn recorded_set_target_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_calls
            .iter()
            .map(|id| id.as_str().to_string())
            .collect()
    }

    fn find_target(
        &self,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Option<&RebornOutboundDeliveryTargetSummary> {
        self.targets
            .iter()
            .map(|option| &option.target)
            .find(|summary| summary.target_id == *target_id)
    }
}

#[async_trait]
impl OutboundPreferencesProductFacade for FakeOutboundPreferencesFacade {
    async fn get_outbound_preferences(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundPreferencesResponse, ProductSurfaceError> {
        let last_accepted = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last_accepted
            .clone();
        match last_accepted {
            Some(summary) => Ok(RebornOutboundPreferencesResponse {
                final_reply_target: Some(summary),
                final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
                default_modality: RebornOutboundDeliveryModality::Text,
            }),
            None => Ok(RebornOutboundPreferencesResponse::default()),
        }
    }

    async fn set_outbound_preferences(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, ProductSurfaceError> {
        let Some(target_id) = request.final_reply_target_id else {
            return Err(target_not_found());
        };
        let Some(summary) = self.find_target(&target_id).cloned() else {
            return Err(target_not_found());
        };
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.set_calls.push(target_id);
            state.last_accepted = Some(summary.clone());
        }
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: Some(summary),
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets: self.targets.clone(),
            next_cursor: None,
        })
    }
}

fn target_option(target_id: &str, display_name: &str) -> RebornOutboundDeliveryTargetOption {
    RebornOutboundDeliveryTargetOption {
        target: RebornOutboundDeliveryTargetSummary::new(
            RebornOutboundDeliveryTargetId::new(target_id).expect("valid target id"),
            "slack",
            display_name,
            Some(format!("{display_name} (test)")),
        )
        .expect("valid target summary"),
        capabilities: RebornOutboundDeliveryTargetCapabilities {
            final_replies: true,
            gate_prompts: true,
            auth_prompts: true,
        },
    }
}

/// The `NotFound` the production handler maps to `Failed(InvalidInput)` — see
/// `OutboundDeliveryTargetSetHandler`'s `NotFound` arm in
/// `runtime/local_dev/outbound_delivery.rs`.
fn target_not_found() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::NotFound,
        kind: ProductSurfaceErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: None,
        validation_code: None,
    }
}
