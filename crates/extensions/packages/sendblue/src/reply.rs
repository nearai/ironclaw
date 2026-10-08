//! Terminal replies use the same bounded send path as discrete delivery.
//! Checkpoints prevent replay after partial or ambiguous non-idempotent sends.
use crate::SendblueChannelAdapter;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelError, OutboundEnvelope, OutboundPart, OutboundTarget,
    OutboundVisibility, PartDeliveryOutcome,
};
use ironclaw_extension_contracts::reply::{
    ReplyOutcome, ReplyOutcomeReason, ReplyProviderRef, ReplyReconcilePoint, ReplyReconcileRequest,
    ReplySink, ReplySinkCheckpoint, ReplySinkEvidence, ReplySinkOutcome, ReplySinkReport,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;

#[derive(serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    applied: bool,
    evidence: ReplySinkEvidence,
}

#[async_trait]
impl ReplySink for SendblueChannelAdapter {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError> {
        if !matches!(request.point, ReplyReconcilePoint::Terminal) {
            return Ok(ReplySinkReport::applied(
                request.checkpoint,
                ReplySinkEvidence::default(),
            ));
        }
        if let Some(checkpoint) = request.checkpoint {
            let (prior, invalid_reason) = match checkpoint.version() {
                1 => match serde_json::from_str::<Checkpoint>(checkpoint.payload()) {
                    Ok(prior) => (Some(prior), None),
                    Err(error) => {
                        // serde's Display may echo a value from the payload. Retain
                        // structured failure diagnostics without private message text.
                        tracing::debug!(category = ?error.classify(), line = error.line(), column = error.column(),
                            "Malformed Sendblue reply checkpoint");
                        (
                            None,
                            Some(
                                "Sendblue reply checkpoint is malformed; inspect provider history before retrying",
                            ),
                        )
                    }
                },
                version => {
                    tracing::debug!(version, "Unsupported Sendblue reply checkpoint version");
                    (
                        None,
                        Some(
                            "Sendblue reply checkpoint version is unsupported; inspect provider history before retrying",
                        ),
                    )
                }
            };
            let outcome = if prior.as_ref().is_some_and(|prior| prior.applied) {
                ReplySinkOutcome::Applied
            } else {
                ReplySinkOutcome::Permanent {
                    reason: ReplyOutcomeReason::new(
                        invalid_reason.unwrap_or("An earlier Sendblue attempt may have sent messages; inspect provider history before retrying"),
                    ),
                }
            };
            return Ok(ReplySinkReport {
                outcome,
                checkpoint: Some(checkpoint),
                evidence: prior.map(|prior| prior.evidence).unwrap_or_default(),
            });
        }

        let document = &request.revision.document;
        let text = match &document.outcome {
            Some(ReplyOutcome::Completed) => document.answer.text.as_str().to_string(),
            Some(ReplyOutcome::Failed { summary }) => summary.as_str().to_string(),
            Some(ReplyOutcome::Cancelled) => "This reply was stopped before it finished.".into(),
            None => {
                return Err(ChannelError::Render {
                    reason: "Terminal reply has no outcome".into(),
                });
            }
        };
        let conversation = request
            .target
            .conversation
            .ok_or_else(|| ChannelError::Render {
                reason: "Sendblue reply has no phone conversation".into(),
            })?;
        let mut parts = Vec::new();
        if !text.trim().is_empty() {
            parts.push(OutboundPart::Text(text));
        }
        if !request.materialized_attachments.is_empty() {
            parts.push(OutboundPart::Text("[This reply includes a file. Open IronClaw to view it; this channel supports text only.]".into()));
        }
        let report = self
            .deliver(
                OutboundEnvelope {
                    target: OutboundTarget {
                        conversation,
                        thread_anchor: None,
                    },
                    parts,
                    reply_context: None,
                    registrations: Vec::new(),
                    visibility: OutboundVisibility::Public,
                },
                egress,
            )
            .await?;
        let mut outcome = ReplySinkOutcome::Applied;
        let mut evidence = ReplySinkEvidence::default();
        for part in report.parts {
            match part {
                PartDeliveryOutcome::Sent {
                    vendor_message_ref: Some(reference),
                } => {
                    if let Ok(reference) = ReplyProviderRef::new(reference) {
                        // Evidence is bounded by the host; the attempt checkpoint
                        // protects the entire render even if the reference list fills.
                        // Sixteen 256-byte handles still fit the 16 KiB
                        // checkpoint even when every character needs JSON escaping.
                        if evidence.provider_refs.len() < 16 {
                            evidence.provider_refs.push(reference).map_err(|error| {
                                ChannelError::Render {
                                    reason: error.to_string(),
                                }
                            })?;
                        }
                    }
                }
                PartDeliveryOutcome::Sent { .. } => {}
                PartDeliveryOutcome::Ambiguous { reason } => {
                    outcome = ReplySinkOutcome::Ambiguous {
                        reason: ReplyOutcomeReason::new(reason),
                    };
                    break;
                }
                PartDeliveryOutcome::Unauthorized { reason } => {
                    outcome = ReplySinkOutcome::Unauthorized {
                        reason: ReplyOutcomeReason::new(reason),
                    };
                    break;
                }
                PartDeliveryOutcome::Permanent { reason }
                | PartDeliveryOutcome::Retryable { reason } => {
                    outcome = ReplySinkOutcome::Permanent {
                        reason: ReplyOutcomeReason::new(reason),
                    };
                    break;
                }
            }
        }
        let payload = serde_json::to_string(&Checkpoint {
            applied: outcome.is_applied(),
            evidence: evidence.clone(),
        })
        .map_err(|error| ChannelError::Render {
            reason: error.to_string(),
        })?;
        let checkpoint =
            ReplySinkCheckpoint::new(1, payload).map_err(|error| ChannelError::Render {
                reason: error.to_string(),
            })?;
        Ok(ReplySinkReport {
            outcome,
            checkpoint: Some(checkpoint),
            evidence,
        })
    }
}
