//! In-process `OutboundPreferencesProductFacade` double for the C-SYNTH outbound
//! seam.
//!
//! Substitutes ONLY at the production-wired facade trait seam the two synthetic
//! `outbound_delivery_*` capabilities consume (see
//! `ironclaw_reborn_composition::runtime::local_dev::outbound_delivery`). Holds a
//! fixed in-memory target inventory; `set_outbound_preferences` succeeds when the
//! requested target is in the inventory and returns `NotFound` otherwise — so the
//! same double drives BOTH the happy-path (`set` a known target) and the
//! `invalid_input`/`NotFound` reject route (`set` an unknown target) without a
//! per-test facade config.
//!
//! Distinct from `delivery::RecordingOutboundDeliverySink` (the final-reply
//! delivery sink); this is the delivery-*preference* facade.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryModality,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetStatus, RebornOutboundDeliveryTargetSummary,
    RebornOutboundPreferencesResponse, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
};

/// A fixed in-memory `OutboundPreferencesProductFacade` double.
///
/// Stateful: `set_outbound_preferences` accepting a known target updates
/// `last_accepted`, and `get_outbound_preferences` reads it back — so a test
/// can prove a `set` actually persisted (not just that the setter's own
/// response echoed the request) by reading it back through a *different*
/// facade method.
pub(crate) struct FakeOutboundPreferencesFacade {
    targets: Vec<RebornOutboundDeliveryTargetOption>,
    set_calls: Mutex<Vec<RebornOutboundDeliveryTargetId>>,
    last_accepted: Mutex<Option<RebornOutboundDeliveryTargetSummary>>,
}

impl FakeOutboundPreferencesFacade {
    /// Seed a double whose inventory carries two Slack targets. A `target_set`
    /// call for either id resolves; any other id surfaces as `NotFound`.
    pub(crate) fn with_default_targets() -> Arc<Self> {
        Arc::new(Self {
            targets: vec![
                target_option("slack:dm:alpha", "Slack DM Alpha"),
                target_option("slack:channel:beta", "Slack Channel Beta"),
            ],
            set_calls: Mutex::new(Vec::new()),
            last_accepted: Mutex::new(None),
        })
    }

    /// The target ids passed to `set_outbound_preferences`, in call order —
    /// read-back that a `Completed` outcome actually reached the facade seam (a
    /// no-op set that still fabricated a success payload would leave this empty).
    pub(crate) fn recorded_set_target_ids(&self) -> Vec<String> {
        self.set_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let last_accepted = self
            .last_accepted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        _caller: WebUiAuthenticatedCaller,
        request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        let Some(target_id) = request.final_reply_target_id else {
            return Err(target_not_found());
        };
        let Some(summary) = self.find_target(&target_id).cloned() else {
            return Err(target_not_found());
        };
        self.set_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(target_id);
        *self
            .last_accepted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(summary.clone());
        Ok(RebornOutboundPreferencesResponse {
            final_reply_target: Some(summary),
            final_reply_target_status: RebornOutboundDeliveryTargetStatus::Available,
            default_modality: RebornOutboundDeliveryModality::Text,
        })
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
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

/// The `NotFound` the production handler maps to
/// `CapabilityOutcome::Failed(InvalidInput)` ("outbound delivery target is not
/// available") — see `outbound_delivery.rs:212`.
fn target_not_found() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::NotFound,
        kind: RebornServicesErrorKind::NotFound,
        status_code: 404,
        retryable: false,
        field: None,
        validation_code: None,
    }
}
