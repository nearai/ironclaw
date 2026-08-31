//! The authenticated HTTP notification operations (2026-08-13 design §7.8).
//!
//! Intents, acknowledgements, and thread-read evidence are ordinary product
//! mutations behind `ProductSurface::invoke` — never WebSocket frames. The
//! server derives user/tenant authority from the bound caller and answers
//! `NotFound` for foreign notices; client claims cannot mint permission or
//! a target. Every write is idempotent: duplicates return the existing
//! outcome.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_product_contracts::run_completions::{
    RUN_COMPLETION_OPAQUE_ID_MAX_BYTES, RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT,
    RunCompletionAcknowledgeOutcome, RunCompletionAcknowledgeRequest, RunCompletionIntentKind,
    RunCompletionIntentRequest, RunCompletionMutationResponse, RunCompletionThreadReadRequest,
    RunCompletionUnreadResponse,
};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode,
};

use super::RunCompletionSurfaceServices;
use super::ingest::ARBITRATION_WINDOW_MS;
use super::records::{CompletionIntentRecord, CompletionReadEvidence, RunCompletionNotice};
use super::store::{RunCompletionOwner, RunCompletionStoreError};

fn owner_of(caller: &ProductSurfaceCaller) -> RunCompletionOwner {
    RunCompletionOwner {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
    }
}

fn bounded_opaque_id(field: &'static str, value: &str) -> Result<(), ProductSurfaceError> {
    if value.is_empty() || value.len() > RUN_COMPLETION_OPAQUE_ID_MAX_BYTES {
        return Err(ProductSurfaceError::validation(
            field,
            ProductSurfaceValidationCode::InvalidValue,
        ));
    }
    Ok(())
}

fn surface_error(error: RunCompletionStoreError) -> ProductSurfaceError {
    match error {
        RunCompletionStoreError::NotFound => ProductSurfaceError::not_found(),
        RunCompletionStoreError::Invalid { .. } => {
            ProductSurfaceError::validation("input", ProductSurfaceValidationCode::InvalidValue)
        }
        RunCompletionStoreError::Conflict { .. } => ProductSurfaceError::from_status(
            ironclaw_product_contracts::surface::ProductSurfaceErrorCode::Conflict,
            409,
            true,
        ),
        RunCompletionStoreError::Unavailable { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::run_completions",
                %reason,
                "run completion store unavailable",
            );
            ProductSurfaceError::unavailable(true)
        }
    }
}

/// Load the caller's own notice; foreign and missing collapse to `NotFound`.
async fn owned_notice(
    services: &RunCompletionSurfaceServices,
    owner: &RunCompletionOwner,
    notice_id: &str,
) -> Result<RunCompletionNotice, ProductSurfaceError> {
    services
        .notices
        .get(owner, notice_id)
        .await
        .map_err(surface_error)?
        .ok_or_else(ProductSurfaceError::not_found)
}

/// `webui.run_completion.intent.v1`
pub async fn submit_intent(
    services: &Arc<RunCompletionSurfaceServices>,
    caller: ProductSurfaceCaller,
    request: RunCompletionIntentRequest,
) -> Result<RunCompletionMutationResponse, ProductSurfaceError> {
    bounded_opaque_id("notice_id", &request.notice_id)?;
    bounded_opaque_id("browser_instance_id", &request.browser_instance_id)?;
    bounded_opaque_id("tab_id", &request.tab_id)?;
    let owner = owner_of(&caller);
    let notice = owned_notice(services, &owner, &request.notice_id).await?;
    if notice.is_read() {
        // Already settled elsewhere: idempotent no-op.
        return Ok(RunCompletionMutationResponse {
            settled_notice_ids: vec![notice.notice_id],
        });
    }
    if request.intent == RunCompletionIntentKind::ReplyObserved {
        // Exact reply-render evidence settles the notice without any
        // presentation (§6.1 row 1).
        let read = services
            .notices
            .mark_read(
                &owner,
                &request.notice_id,
                CompletionReadEvidence::ReplyRendered {
                    browser_instance_id: request.browser_instance_id.clone(),
                },
                Utc::now(),
            )
            .await
            .map_err(surface_error)?;
        services.hub.publish_clear(&owner, &read);
        services.settle_inbox_row(&owner, &read.run_id).await;
        services.wake_owner(&owner);
        return Ok(RunCompletionMutationResponse {
            settled_notice_ids: vec![read.notice_id],
        });
    }
    services
        .notices
        .record_intent(
            &owner,
            &request.notice_id,
            CompletionIntentRecord {
                browser_instance_id: request.browser_instance_id,
                tab_id: request.tab_id,
                state_revision: request.state_revision,
                focus_epoch: request.focus_epoch,
                intent: request.intent,
                offered_at: Utc::now(),
            },
        )
        .await
        .map_err(surface_error)?;
    services.wake_owner(&owner);
    Ok(RunCompletionMutationResponse {
        settled_notice_ids: Vec::new(),
    })
}

/// `webui.run_completion.acknowledge.v1`
pub async fn acknowledge(
    services: &Arc<RunCompletionSurfaceServices>,
    caller: ProductSurfaceCaller,
    request: RunCompletionAcknowledgeRequest,
) -> Result<RunCompletionMutationResponse, ProductSurfaceError> {
    bounded_opaque_id("notice_id", &request.notice_id)?;
    bounded_opaque_id("grant_id", &request.grant_id)?;
    let owner = owner_of(&caller);
    let notice = owned_notice(services, &owner, &request.notice_id).await?;
    match request.outcome {
        RunCompletionAcknowledgeOutcome::ReplyRendered => {
            // Read evidence must name the browser the grant was issued to; a
            // stale or foreign grant id cannot mint evidence with a forged
            // empty identity. The acknowledger falls back to the
            // reply-observed intent path, which carries its own identity.
            let Some(browser_instance_id) = granted_browser(&notice, &request.grant_id) else {
                return Err(ProductSurfaceError::from_status(
                    ironclaw_product_contracts::surface::ProductSurfaceErrorCode::Conflict,
                    409,
                    true,
                ));
            };
            let read = services
                .notices
                .mark_read(
                    &owner,
                    &request.notice_id,
                    CompletionReadEvidence::ReplyRendered {
                        browser_instance_id,
                    },
                    Utc::now(),
                )
                .await
                .map_err(surface_error)?;
            services.hub.publish_clear(&owner, &read);
            services.settle_inbox_row(&owner, &read.run_id).await;
            services.wake_owner(&owner);
            Ok(RunCompletionMutationResponse {
                settled_notice_ids: vec![read.notice_id],
            })
        }
        RunCompletionAcknowledgeOutcome::Presented => {
            let presented = services
                .notices
                .acknowledge_presented(&owner, &request.notice_id, &request.grant_id, Utc::now())
                .await
                .map_err(surface_error)?;
            services.wake_owner(&owner);
            Ok(RunCompletionMutationResponse {
                settled_notice_ids: vec![presented.notice_id],
            })
        }
        RunCompletionAcknowledgeOutcome::StaleState
        | RunCompletionAcknowledgeOutcome::EffectFailed => {
            // One re-arbitration follows (§5.4); the coordinator's due-work
            // loop owns the fallback decision after that.
            services.record_stale_grant();
            tracing::debug!(
                target: "ironclaw::reborn::run_completions",
                outcome = ?request.outcome,
                stale_grants = services.stale_grant_count(),
                "grant regressed by browser acknowledgement",
            );
            let closes_at = Utc::now() + ChronoDuration::milliseconds(ARBITRATION_WINDOW_MS);
            let regressed = services
                .notices
                .regress_expired_grant(&owner, &request.notice_id, &request.grant_id, closes_at)
                .await
                .map_err(surface_error)?;
            services.wake_owner(&owner);
            Ok(RunCompletionMutationResponse {
                settled_notice_ids: vec![regressed.notice_id],
            })
        }
    }
}

/// The browser the outstanding grant names, or `None` when `grant_id` does
/// not match the outstanding grant (stale, replaced, or foreign).
fn granted_browser(notice: &RunCompletionNotice, grant_id: &str) -> Option<String> {
    match &notice.delivery {
        super::records::CompletionDeliveryState::Granted {
            grant_id: outstanding,
            browser_instance_id,
            ..
        } if outstanding == grant_id => Some(browser_instance_id.clone()),
        _ => None,
    }
}

/// `webui.run_completion.thread_read.v1`
pub async fn thread_read(
    services: &Arc<RunCompletionSurfaceServices>,
    caller: ProductSurfaceCaller,
    request: RunCompletionThreadReadRequest,
) -> Result<RunCompletionMutationResponse, ProductSurfaceError> {
    bounded_opaque_id("thread_id", &request.thread_id)?;
    bounded_opaque_id("browser_instance_id", &request.browser_instance_id)?;
    let through_sequence: u64 = request.through_sequence.parse().map_err(|_| {
        ProductSurfaceError::validation(
            "through_sequence",
            ProductSurfaceValidationCode::InvalidValue,
        )
    })?;
    let owner = owner_of(&caller);
    let unread = services
        .notices
        .unread_for_thread(
            &owner,
            &request.thread_id,
            RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT,
        )
        .await
        .map_err(surface_error)?;
    let mut settled = Vec::new();
    for notice in unread {
        // Advance only through notices at or below the rendered sequence;
        // later completions the view has not proven remain unread (§7.8).
        if notice.sequence > through_sequence {
            continue;
        }
        let read = services
            .notices
            .mark_read(
                &owner,
                &notice.notice_id,
                CompletionReadEvidence::FocusedThreadVisit {
                    browser_instance_id: request.browser_instance_id.clone(),
                },
                Utc::now(),
            )
            .await
            .map_err(surface_error)?;
        services.hub.publish_clear(&owner, &read);
        services.settle_inbox_row(&owner, &read.run_id).await;
        settled.push(read.notice_id);
    }
    if !settled.is_empty() {
        services.wake_owner(&owner);
    }
    Ok(RunCompletionMutationResponse {
        settled_notice_ids: settled,
    })
}

/// `webui.run-completions.unread.v1`
pub async fn unread_view(
    services: &Arc<RunCompletionSurfaceServices>,
    caller: ProductSurfaceCaller,
) -> Result<RunCompletionUnreadResponse, ProductSurfaceError> {
    let owner = owner_of(&caller);
    let unread = services
        .notices
        .unread_snapshot(&owner)
        .await
        .map_err(surface_error)?;
    let mut notices = Vec::with_capacity(unread.len());
    for notice in &unread {
        notices.push(services.hub.notice_event(&owner, notice).await);
    }
    // The owner's stream head, not the unread maximum: resuming from an
    // unread-only maximum would replay newer already-read notices, and an
    // empty snapshot would reset the subscription to origin.
    let resume_sequence = services
        .notices
        .head_sequence(&owner)
        .await
        .map_err(surface_error)?;
    Ok(RunCompletionUnreadResponse {
        notices,
        resume_sequence: resume_sequence.to_string(),
    })
}
