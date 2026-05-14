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
use crate::render::{render_final_reply, render_progress_typing};

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
            _ => None,
        };

        let request = match envelope.payload {
            ProductOutboundPayload::FinalReply(view) => match render_final_reply(
                &envelope.target.reply_target_binding_ref,
                &view,
                self.config.egress_credential_handle.clone(),
            ) {
                Ok(req) => req,
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
            },
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
                match render_progress_typing(
                    &envelope.target.reply_target_binding_ref,
                    &view,
                    self.config.egress_credential_handle.clone(),
                ) {
                    Ok(Some(req)) => req,
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
            ProductOutboundPayload::GatePrompt(_) | ProductOutboundPayload::AuthPrompt(_) => {
                // Deferred to #3094. The workflow renders a placeholder body
                // via this branch in fake contract tests; real production
                // flows do not produce gate envelopes for Telegram yet.
                record_status(
                    delivery_sink,
                    DeliveryStatus::Deferred {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id: None,
                        reason: RedactedString::new(
                            "gate/auth prompts deferred to #3094 on Telegram",
                        ),
                    },
                )
                .await;
                return Ok(ProductRenderOutcome::Deferred);
            }
            ProductOutboundPayload::ProjectionSnapshot { .. }
            | ProductOutboundPayload::ProjectionUpdate { .. } => {
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

        let response = match egress.send(request).await {
            Ok(resp) => resp,
            Err(egress_err) => {
                record_status(
                    delivery_sink,
                    egress_err_to_delivery_status(
                        &egress_err,
                        attempt_id,
                        target_binding.clone(),
                        run_id,
                    ),
                )
                .await;
                return Err(map_egress_error(egress_err));
            }
        };

        if !(200..300).contains(&response.status()) {
            let reason = RedactedString::new(format!(
                "telegram bot api returned status {}",
                response.status()
            ));
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
}
