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

use super::coordinator::ARBITRATION_WINDOW_MS;
use super::records::{CompletionIntentRecord, CompletionReadEvidence, RunCompletionNotice};
use super::store::{RunCompletionOwner, RunCompletionStoreError};
use super::{RunCompletionSurfaceServices, TRACE_TARGET};

/// Bounded number of unread pages one `thread_read` request drains
/// (`RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT` notices each), so a single request
/// cannot settle an unbounded backlog.
const MAX_THREAD_READ_PAGES: usize = 16;

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

/// The one translation from store failures to the public surface error
/// contract, shared by the HTTP operations and the `RunCompletions` stream
/// selector so the two can never drift.
/// Settle one notice on read evidence: the durable read transition, the
/// live clear frame, and the Inbox read bridge, in that order. Read is the
/// only thing that clears surfaces (design principle 3), so every read path
/// funnels through here rather than repeating the sequence.
async fn settle_read(
    services: &Arc<RunCompletionSurfaceServices>,
    owner: &RunCompletionOwner,
    notice_id: &str,
    evidence: CompletionReadEvidence,
) -> Result<RunCompletionNotice, ProductSurfaceError> {
    let read = services
        .notices
        .mark_read(owner, notice_id, evidence, Utc::now())
        .await
        .map_err(surface_error)?;
    services.hub.publish_clear(owner, &read);
    services.settle_inbox_row(owner, &read.run_id).await;
    Ok(read)
}

pub(crate) fn surface_error(error: RunCompletionStoreError) -> ProductSurfaceError {
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
                target: TRACE_TARGET,
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
        let read = settle_read(
            services,
            &owner,
            &request.notice_id,
            CompletionReadEvidence::ReplyRendered {
                browser_instance_id: request.browser_instance_id.clone(),
            },
        )
        .await?;
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
            let read = settle_read(
                services,
                &owner,
                &request.notice_id,
                CompletionReadEvidence::ReplyRendered {
                    browser_instance_id,
                },
            )
            .await?;
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
                target: TRACE_TARGET,
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
    let through_sequence: u64 = request.through_sequence.parse().map_err(|error| {
        // Sanitized for the client; cause retained server-side.
        tracing::debug!(target: TRACE_TARGET, %error, "through_sequence rejected");
        ProductSurfaceError::validation(
            "through_sequence",
            ProductSurfaceValidationCode::InvalidValue,
        )
    })?;
    let owner = owner_of(&caller);
    let mut settled = Vec::new();
    // The unread-per-thread query is oldest-first and bounded; a settled
    // notice leaves the unread partition, so re-querying pages through the
    // backlog until a page is short or every remaining unread notice is
    // newer than the evidence. Bounded so one request cannot drain an
    // unbounded backlog.
    for _ in 0..MAX_THREAD_READ_PAGES {
        let unread = services
            .notices
            .unread_for_thread(
                &owner,
                &request.thread_id,
                RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT,
            )
            .await
            .map_err(surface_error)?;
        let page_len = unread.len();
        let mut settled_this_page = 0;
        for notice in unread {
            // Advance only through notices at or below the rendered
            // sequence; later completions the view has not proven remain
            // unread (§7.8).
            if notice.sequence > through_sequence {
                continue;
            }
            let read = settle_read(
                services,
                &owner,
                &notice.notice_id,
                CompletionReadEvidence::FocusedThreadVisit {
                    browser_instance_id: request.browser_instance_id.clone(),
                },
            )
            .await?;
            settled.push(read.notice_id);
            settled_this_page += 1;
        }
        if page_len < RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT || settled_this_page == 0 {
            break;
        }
    }
    if !settled.is_empty() {
        services.wake_owner(&owner);
    }
    Ok(RunCompletionMutationResponse {
        settled_notice_ids: settled,
    })
}

/// `webui.run_completion.unread.v1`
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
    // Batch projection: one unread-count query per distinct thread, not one
    // per notice, so a full 250-notice snapshot never costs 250 index scans.
    let notices = services.hub.notice_events(&owner, &unread).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_completions::records::{CompletionDeliveryState, CompletionSurface};
    use crate::run_completions::store::{
        NewGrant, NewRunCompletionNotice, RunCompletionNoticeStore, RunCompletionNotices,
    };
    use crate::run_completions::stream::RunCompletionStreamHub;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_product_contracts::surface::ProductSurfaceErrorCode;

    fn services() -> Arc<RunCompletionSurfaceServices> {
        let store = Arc::new(RunCompletionNoticeStore::new(Arc::new(
            ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope: &ResourceScope| {
                MountView::new(vec![
                    MountGrant::new(
                        MountAlias::new(crate::run_completions::store::RUN_NOTICES_MOUNT_ALIAS)?,
                        VirtualPath::new(format!(
                            "/tenants/{}/users/{}/run-notices",
                            scope.tenant_id, scope.user_id
                        ))?,
                        MountPermissions::read_write_list_delete(),
                    ),
                    MountGrant::new(
                        MountAlias::new("/tenant-shared")?,
                        VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id))?,
                        MountPermissions::read_write(),
                    ),
                ])
            }),
        ))) as Arc<dyn RunCompletionNotices>;
        let hub = Arc::new(RunCompletionStreamHub::new(Arc::clone(&store)));
        Arc::new(RunCompletionSurfaceServices::new(
            store,
            hub,
            Arc::new(ironclaw_notifications::NoopNotificationInboxStore),
        ))
    }

    fn caller(user: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    async fn seed(
        services: &RunCompletionSurfaceServices,
        user: &str,
        suffix: &str,
        thread: &str,
    ) -> RunCompletionNotice {
        let owner = owner_of(&caller(user));
        match services
            .notices
            .create_notice(
                &owner,
                NewRunCompletionNotice {
                    notice_id: format!("rcn-{suffix}"),
                    run_id: format!("run-{suffix}"),
                    thread_id: thread.to_string(),
                    agent_id: Some("agent-alpha".to_string()),
                    project_id: None,
                    thread_tag: format!("rct-{thread}"),
                    terminal_projection_ref: format!("run-completion/rcn-{suffix}"),
                    completed_at: Utc::now(),
                    arbitration_closes_at: Utc::now() + ChronoDuration::seconds(1),
                },
            )
            .await
            .expect("create notice")
        {
            crate::run_completions::store::NoticeCreateOutcome::Created(notice)
            | crate::run_completions::store::NoticeCreateOutcome::AlreadyRecorded(notice) => notice,
        }
    }

    #[tokio::test]
    async fn foreign_notices_collapse_to_not_found() {
        let services = services();
        let notice = seed(&services, "user-owner", "owned", "thread-a").await;
        // Another user naming a real notice id learns nothing: not that it
        // exists, not whose it is.
        let error = submit_intent(
            &services,
            caller("user-other"),
            RunCompletionIntentRequest {
                notice_id: notice.notice_id.clone(),
                browser_instance_id: "rbi-1".to_string(),
                tab_id: "rtb-1".to_string(),
                state_revision: 1,
                focus_epoch: 0,
                intent: RunCompletionIntentKind::InApp,
            },
        )
        .await
        .expect_err("a foreign notice is not found");
        assert_eq!(error.code, ProductSurfaceErrorCode::NotFound);
        let error = acknowledge(
            &services,
            caller("user-other"),
            RunCompletionAcknowledgeRequest {
                notice_id: notice.notice_id,
                grant_id: "rcg-1".to_string(),
                state_revision: 1,
                outcome: RunCompletionAcknowledgeOutcome::Presented,
            },
        )
        .await
        .expect_err("a foreign acknowledgement is not found");
        assert_eq!(error.code, ProductSurfaceErrorCode::NotFound);
    }

    #[tokio::test]
    async fn thread_read_settles_every_unread_notice_through_the_rendered_sequence() {
        let services = services();
        // One more than a single unread page, all in one thread, plus one
        // notice in another thread that must stay untouched.
        let backlog = RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT + 1;
        let mut newest = 0;
        for index in 0..backlog {
            let notice = seed(&services, "user-a", &format!("t-{index}"), "thread-a").await;
            newest = newest.max(notice.sequence);
        }
        let other = seed(&services, "user-a", "elsewhere", "thread-b").await;

        let response = thread_read(
            &services,
            caller("user-a"),
            RunCompletionThreadReadRequest {
                thread_id: "thread-a".to_string(),
                through_sequence: newest.to_string(),
                browser_instance_id: "rbi-1".to_string(),
            },
        )
        .await
        .expect("thread read");
        assert_eq!(
            response.settled_notice_ids.len(),
            backlog,
            "a focused visit through the newest sequence drains past one page"
        );
        let owner = owner_of(&caller("user-a"));
        let remaining = services
            .notices
            .unread_snapshot(&owner)
            .await
            .expect("snapshot");
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].notice_id, other.notice_id,
            "other threads stay unread"
        );
    }

    #[tokio::test]
    async fn thread_read_never_settles_past_the_rendered_sequence() {
        let services = services();
        let older = seed(&services, "user-a", "older", "thread-a").await;
        let newer = seed(&services, "user-a", "newer", "thread-a").await;
        let response = thread_read(
            &services,
            caller("user-a"),
            RunCompletionThreadReadRequest {
                thread_id: "thread-a".to_string(),
                through_sequence: older.sequence.to_string(),
                browser_instance_id: "rbi-1".to_string(),
            },
        )
        .await
        .expect("thread read");
        assert_eq!(response.settled_notice_ids, vec![older.notice_id]);
        let owner = owner_of(&caller("user-a"));
        let remaining = services
            .notices
            .unread_snapshot(&owner)
            .await
            .expect("snapshot");
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].notice_id, newer.notice_id,
            "a completion the view has not rendered stays unread"
        );
    }

    #[tokio::test]
    async fn oversized_or_empty_opaque_ids_are_rejected_before_any_store_read() {
        let services = services();
        let oversized = submit_intent(
            &services,
            caller("user-a"),
            RunCompletionIntentRequest {
                notice_id: "rcn-any".to_string(),
                browser_instance_id: "b".repeat(RUN_COMPLETION_OPAQUE_ID_MAX_BYTES + 1),
                tab_id: "tab-1".to_string(),
                state_revision: 1,
                focus_epoch: 0,
                intent: RunCompletionIntentKind::InApp,
            },
        )
        .await
        .expect_err("an oversized browser id is invalid input");
        assert_eq!(oversized.status_code, 400);
        assert_eq!(oversized.field.as_deref(), Some("browser_instance_id"));

        let empty = submit_intent(
            &services,
            caller("user-a"),
            RunCompletionIntentRequest {
                notice_id: "rcn-any".to_string(),
                browser_instance_id: "rbi-1".to_string(),
                tab_id: String::new(),
                state_revision: 1,
                focus_epoch: 0,
                intent: RunCompletionIntentKind::InApp,
            },
        )
        .await
        .expect_err("an empty tab id is invalid input");
        assert_eq!(empty.status_code, 400);
        assert_eq!(empty.field.as_deref(), Some("tab_id"));
    }

    async fn granted(
        services: &RunCompletionSurfaceServices,
        notice: &RunCompletionNotice,
        grant_id: &str,
    ) {
        services
            .notices
            .issue_grant(
                &owner_of(&caller("user-a")),
                &notice.notice_id,
                NewGrant {
                    grant_id: grant_id.to_string(),
                    browser_instance_id: "rbi-1".to_string(),
                    surface: CompletionSurface::InApp,
                    state_revision: 1,
                    expires_at: Utc::now() + ChronoDuration::seconds(2),
                },
            )
            .await
            .expect("grant issued");
    }

    /// Read evidence must name the browser the outstanding grant was issued
    /// to: a stale or foreign grant id is a conflict and mints nothing.
    #[tokio::test]
    async fn reply_rendered_acknowledgement_with_a_stale_grant_id_is_a_conflict() {
        let services = services();
        let notice = seed(&services, "user-a", "ack", "thread-a").await;
        granted(&services, &notice, "rcg-real").await;

        let forged = acknowledge(
            &services,
            caller("user-a"),
            RunCompletionAcknowledgeRequest {
                notice_id: notice.notice_id.clone(),
                grant_id: "rcg-forged".to_string(),
                state_revision: 1,
                outcome: RunCompletionAcknowledgeOutcome::ReplyRendered,
            },
        )
        .await
        .expect_err("a grant id that is not the outstanding grant cannot mint read evidence");
        assert_eq!(forged.status_code, 409);
        let untouched = services
            .notices
            .get(&owner_of(&caller("user-a")), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(!untouched.is_read(), "the notice stays unread");
        assert!(matches!(
            untouched.delivery,
            CompletionDeliveryState::Granted { .. }
        ));

        let settled = acknowledge(
            &services,
            caller("user-a"),
            RunCompletionAcknowledgeRequest {
                notice_id: notice.notice_id.clone(),
                grant_id: "rcg-real".to_string(),
                state_revision: 1,
                outcome: RunCompletionAcknowledgeOutcome::ReplyRendered,
            },
        )
        .await
        .expect("the real grant mints read evidence");
        assert_eq!(settled.settled_notice_ids, vec![notice.notice_id.clone()]);
        let read = services
            .notices
            .get(&owner_of(&caller("user-a")), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(read.is_read());
    }

    #[tokio::test]
    async fn presented_and_effect_failed_acknowledgements_transition_the_grant() {
        let services = services();
        let presented_notice = seed(&services, "user-a", "presented", "thread-a").await;
        granted(&services, &presented_notice, "rcg-presented").await;
        acknowledge(
            &services,
            caller("user-a"),
            RunCompletionAcknowledgeRequest {
                notice_id: presented_notice.notice_id.clone(),
                grant_id: "rcg-presented".to_string(),
                state_revision: 1,
                outcome: RunCompletionAcknowledgeOutcome::Presented,
            },
        )
        .await
        .expect("presented");
        let presented = services
            .notices
            .get(&owner_of(&caller("user-a")), &presented_notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(matches!(
            presented.delivery,
            CompletionDeliveryState::Presented {
                surface: CompletionSurface::InApp,
                ..
            }
        ));
        assert!(
            !presented.is_read(),
            "presentation is not read (principle 3)"
        );

        let failed_notice = seed(&services, "user-a", "failed", "thread-b").await;
        granted(&services, &failed_notice, "rcg-failed").await;
        let before = services.stale_grant_count();
        acknowledge(
            &services,
            caller("user-a"),
            RunCompletionAcknowledgeRequest {
                notice_id: failed_notice.notice_id.clone(),
                grant_id: "rcg-failed".to_string(),
                state_revision: 1,
                outcome: RunCompletionAcknowledgeOutcome::EffectFailed,
            },
        )
        .await
        .expect("effect failed");
        let regressed = services
            .notices
            .get(&owner_of(&caller("user-a")), &failed_notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        match regressed.delivery {
            CompletionDeliveryState::PendingArbitration {
                closes_at,
                grants_issued,
            } => {
                assert_eq!(grants_issued, 1, "the spent grant stays counted");
                assert!(closes_at > Utc::now(), "a fresh arbitration window reopens");
            }
            other => panic!("expected a regressed pending record, got {other:?}"),
        }
        assert_eq!(services.stale_grant_count(), before + 1);
    }
}
