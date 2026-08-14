//! In-process `OutboundPreferencesProductService` double for the C-SYNTH seam
//! (`ironclaw_composition::runtime::standalone::outbound_delivery`). Fixed
//! in-memory inventory: succeeds for a known target, `NotFound` otherwise — one
//! double drives both the happy path and the reject route without per-test
//! config. Distinct from `delivery::RecordingOutboundDeliverySink` (the
//! final-reply delivery sink; this is the delivery-*preference* service).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_assistant::{
    OutboundPreferencesProductService, RebornNotificationChannel,
    RebornNotificationChannelsResponse, RebornOutboundDeliveryTargetCapabilities,
    RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetListResponse,
    RebornOutboundDeliveryTargetOption, RebornOutboundDeliveryTargetStatus,
    RebornOutboundDeliveryTargetSummary, RebornSetNotificationChannelsRequest,
};
use ironclaw_outbound::NOTIFICATION_TARGETS_CAP;
use ironclaw_product_contracts::surface::ProductSurfaceValidationCode;
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};

#[derive(Default)]
struct FakeOutboundState {
    /// Last full-replace notification-channel set applied through
    /// `set_notification_channels` (dedup-preserving-order already applied).
    notification_channel_ids: Vec<RebornOutboundDeliveryTargetId>,
}

/// Fixed in-memory `OutboundPreferencesProductService` double. Stateful:
/// `set_notification_channels` records the applied set and
/// `get_notification_channels` reads it back — proving a `set` persisted via a
/// different service method, not just an echo.
pub(crate) struct FakeOutboundPreferencesService {
    targets: Vec<RebornOutboundDeliveryTargetOption>,
    state: Mutex<FakeOutboundState>,
}

impl FakeOutboundPreferencesService {
    /// Seed a double whose inventory carries two Slack targets. A
    /// `notification_channels_set` call for either id resolves; any other id
    /// surfaces as `NotFound`.
    pub(crate) fn with_default_targets() -> Arc<Self> {
        Arc::new(Self {
            targets: vec![
                target_option("slack:dm:alpha", "Slack DM Alpha"),
                target_option("slack:channel:beta", "Slack Channel Beta"),
            ],
            state: Mutex::new(FakeOutboundState::default()),
        })
    }

    /// The stored notification-channel set after the most recent
    /// `set_notification_channels` call — proves a `Completed`/applied outcome
    /// actually reached the service seam (a no-op set would leave this at its
    /// prior value, and a never-called service would leave it empty).
    pub(crate) fn recorded_notification_channel_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .notification_channel_ids
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
impl OutboundPreferencesProductService for FakeOutboundPreferencesService {
    async fn list_outbound_delivery_targets(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets: self.targets.clone(),
            next_cursor: None,
        })
    }

    async fn set_notification_channels(
        &self,
        _caller: ProductSurfaceCaller,
        request: RebornSetNotificationChannelsRequest,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<_> = request
            .target_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if deduped.len() > NOTIFICATION_TARGETS_CAP {
            return Err(too_many_notification_targets());
        }
        let mut channels = Vec::with_capacity(deduped.len());
        for id in &deduped {
            let target = self.find_target(id).cloned().ok_or_else(target_not_found)?;
            channels.push(RebornNotificationChannel {
                target_id: id.clone(),
                status: RebornOutboundDeliveryTargetStatus::Available,
                option: Some(RebornOutboundDeliveryTargetOption {
                    target,
                    capabilities: RebornOutboundDeliveryTargetCapabilities {
                        final_replies: true,
                        gate_prompts: true,
                        auth_prompts: true,
                        notifications: true,
                    },
                }),
            });
        }
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.notification_channel_ids = deduped;
        }
        Ok(RebornNotificationChannelsResponse { channels })
    }

    async fn get_notification_channels(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornNotificationChannelsResponse, ProductSurfaceError> {
        let ids = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .notification_channel_ids
            .clone();
        // Mirrors the production service: a stored id that no longer resolves
        // is represented as `Unavailable` (no `option`), never dropped from
        // the list.
        let channels = ids
            .iter()
            .map(|id| match self.find_target(id).cloned() {
                Some(target) => RebornNotificationChannel {
                    target_id: id.clone(),
                    status: RebornOutboundDeliveryTargetStatus::Available,
                    option: Some(RebornOutboundDeliveryTargetOption {
                        target,
                        capabilities: RebornOutboundDeliveryTargetCapabilities {
                            final_replies: true,
                            gate_prompts: true,
                            auth_prompts: true,
                            notifications: true,
                        },
                    }),
                },
                None => RebornNotificationChannel {
                    target_id: id.clone(),
                    status: RebornOutboundDeliveryTargetStatus::Unavailable,
                    option: None,
                },
            })
            .collect();
        Ok(RebornNotificationChannelsResponse { channels })
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
            notifications: true,
        },
    }
}

/// The `NotFound` the production handler maps to `Failed(InvalidInput)` — see
/// `OutboundDeliveryTargetSetHandler`'s `NotFound` arm in
/// `runtime/standalone/outbound_delivery.rs`.
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

/// Mirrors the shape `RebornOutboundPreferencesService::set_notification_channels`
/// returns for a `target_ids` list exceeding `NOTIFICATION_TARGETS_CAP` — an
/// `InvalidRequest`/`Validation` error naming the `target_ids` field, which
/// `outbound_delivery_outcome` maps to a model-visible `Failed(InvalidInput)`.
fn too_many_notification_targets() -> ProductSurfaceError {
    ProductSurfaceError {
        code: ProductSurfaceErrorCode::InvalidRequest,
        kind: ProductSurfaceErrorKind::Validation,
        status_code: 400,
        retryable: false,
        field: Some("target_ids".to_string()),
        validation_code: Some(ProductSurfaceValidationCode::TooLong),
    }
}
