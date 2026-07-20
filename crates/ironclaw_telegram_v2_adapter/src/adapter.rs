//! Telegram v2 ProductAdapter implementation.

use async_trait::async_trait;
use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    DeliveryStatus, EgressCredentialHandle, OutboundDeliverySink, ParsedProductInbound,
    ProductAdapter, ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId,
    ProductCapabilityFlag, ProductOutboundEnvelope, ProductOutboundPayload, ProductRenderOutcome,
    ProductSurfaceKind, ProtocolAuthEvidence, ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};

use crate::payload::{GroupTriggerPolicy, TELEGRAM_API_HOST, parse_telegram_update};
use crate::render::{
    render_auth_prompt, render_final_reply, render_gate_prompt, render_progress_typing,
};

/// Configuration for a Telegram v2 adapter installation.
#[derive(Debug, Clone)]
pub struct TelegramV2AdapterConfig {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub group_trigger_policy: GroupTriggerPolicy,
    /// Credential handle (resolved by the host to the bot token at request
    /// time) used for egress to api.telegram.org.
    pub egress_credential_handle: EgressCredentialHandle,
    /// Auth requirement the host enforces before invoking `parse_inbound`.
    /// Telegram webhooks use a shared-secret header; the host verifies the
    /// header and mints a `ProtocolAuthEvidence::Verified` claim before
    /// any adapter-side parsing happens.
    pub auth_requirement: AuthRequirement,
    /// If true, the adapter advertises `ExternalProgressPush` and renders
    /// typing indicators on outbound `Progress` envelopes. Default: false
    /// (#3266 progress-opt-in policy).
    pub progress_push_enabled: bool,
}

pub struct TelegramV2Adapter {
    config: TelegramV2AdapterConfig,
    capabilities: ProductAdapterCapabilities,
    /// Per-installation egress allowlist. One paired
    /// `(api.telegram.org, Some(bot_token_handle))` entry — the host
    /// enforces this declaration when policing outbound requests, so
    /// without overriding the trait default the adapter would
    /// implicitly declare an empty allowlist and every Telegram send
    /// would be denied (Copilot review on PR #3355).
    declared_egress: Vec<DeclaredEgressTarget>,
}

impl TelegramV2Adapter {
    pub fn new(config: TelegramV2AdapterConfig) -> Self {
        let mut capabilities = telegram_default_capabilities();
        if config.progress_push_enabled {
            capabilities = capabilities.with(ProductCapabilityFlag::ExternalProgressPush);
        }
        let declared_egress = vec![DeclaredEgressTarget::new(
            DeclaredEgressHost::new(TELEGRAM_API_HOST).expect("static host valid"), // safety: compile-time const "api.telegram.org" satisfies DeclaredEgressHost validator
            Some(config.egress_credential_handle.clone()),
        )];
        Self {
            config,
            capabilities,
            declared_egress,
        }
    }

    pub fn config(&self) -> &TelegramV2AdapterConfig {
        &self.config
    }
}

/// Capabilities a Telegram v2 adapter advertises by default, before any
/// per-config opt-ins (e.g. `ExternalProgressPush` via
/// [`TelegramV2AdapterConfig::progress_push_enabled`], #3266) are applied.
pub fn telegram_default_capabilities() -> ProductAdapterCapabilities {
    ProductAdapterCapabilities::external_channel_default()
}

/// Egress hosts that any Telegram v2 installation may target.
///
/// Helper retained for tests and host-glue code that needs the
/// installation-agnostic host list (no credential pairing). Production
/// hosts should drive policy from
/// [`ProductAdapter::declared_egress`] on a concrete adapter instance,
/// which carries the paired `(host, credential_handle)` shape.
pub fn telegram_declared_egress_hosts() -> Vec<DeclaredEgressHost> {
    vec![DeclaredEgressHost::new(TELEGRAM_API_HOST).expect("static host valid")] // safety: compile-time const "api.telegram.org" satisfies DeclaredEgressHost validator
}

#[async_trait]
impl ProductAdapter for TelegramV2Adapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.config.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.config.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::ExternalChannel
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        &self.config.auth_requirement
    }

    fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &self.declared_egress
    }

    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedProductInbound, ProductAdapterError> {
        parse_telegram_update(
            raw_payload,
            auth_evidence,
            &self.config.installation_id,
            &self.config.group_trigger_policy,
        )
        .map_err(|err| match err {
            crate::payload::PayloadParseError::UnauthenticatedPayload => {
                ProductAdapterError::Authentication(
                    ironclaw_product_adapters::ProtocolAuthFailure::Missing,
                )
            }
            crate::payload::PayloadParseError::InvalidJson { reason } => {
                ProductAdapterError::MalformedInboundPayload {
                    reason: ironclaw_product_adapters::redaction::RedactedString::new(reason),
                }
            }
            crate::payload::PayloadParseError::MissingUpdateId => {
                ProductAdapterError::MalformedInboundPayload {
                    reason: ironclaw_product_adapters::redaction::RedactedString::new(
                        "telegram update missing update_id",
                    ),
                }
            }
            crate::payload::PayloadParseError::InvalidExternalRef { kind, reason } => {
                ProductAdapterError::InvalidIdentifier { kind, reason }
            }
        })
    }

    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        // Henry's review on PR #3355: fail closed when the envelope's
        // installation does not match this adapter. Projection routing
        // mistakes must not let one Telegram installation render with
        // another installation's bot token / chat binding. No delivery-
        // sink record is emitted on mismatch — the attempt never
        // belonged to this adapter, so this adapter is not the
        // authoritative reporter for it.
        if envelope.adapter_id != self.config.adapter_id {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "envelope.adapter_id",
                reason: format!(
                    "envelope adapter_id `{}` does not match this adapter `{}`",
                    envelope.adapter_id.as_str(),
                    self.config.adapter_id.as_str(),
                ),
            });
        }
        if envelope.installation_id != self.config.installation_id {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "envelope.installation_id",
                reason: format!(
                    "envelope installation_id `{}` does not match this installation `{}`",
                    envelope.installation_id.as_str(),
                    self.config.installation_id.as_str(),
                ),
            });
        }

        let attempt_id = envelope.delivery_attempt_id;
        let target_binding = envelope.target.reply_target_binding_ref.clone();

        // Capture the run_id where the payload carries one — drives
        // `DeliveryStatus::*.run_id` so the projection layer can
        // correlate the delivery report back to the originating turn.
        let run_id: Option<TurnRunId> = match &envelope.payload {
            ProductOutboundPayload::FinalReply(view) => Some(view.turn_run_id),
            ProductOutboundPayload::Progress(view) => Some(view.turn_run_id),
            ProductOutboundPayload::GatePrompt(view) => Some(view.turn_run_id),
            ProductOutboundPayload::AuthPrompt(view) => Some(view.turn_run_id),
            _ => None,
        };

        // Resolve the concrete chat target once, preferring the host-resolved
        // conversation ref (populated on the live reply path) over the opaque
        // `reply:` binding token. A resolution failure folds into the same
        // permanent-failure handling as a render failure per payload below.
        let resolved_target = crate::render::resolve_reply_target(&envelope.target);

        let requests = match envelope.payload {
            ProductOutboundPayload::FinalReply(view) => {
                match resolved_target.clone().and_then(|reply| {
                    render_final_reply(&reply, &view, self.config.egress_credential_handle.clone())
                }) {
                    Ok(reqs) => reqs,
                    Err(render_err) => {
                        // Malformed reply target is a permanent data-shape
                        // failure; retrying won't help.
                        record_status(
                            delivery_sink,
                            DeliveryStatus::FailedPermanent {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(render_err.to_string()),
                            },
                        )
                        .await;
                        return Err(map_render_error(render_err));
                    }
                }
            }
            ProductOutboundPayload::Progress(view) => {
                if !self
                    .capabilities
                    .contains(ProductCapabilityFlag::ExternalProgressPush)
                {
                    // Progress not advertised; defer and record so the
                    // host can dedupe by attempt id.
                    record_status(
                        delivery_sink,
                        DeliveryStatus::Deferred {
                            attempt_id,
                            target: target_binding.clone(),
                            run_id,
                            reason: RedactedString::new(
                                "progress capability not advertised on this installation",
                            ),
                        },
                    )
                    .await;
                    return Ok(ProductRenderOutcome::Deferred);
                }
                match resolved_target.clone().and_then(|reply| {
                    render_progress_typing(
                        &reply,
                        &view,
                        self.config.egress_credential_handle.clone(),
                    )
                }) {
                    Ok(Some(req)) => vec![req],
                    Ok(None) => {
                        record_status(
                            delivery_sink,
                            DeliveryStatus::Deferred {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(
                                    "progress kind did not map to a typing action",
                                ),
                            },
                        )
                        .await;
                        return Ok(ProductRenderOutcome::Deferred);
                    }
                    Err(render_err) => {
                        record_status(
                            delivery_sink,
                            DeliveryStatus::FailedPermanent {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(render_err.to_string()),
                            },
                        )
                        .await;
                        return Err(map_render_error(render_err));
                    }
                }
            }
            ProductOutboundPayload::AuthPrompt(view) => {
                // The shared channel delivery driver produces these for every
                // Telegram run that parks `BlockedAuth` with a link-shaped
                // challenge. Deferring here left the DM silent after the
                // working message was deleted (Ben's 2026-07-17 regression).
                match resolved_target.clone().and_then(|reply| {
                    render_auth_prompt(&reply, &view, self.config.egress_credential_handle.clone())
                }) {
                    Ok(reqs) => reqs,
                    Err(render_err) => {
                        record_status(
                            delivery_sink,
                            DeliveryStatus::FailedPermanent {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(render_err.to_string()),
                            },
                        )
                        .await;
                        return Err(map_render_error(render_err));
                    }
                }
            }
            ProductOutboundPayload::GatePrompt(view) => {
                match resolved_target.clone().and_then(|reply| {
                    render_gate_prompt(&reply, &view, self.config.egress_credential_handle.clone())
                }) {
                    Ok(reqs) => reqs,
                    Err(render_err) => {
                        record_status(
                            delivery_sink,
                            DeliveryStatus::FailedPermanent {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(render_err.to_string()),
                            },
                        )
                        .await;
                        return Err(map_render_error(render_err));
                    }
                }
            }
            ProductOutboundPayload::CapabilityActivity(_)
            | ProductOutboundPayload::CapabilityDisplayPreview(_)
            | ProductOutboundPayload::ProjectionSnapshot { .. }
            | ProductOutboundPayload::ProjectionUpdate { .. }
            | ProductOutboundPayload::KeepAlive => {
                // Telegram never consumes projection subscriptions; the
                // workflow should not route these to a Telegram installation.
                record_status(
                    delivery_sink,
                    DeliveryStatus::Deferred {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id: None,
                        reason: RedactedString::new(
                            "telegram surface does not consume projection envelopes",
                        ),
                    },
                )
                .await;
                return Ok(ProductRenderOutcome::Deferred);
            }
        };

        // Chunked final replies send SEQUENTIALLY and stop at the first
        // failure (qa-telegram:C3): ordering is user-visible, and one honest
        // failure status per attempt beats a hole in the middle of a reply.
        // Chunks already delivered stand — Telegram has no transaction — so
        // the failure status is the truthful record of a partial delivery.
        // Once ANY chunk has been delivered, every subsequent failure is
        // TERMINAL for the attempt (FailedPermanent + a non-transient
        // error): re-delivering the envelope restarts from chunk zero and
        // would duplicate user-visible text. Only a first-chunk failure —
        // which delivered nothing — keeps its normal retryable mapping.
        let mut any_chunk_delivered = false;
        for request in requests {
            let response = match egress.send(request).await {
                Ok(resp) => resp,
                Err(egress_err) => {
                    let status = egress_err_to_delivery_status(
                        &egress_err,
                        attempt_id,
                        target_binding.clone(),
                        run_id,
                    );
                    let status = if any_chunk_delivered {
                        demote_to_permanent_after_partial_delivery(status)
                    } else {
                        status
                    };
                    record_status(delivery_sink, status).await;
                    if any_chunk_delivered {
                        return Err(ProductAdapterError::EgressDenied {
                            reason: RedactedString::new(egress_err.to_string()),
                        });
                    }
                    return Err(map_egress_error(egress_err));
                }
            };

            if !(200..300).contains(&response.status()) {
                // On a multi-chunk reply the failure stops the sequence here;
                // the recorded status is the honest report of the partial
                // delivery (chunk position is deliberately not interpolated —
                // the reason string is a stable category).
                let reason = RedactedString::new(format!(
                    "telegram bot api returned status {}",
                    response.status()
                ));
                if any_chunk_delivered {
                    // Partial delivery: terminal, never re-deliverable.
                    record_status(
                        delivery_sink,
                        DeliveryStatus::FailedPermanent {
                            attempt_id,
                            target: target_binding.clone(),
                            run_id,
                            reason: reason.clone(),
                        },
                    )
                    .await;
                    return Err(ProductAdapterError::EgressDenied { reason });
                }
                // Group transient HTTP outcomes (5xx, 429) into the retryable
                // bucket so the host glue can re-deliver. 4xx (except 429) is
                // a deterministic policy-denied result and should NOT be
                // retried. 401/403 surface as FailedUnauthorized so the host
                // can pause re-delivery until credentials change.
                if response.status() >= 500 || response.status() == 429 {
                    record_status(
                        delivery_sink,
                        DeliveryStatus::FailedRetryable {
                            attempt_id,
                            target: target_binding.clone(),
                            run_id,
                            reason: reason.clone(),
                        },
                    )
                    .await;
                    return Err(ProductAdapterError::WorkflowTransient { reason });
                }
                if response.status() == 401 || response.status() == 403 {
                    record_status(
                        delivery_sink,
                        DeliveryStatus::FailedUnauthorized {
                            attempt_id,
                            target: target_binding.clone(),
                            run_id,
                            reason: reason.clone(),
                        },
                    )
                    .await;
                } else {
                    record_status(
                        delivery_sink,
                        DeliveryStatus::FailedPermanent {
                            attempt_id,
                            target: target_binding.clone(),
                            run_id,
                            reason: reason.clone(),
                        },
                    )
                    .await;
                }
                return Err(ProductAdapterError::EgressDenied { reason });
            }
            any_chunk_delivered = true;
        }

        record_status(
            delivery_sink,
            DeliveryStatus::Delivered {
                attempt_id,
                target: target_binding,
                run_id,
            },
        )
        .await;
        Ok(ProductRenderOutcome::DeliveryRecorded)
    }
}

/// After a chunk has already been delivered, retryable statuses demote to
/// permanent: automatic re-delivery restarts from chunk zero and would
/// duplicate the already-delivered text. Non-retryable statuses pass through.
fn demote_to_permanent_after_partial_delivery(status: DeliveryStatus) -> DeliveryStatus {
    match status {
        DeliveryStatus::FailedRetryable {
            attempt_id,
            target,
            run_id,
            reason,
        }
        | DeliveryStatus::FailedUnauthorized {
            attempt_id,
            target,
            run_id,
            reason,
        } => DeliveryStatus::FailedPermanent {
            attempt_id,
            target,
            run_id,
            reason,
        },
        other => other,
    }
}

/// Forward a delivery status to the sink. Pulled out so each
/// render/egress branch records exactly once and the trait's `record`
/// future is fully awaited before returning to the host.
async fn record_status(sink: &dyn OutboundDeliverySink, status: DeliveryStatus) {
    sink.record(status).await;
}

/// Classify a `ProtocolHttpEgressError` for delivery-sink reporting.
/// Mirrors `map_egress_error` but produces a `DeliveryStatus` rather
/// than a `ProductAdapterError` — the host needs both, because the
/// error drives the protocol response (status code) and the delivery
/// status drives projection re-delivery / pause-on-auth.
fn egress_err_to_delivery_status(
    err: &ProtocolHttpEgressError,
    attempt_id: ironclaw_product_adapters::DeliveryAttemptId,
    target: ReplyTargetBindingRef,
    run_id: Option<TurnRunId>,
) -> DeliveryStatus {
    let reason = RedactedString::new(err.to_string());
    match err {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => DeliveryStatus::FailedRetryable {
            attempt_id,
            target,
            run_id,
            reason,
        },
        ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. } => {
            DeliveryStatus::FailedUnauthorized {
                attempt_id,
                target,
                run_id,
                reason,
            }
        }
        ProtocolHttpEgressError::UndeclaredHost { .. }
        | ProtocolHttpEgressError::PolicyDenied { .. } => DeliveryStatus::FailedPermanent {
            attempt_id,
            target,
            run_id,
            reason,
        },
    }
}

/// Map a `TelegramRenderError` to a `ProductAdapterError`. Malformed reply
/// targets surface as `InvalidIdentifier` (matching how `parse_inbound`
/// surfaces malformed inbound external refs) so callers can distinguish
/// data-shape problems from genuine internal failures.
fn map_render_error(err: crate::render::TelegramRenderError) -> ProductAdapterError {
    match err {
        crate::render::TelegramRenderError::InvalidReplyTarget { .. } => {
            ProductAdapterError::InvalidIdentifier {
                kind: "reply_target",
                reason: err.to_string(),
            }
        }
    }
}

/// Map a `ProtocolHttpEgressError` to either a retryable
/// `WorkflowTransient` or a non-retryable `EgressDenied`. Network /
/// timeout / leak-detector failures are treated as transient.
fn map_egress_error(err: ProtocolHttpEgressError) -> ProductAdapterError {
    let reason = ironclaw_product_adapters::redaction::RedactedString::new(err.to_string());
    match err {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => {
            ProductAdapterError::WorkflowTransient { reason }
        }
        ProtocolHttpEgressError::UndeclaredHost { .. }
        | ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. }
        | ProtocolHttpEgressError::PolicyDenied { .. } => {
            ProductAdapterError::EgressDenied { reason }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::FakeProtocolHttpEgress;

    fn config(progress: bool) -> TelegramV2AdapterConfig {
        TelegramV2AdapterConfig {
            adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
            installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
            group_trigger_policy: GroupTriggerPolicy {
                bot_username: "ironclaw_bot".into(),
                bot_user_id: 9000,
                recognized_commands: vec!["start".into()],
            },
            egress_credential_handle: EgressCredentialHandle::new("telegram_bot_token")
                .expect("valid"),
            auth_requirement: AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            },
            progress_push_enabled: progress,
        }
    }

    fn test_outbound_target() -> ironclaw_product_adapters::ProductOutboundTarget {
        let reply = crate::render::build_reply_target_binding(-100, Some(7), Some(42));
        let conv = ironclaw_product_adapters::ExternalConversationRef::new(
            None,
            "-100",
            None::<&str>,
            None::<&str>,
        )
        .expect("valid");
        ironclaw_product_adapters::ProductOutboundTarget::new(reply, conv, None)
    }

    fn test_outbound_target_no_topic_no_reply() -> ironclaw_product_adapters::ProductOutboundTarget
    {
        let reply = crate::render::build_reply_target_binding(-100, None, None);
        let conv = ironclaw_product_adapters::ExternalConversationRef::new(
            None,
            "-100",
            None::<&str>,
            None::<&str>,
        )
        .expect("valid");
        ironclaw_product_adapters::ProductOutboundTarget::new(reply, conv, None)
    }

    fn test_projection_cursor() -> ironclaw_product_adapters::ProjectionCursor {
        ironclaw_product_adapters::ProjectionCursor::new("test-cursor").expect("valid")
    }

    #[test]
    fn capabilities_default_excludes_progress() {
        let adapter = TelegramV2Adapter::new(config(false));
        assert!(
            !adapter
                .capabilities()
                .contains(ProductCapabilityFlag::ExternalProgressPush)
        );
        assert!(
            adapter
                .capabilities()
                .contains(ProductCapabilityFlag::ExternalFinalReplyPush)
        );
    }

    #[test]
    fn capabilities_with_progress_opt_in_includes_progress_push() {
        let adapter = TelegramV2Adapter::new(config(true));
        assert!(
            adapter
                .capabilities()
                .contains(ProductCapabilityFlag::ExternalProgressPush)
        );
    }

    #[test]
    fn declared_hosts_only_telegram_api() {
        let hosts = telegram_declared_egress_hosts();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].as_str(), "api.telegram.org");
    }

    #[test]
    fn declared_egress_pairs_telegram_host_with_bot_token_handle() {
        // Copilot review on PR #3355: the trait default returns `&[]`,
        // which would make hosts that enforce `DeclaredEgressTarget`-based
        // policy deny every Telegram send. The override must surface the
        // installation's `(api.telegram.org, Some(<bot_token_handle>))`
        // pair so policy admits the requests rendered by `render_outbound`.
        let adapter = TelegramV2Adapter::new(config(false));
        let declared = adapter.declared_egress();
        assert_eq!(declared.len(), 1, "expected exactly one declared target");
        assert_eq!(declared[0].host.as_str(), "api.telegram.org");
        let handle = declared[0]
            .credential_handle
            .as_ref()
            .expect("credential handle paired with telegram host");
        assert_eq!(handle.as_str(), "telegram_bot_token");
    }

    #[test]
    fn parse_inbound_refuses_unverified_evidence() {
        let adapter = TelegramV2Adapter::new(config(false));
        let unverified = ProtocolAuthEvidence::failed(
            ironclaw_product_adapters::ProtocolAuthFailure::SharedSecretMismatch,
        );
        let err = adapter
            .parse_inbound(b"{\"update_id\":1}", &unverified)
            .expect_err("must fail");
        assert!(matches!(err, ProductAdapterError::Authentication(_)));
    }

    /// qa-telegram:C3 — an over-4096-unit final reply egresses as ORDERED
    /// sequential sendMessage chunks and records exactly one Delivered for
    /// the attempt once every chunk lands.
    #[tokio::test]
    async fn render_outbound_sends_chunks_sequentially_and_records_one_delivered() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "x".repeat(9000),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };
        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("render ok");
        let calls = egress.calls();
        assert_eq!(
            calls.len(),
            3,
            "9000 units -> three sequential sendMessage calls"
        );
        let mut reassembled = String::new();
        for call in &calls {
            assert_eq!(call.path.as_str(), "/sendMessage");
            let body: serde_json::Value = serde_json::from_slice(&call.body).expect("body json");
            reassembled.push_str(body["text"].as_str().expect("text"));
        }
        assert_eq!(reassembled.len(), 9000, "ordered chunks are lossless");
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1, "one status per delivery attempt");
        assert!(
            matches!(statuses[0], DeliveryStatus::Delivered { .. }),
            "all chunks landed -> Delivered, got {:?}",
            statuses[0]
        );
    }

    /// qa-telegram:C3 — once ANY chunk has been delivered, a later failure
    /// is TERMINAL for the attempt (FailedPermanent): re-delivering the
    /// envelope would resend the already-delivered chunks and duplicate
    /// user-visible text, so the host must never auto-retry it.
    #[tokio::test]
    async fn render_outbound_partial_chunk_failure_is_terminal_not_retryable() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        // First chunk lands, second hits a 500; a third response staying
        // queued would prove an unwanted third send.
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                200,
                br#"{"ok":true,"result":{"message_id":1}}"#.to_vec(),
            )),
        );
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                500,
                br#"{"ok":false,"description":"scripted outage"}"#.to_vec(),
            )),
        );
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "y".repeat(9000),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };
        let error = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("mid-sequence failure surfaces");
        assert!(
            !matches!(error, ProductAdapterError::WorkflowTransient { .. }),
            "a partial delivery must not surface as transient/retryable: {error:?}"
        );
        assert_eq!(
            egress.calls().len(),
            2,
            "the failed second chunk stops the sequence"
        );
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1, "one honest status for the attempt");
        assert!(
            matches!(statuses[0], DeliveryStatus::FailedPermanent { .. }),
            "500 AFTER a delivered chunk -> FailedPermanent (re-delivery would duplicate text), got {:?}",
            statuses[0]
        );
    }

    /// qa-telegram:C3 — a FIRST-chunk failure delivered nothing, so it stays
    /// honestly retryable: re-delivery cannot duplicate anything.
    #[tokio::test]
    async fn render_outbound_first_chunk_failure_stays_retryable() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                500,
                br#"{"ok":false,"description":"scripted outage"}"#.to_vec(),
            )),
        );
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "z".repeat(9000),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };
        let error = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("first-chunk failure surfaces");
        assert!(matches!(
            error,
            ProductAdapterError::WorkflowTransient { .. }
        ));
        assert_eq!(egress.calls().len(), 1, "nothing delivered, sequence stops");
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            matches!(statuses[0], DeliveryStatus::FailedRetryable { .. }),
            "500 with zero delivered chunks stays retryable, got {:?}",
            statuses[0]
        );
    }

    #[tokio::test]
    async fn render_outbound_final_reply_uses_constrained_egress() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };
        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("render ok");
        let calls = egress.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].host.as_str(), "api.telegram.org");
        assert_eq!(calls[0].method.as_str(), "POST");
        assert_eq!(calls[0].path.as_str(), "/sendMessage");
        assert_eq!(
            calls[0].credential_handle.as_deref(),
            Some("telegram_bot_token")
        );
    }

    #[tokio::test]
    async fn render_outbound_progress_skipped_when_capability_off() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target_no_topic_no_reply(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::Progress(
                ironclaw_product_adapters::ProgressUpdateView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    kind: ironclaw_product_adapters::ProgressKind::Typing,
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };
        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("ok");
        // Progress is not advertised -> egress NOT called.
        assert!(egress.calls().is_empty());
        // …and the delivery sink records a Deferred for the attempt so
        // the host can dedupe by attempt id.
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], DeliveryStatus::Deferred { .. }));
    }

    #[tokio::test]
    async fn render_outbound_capability_activity_deferred_without_egress() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target_no_topic_no_reply(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::CapabilityActivity(
                serde_json::from_value(serde_json::json!({
                    "invocation_id": uuid::Uuid::new_v4(),
                    "thread_id": "thread-activity",
                    "capability_id": "script.echo",
                    "status": "running",
                    "provider": "script",
                    "runtime": "script",
                    "process_id": null,
                    "output_bytes": null,
                    "error_kind": null,
                    "updated_at": chrono::Utc::now(),
                }))
                .expect("activity view"),
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        let outcome = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("ok");

        assert_eq!(outcome, ProductRenderOutcome::Deferred);
        assert!(egress.calls().is_empty());
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], DeliveryStatus::Deferred { .. }));
    }

    fn final_reply_envelope(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
    ) -> ProductOutboundEnvelope {
        ProductOutboundEnvelope {
            adapter_id,
            installation_id,
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        }
    }

    #[tokio::test]
    async fn render_outbound_rejects_mismatched_adapter_id_and_does_not_egress() {
        // Henry's review on PR #3355: a misrouted envelope from a
        // different adapter must never render with this adapter's bot
        // credential. Fail closed via `InvalidIdentifier` and ensure
        // no HTTP call leaks to api.telegram.org.
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let wrong_adapter_id = ProductAdapterId::new("some_other_adapter").expect("valid");
        let envelope = final_reply_envelope(wrong_adapter_id, adapter.installation_id().clone());

        let err = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("must reject mismatched adapter_id");

        match err {
            ProductAdapterError::InvalidIdentifier { kind, .. } => {
                assert_eq!(kind, "envelope.adapter_id");
            }
            other => panic!("expected InvalidIdentifier, got: {other:?}"),
        }
        assert!(
            egress.calls().is_empty(),
            "no egress should fire for a mismatched envelope",
        );
        // No delivery-sink record either — this adapter is not the
        // authoritative reporter for an attempt that never belonged to
        // it (the routing layer that misdelivered owns the report).
        assert!(sink.statuses().is_empty());
    }

    #[tokio::test]
    async fn render_outbound_rejects_mismatched_installation_id_and_does_not_egress() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let wrong_installation_id = AdapterInstallationId::new("install_beta").expect("valid");
        let envelope = final_reply_envelope(adapter.adapter_id().clone(), wrong_installation_id);

        let err = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("must reject mismatched installation_id");

        match err {
            ProductAdapterError::InvalidIdentifier { kind, .. } => {
                assert_eq!(kind, "envelope.installation_id");
            }
            other => panic!("expected InvalidIdentifier, got: {other:?}"),
        }
        assert!(
            egress.calls().is_empty(),
            "no egress should fire for a mismatched envelope",
        );
        assert!(sink.statuses().is_empty());
    }

    #[tokio::test]
    async fn render_outbound_records_delivered_on_2xx() {
        // The adapter advertises `DeliveryStatusReporting`; a successful
        // send must produce a `DeliveryStatus::Delivered` on the sink
        // (Henry's review on PR #3355 — without this the capability is
        // a false claim).
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let attempt_id = uuid::Uuid::new_v4();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: attempt_id,
        };

        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("render ok");

        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1, "exactly one delivery status recorded");
        match &statuses[0] {
            DeliveryStatus::Delivered {
                attempt_id: recorded,
                run_id,
                ..
            } => {
                assert_eq!(*recorded, attempt_id);
                assert!(run_id.is_some(), "FinalReply propagates the turn run id");
            }
            other => panic!("expected DeliveryStatus::Delivered, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_outbound_records_retryable_on_telegram_5xx() {
        // 500 + 429 ⇒ FailedRetryable. The host glue uses this to
        // re-deliver later instead of pausing for credential rotation.
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                502,
                Vec::new(),
            )),
        );
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        let err = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("502 must surface as transient");
        assert!(matches!(err, ProductAdapterError::WorkflowTransient { .. }));

        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(
            statuses[0],
            DeliveryStatus::FailedRetryable { .. }
        ));
    }

    #[tokio::test]
    async fn render_outbound_records_unauthorized_on_telegram_401() {
        // 401 / 403 ⇒ FailedUnauthorized so the host can pause
        // re-delivery until the bot token is rotated.
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                401,
                Vec::new(),
            )),
        );
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        let err = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("401 must surface as EgressDenied");
        assert!(matches!(err, ProductAdapterError::EgressDenied { .. }));

        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(
            statuses[0],
            DeliveryStatus::FailedUnauthorized { .. }
        ));
    }

    #[tokio::test]
    async fn render_outbound_records_permanent_on_telegram_400() {
        // 4xx other than 401/403/429 ⇒ FailedPermanent: the request is
        // malformed and the host should NOT re-deliver.
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        egress.program_response(
            "api.telegram.org",
            Ok(ironclaw_product_adapters::EgressResponse::new(
                400,
                Vec::new(),
            )),
        );
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::FinalReply(
                ironclaw_product_adapters::FinalReplyView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    text: "hi".into(),
                    generated_at: chrono::Utc::now(),
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        let err = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect_err("400 must surface as EgressDenied");
        assert!(matches!(err, ProductAdapterError::EgressDenied { .. }));

        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(
            statuses[0],
            DeliveryStatus::FailedPermanent { .. }
        ));
    }

    /// Ben's regression (2026-07-17): a Telegram run parked `BlockedAuth`
    /// with an OAuth-shaped challenge produced SILENCE — the shared channel
    /// delivery driver built an `AuthPrompt` (authorization URL and all), but
    /// this adapter's stub arm recorded `Deferred` and rendered nothing, so
    /// the DM saw "thinking…" get deleted and then no message. An auth prompt
    /// must deliver a `sendMessage` carrying the authorization link.
    #[tokio::test]
    async fn render_outbound_auth_prompt_sends_link_message_and_records_delivered() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let run_id = ironclaw_turns::TurnRunId::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::AuthPrompt(
                ironclaw_product_adapters::AuthPromptView {
                    turn_run_id: run_id,
                    auth_request_ref: "auth:flow-1".into(),
                    invocation_id: None,
                    headline: "Authorization needed".into(),
                    body: "Connect your Google account to continue this run.".into(),
                    challenge_kind: None,
                    provider: Some("google".into()),
                    account_label: None,
                    authorization_url: Some(
                        "https://accounts.google.com/o/oauth2/v2/auth?client_id=test".into(),
                    ),
                    expires_at: None,
                    connection: None,
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("auth prompt renders");

        let calls = egress.calls();
        assert_eq!(calls.len(), 1, "auth prompt is one sendMessage");
        assert_eq!(calls[0].path.as_str(), "/sendMessage");
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).expect("body json");
        assert_eq!(body["chat_id"], -100);
        let text = body["text"].as_str().expect("text");
        assert!(text.contains("Authorization needed"), "headline: {text}");
        assert!(
            text.contains("https://accounts.google.com/o/oauth2/v2/auth?client_id=test"),
            "the authorization URL is the actionable part of the prompt: {text}"
        );
        assert!(
            body.get("parse_mode").is_none(),
            "prompts are plain text — no parse_mode (qa-telegram:C4)"
        );
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            matches!(
                statuses[0],
                DeliveryStatus::Delivered {
                    run_id: Some(recorded),
                    ..
                } if recorded == run_id
            ),
            "delivered with the originating run id for correlation, got {:?}",
            statuses[0]
        );
    }

    /// Companion to the auth-prompt regression: a `BlockedApproval` run's
    /// `GatePrompt` also rendered to nothing. The prompt advertises the
    /// in-chat `approve`/`deny` reply (parsed by the shared grammar in
    /// `ironclaw_product_adapters::interaction_commands`) plus the web-app
    /// fallback — and the advertised command is round-tripped through that
    /// grammar below so copy and parser cannot drift.
    #[tokio::test]
    async fn render_outbound_gate_prompt_sends_webapp_redirect_and_records_delivered() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = ironclaw_product_adapters::FakeOutboundDeliverySink::new();
        let run_id = ironclaw_turns::TurnRunId::new();
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: test_outbound_target(),
            projection_cursor: test_projection_cursor(),
            payload: ProductOutboundPayload::GatePrompt(
                ironclaw_product_adapters::GatePromptView {
                    turn_run_id: run_id,
                    gate_ref: "gate:approval-1".into(),
                    invocation_id: None,
                    headline: "Approval needed".into(),
                    body: "I want to write notes.md to your workspace.".into(),
                    allow_always: false,
                    approval_context: None,
                },
            ),
            delivery_attempt_id: uuid::Uuid::new_v4(),
        };

        adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("gate prompt renders");

        let calls = egress.calls();
        assert_eq!(calls.len(), 1, "gate prompt is one sendMessage");
        assert_eq!(calls[0].path.as_str(), "/sendMessage");
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).expect("body json");
        assert_eq!(body["chat_id"], -100);
        let text = body["text"].as_str().expect("text");
        assert!(text.contains("Approval needed"), "headline: {text}");
        assert!(
            text.contains("Reply approve or deny in this chat"),
            "the prompt advertises the in-chat reply: {text}"
        );
        assert!(
            text.contains("IronClaw web app"),
            "the prompt keeps the web-app fallback: {text}"
        );
        // Drift guard at the adapter tier: the targeted command this copy
        // advertises must parse through the shared interaction grammar.
        let start = text
            .find("approve gate:")
            .expect("advertised targeted command");
        let advertised: String = text[start..]
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        let parsed = ironclaw_product_adapters::parse_interaction_resolution_text(
            &advertised,
            ironclaw_product_adapters::ProductTriggerReason::DirectChat,
        )
        .expect("advertised command is grammatical");
        assert!(
            matches!(
                parsed,
                Some(ironclaw_product_adapters::ProductInboundPayload::ApprovalResolution(_))
            ),
            "advertised command parses as an approval resolution, got {parsed:?}"
        );
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            matches!(
                statuses[0],
                DeliveryStatus::Delivered {
                    run_id: Some(recorded),
                    ..
                } if recorded == run_id
            ),
            "delivered with the originating run id for correlation, got {:?}",
            statuses[0]
        );
    }
}
