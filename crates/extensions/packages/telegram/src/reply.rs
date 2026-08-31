//! The Telegram reply half: a [`ReplySink`] at `message` cadence
//! (`[channel.reply] transport = "message"`).
//!
//! The host asks a `message` sink to reconcile at exactly one point — the
//! terminal materialization ([`ReplyReconcilePoint::Terminal`]) — so this sink
//! renders the terminal document as Bot API messages through the same send
//! path `deliver` uses ([`TelegramChannelAdapter::send`]): the answer text
//! split at Telegram's UTF-16 limit, then each materialized attachment as a
//! `sendDocument`. A failure summary or a cancellation is one short message.
//! Every other reconcile point is a no-op by construction — the trait is one,
//! the cadence is the manifest's.
//!
//! Idempotency rides the checkpoint, never the revision number. A terminal
//! render the provider accepted in full is recorded as
//! `{ "terminal_applied": true, "message_refs": [..] }` under
//! [`TELEGRAM_REPLY_CHECKPOINT_VERSION`], and a repeated terminal reconcile
//! carrying it answers `Applied` without a provider call. A partial render —
//! Telegram accepted some messages, then refused one — records
//! `terminal_applied: false` with the accepted refs, and every later
//! reconcile stays `Permanent`: the Bot API has no idempotency key and no
//! read-back, so resuming would duplicate what the user already saw (the
//! coordinator's OUT-7 rule, held here across host retries and lease
//! takeovers rather than only within one call).

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelError, OutboundEnvelope, OutboundPart, OutboundTarget, OutboundVisibility,
    PartDeliveryOutcome,
};
use ironclaw_extension_contracts::reply::{
    ReplyAnswer, ReplyOutcome, ReplyOutcomeReason, ReplyProviderRef, ReplyReconcilePoint,
    ReplyReconcileRequest, ReplySink, ReplySinkCheckpoint, ReplySinkEvidence, ReplySinkOutcome,
    ReplySinkReport,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::attachment::WorkspaceFile;

use crate::channel::{TelegramChannelAdapter, TelegramSendReport};

/// The checkpoint schema this sink writes and understands. Bump it when the
/// payload shape changes; a checkpoint of any other version is ignored (see
/// [`TelegramReplyCheckpoint::decode`] for why that means "render again").
pub(crate) const TELEGRAM_REPLY_CHECKPOINT_VERSION: u32 = 1;

/// The one-line terminal message for a cancelled reply. Channel-neutral and
/// diagnostic-free: why the run stopped stays in the web app.
pub(crate) const TELEGRAM_REPLY_CANCELLED_TEXT: &str = "This reply was stopped before it finished.";

/// What the sink persisted after its last terminal render of one reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TelegramReplyCheckpoint {
    /// True once the whole terminal materialization was accepted by the Bot
    /// API. False records a partial render: `message_refs` exist
    /// provider-side, the rest of the answer never shipped.
    terminal_applied: bool,
    /// Bot API message ids the provider returned for the accepted parts, in
    /// send order.
    #[serde(default)]
    message_refs: Vec<String>,
}

impl TelegramReplyCheckpoint {
    /// Read the previous checkpoint, or `None` when there is nothing this
    /// sink may act on.
    ///
    /// A checkpoint of another version — or one whose payload does not
    /// decode — is deliberately "not applied" rather than an error. The host
    /// only re-reconciles a terminal revision it has not settled, and an
    /// unreadable checkpoint carries no evidence that could override that
    /// (the Bot API cannot be read back to find out). Between a possible
    /// duplicate answer (visible, recoverable) and a possibly never-delivered
    /// one (silent), the seam prefers the former; the window is one in-flight
    /// reply across a package upgrade, not steady state.
    fn decode(checkpoint: Option<&ReplySinkCheckpoint>) -> Option<Self> {
        let checkpoint = checkpoint?;
        if checkpoint.version() != TELEGRAM_REPLY_CHECKPOINT_VERSION {
            tracing::debug!(
                version = checkpoint.version(),
                "ignoring a telegram reply checkpoint of an unknown version"
            );
            return None;
        }
        match serde_json::from_str::<Self>(checkpoint.payload()) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                tracing::debug!(%error, "ignoring an undecodable telegram reply checkpoint");
                None
            }
        }
    }

    fn encode(&self) -> Result<ReplySinkCheckpoint, ChannelError> {
        let payload = serde_json::to_string(self).map_err(|error| ChannelError::Render {
            reason: format!("telegram reply checkpoint failed to serialize: {error}"),
        })?;
        ReplySinkCheckpoint::new(TELEGRAM_REPLY_CHECKPOINT_VERSION, payload).map_err(|error| {
            ChannelError::Render {
                reason: format!("telegram reply checkpoint exceeds the host bound: {error}"),
            }
        })
    }

    /// The accepted message ids as sink evidence. Never read back: the Bot
    /// API has no call that fetches a sent message.
    fn evidence(&self) -> ReplySinkEvidence {
        let mut evidence = ReplySinkEvidence::default();
        for reference in &self.message_refs {
            let reference = match ReplyProviderRef::new(reference.clone()) {
                Ok(reference) => reference,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "skipping a telegram message id that is not a valid provider ref"
                    );
                    continue;
                }
            };
            if let Err(error) = evidence.provider_refs.push(reference) {
                // Past the host's per-report bound; the checkpoint still
                // holds every id, so nothing is lost by stopping here.
                tracing::debug!(%error, "telegram reply evidence reached the provider-ref bound");
                break;
            }
        }
        evidence
    }
}

#[async_trait]
impl ReplySink for TelegramChannelAdapter {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError> {
        if !matches!(request.point, ReplyReconcilePoint::Terminal) {
            // A `message` channel is only ever asked at the terminal point.
            // Any other point is answered without touching the provider and
            // without disturbing the checkpoint, so the trait stays total.
            return Ok(ReplySinkReport::applied(
                request.checkpoint,
                ReplySinkEvidence::default(),
            ));
        }

        if let Some(prior) = TelegramReplyCheckpoint::decode(request.checkpoint.as_ref()) {
            if prior.terminal_applied {
                return Ok(ReplySinkReport::applied(
                    Some(prior.encode()?),
                    prior.evidence(),
                ));
            }
            if !prior.message_refs.is_empty() {
                return Ok(ReplySinkReport {
                    outcome: ReplySinkOutcome::Permanent {
                        reason: ReplyOutcomeReason::new(format!(
                            "an earlier terminal render was partially accepted ({} message(s) \
                             already accepted); resending would duplicate them",
                            prior.message_refs.len()
                        )),
                    },
                    checkpoint: Some(prior.encode()?),
                    evidence: prior.evidence(),
                });
            }
        }

        let document = &request.revision.document;
        let Some(outcome) = document.outcome.as_ref() else {
            return Err(ChannelError::Render {
                reason: "terminal reply reconcile carries no terminal outcome".to_string(),
            });
        };
        let Some(conversation) = request.target.conversation.clone() else {
            return Ok(ReplySinkReport {
                outcome: ReplySinkOutcome::Permanent {
                    reason: ReplyOutcomeReason::new("reply target names no vendor conversation"),
                },
                checkpoint: None,
                evidence: ReplySinkEvidence::default(),
            });
        };

        let parts = terminal_parts(outcome, &document.answer, request.materialized_attachments);
        if parts.is_empty() {
            // A completed run with nothing to say and nothing to attach: the
            // desired state is already reflected. Telegram rejects empty
            // text, and the final-reply path never posted these either.
            let checkpoint = TelegramReplyCheckpoint {
                terminal_applied: true,
                message_refs: Vec::new(),
            };
            return Ok(ReplySinkReport::applied(
                Some(checkpoint.encode()?),
                ReplySinkEvidence::default(),
            ));
        }

        let envelope = OutboundEnvelope {
            target: OutboundTarget {
                conversation,
                thread_anchor: request
                    .target
                    .thread_anchor
                    .map(|anchor| anchor.into_inner()),
            },
            parts,
            reply_context: request
                .reply_context
                .map(|context| context.as_bytes().to_vec()),
            registrations: Vec::new(),
            visibility: OutboundVisibility::Public,
        };
        let report = self.send(envelope, egress).await?;
        reply_report(report)
    }
}

/// The message parts a terminal document materializes to. A completed run
/// ships its answer text (when it has any — Telegram rejects empty text)
/// followed by the host-materialized final attachments; a failed or cancelled
/// run ships one line and no attachments.
fn terminal_parts(
    outcome: &ReplyOutcome,
    answer: &ReplyAnswer,
    attachments: Vec<WorkspaceFile>,
) -> Vec<OutboundPart> {
    let mut parts = Vec::new();
    match outcome {
        ReplyOutcome::Completed => {
            let text = answer.text.as_str();
            if !text.trim().is_empty() {
                parts.push(OutboundPart::Text(text.to_string()));
            }
            parts.extend(attachments.into_iter().map(OutboundPart::File));
        }
        ReplyOutcome::Failed { summary } => {
            parts.push(OutboundPart::Text(summary.as_str().to_string()));
        }
        ReplyOutcome::Cancelled => {
            parts.push(OutboundPart::Text(
                TELEGRAM_REPLY_CANCELLED_TEXT.to_string(),
            ));
        }
    }
    parts
}

/// Fold the per-part send outcomes into one sink report.
///
/// Every part accepted ⇒ `Applied`. The first part Telegram did not accept
/// decides the outcome one-to-one (`Retryable` carries the provider's
/// `Retry-After` hint) — except a retryable failure after at least one
/// accepted part, which is `Permanent`: retrying the render would re-post the
/// accepted messages (OUT-7). Accepted message ids are reported as evidence
/// and checkpointed whenever there are any, so the provider-side state is
/// never forgotten even when the outcome is not `Applied`.
fn reply_report(report: TelegramSendReport) -> Result<ReplySinkReport, ChannelError> {
    let TelegramSendReport { parts, retry_after } = report;
    let first_failure = parts
        .iter()
        .position(|part| !matches!(part, PartDeliveryOutcome::Sent { .. }));
    let accepted_parts = first_failure.unwrap_or(parts.len());
    let message_refs: Vec<String> = parts
        .iter()
        .take(accepted_parts)
        .filter_map(|part| match part {
            PartDeliveryOutcome::Sent { vendor_message_ref } => vendor_message_ref.clone(),
            _ => None,
        })
        .collect();
    let outcome = match first_failure.and_then(|index| parts.get(index)) {
        None | Some(PartDeliveryOutcome::Sent { .. }) => ReplySinkOutcome::Applied,
        Some(PartDeliveryOutcome::Retryable { reason }) if accepted_parts > 0 => {
            ReplySinkOutcome::Permanent {
                reason: ReplyOutcomeReason::new(format!(
                    "{accepted_parts} message(s) already accepted before a retryable failure \
                     ({reason}); resending would duplicate them"
                )),
            }
        }
        Some(PartDeliveryOutcome::Retryable { reason }) => ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new(reason),
            retry_after,
        },
        Some(PartDeliveryOutcome::Ambiguous { reason }) => ReplySinkOutcome::Ambiguous {
            reason: ReplyOutcomeReason::new(reason),
        },
        Some(PartDeliveryOutcome::Permanent { reason }) => ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(reason),
        },
        Some(PartDeliveryOutcome::Unauthorized { reason }) => ReplySinkOutcome::Unauthorized {
            reason: ReplyOutcomeReason::new(reason),
        },
    };
    let checkpoint = TelegramReplyCheckpoint {
        terminal_applied: outcome.is_applied(),
        message_refs,
    };
    let evidence = checkpoint.evidence();
    // Nothing accepted means nothing to remember: the host keeps whatever
    // checkpoint it already had (none, for a first render).
    let checkpoint = (accepted_parts > 0)
        .then(|| checkpoint.encode())
        .transpose()?;
    Ok(ReplySinkReport {
        outcome,
        checkpoint,
        evidence,
    })
}

#[cfg(test)]
#[path = "tests/reply.rs"]
mod tests;
