//! Telegram v2 ProductAdapter implementation.

use async_trait::async_trait;
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, EgressCredentialHandle, ProductAdapter,
    ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId, ProductCapabilityFlag,
    ProductInboundEnvelope, ProductOutboundEnvelope, ProductOutboundPayload, ProductSurfaceKind,
    ProtocolAuthEvidence, ProtocolHttpEgress, redaction::RedactedString,
};

use crate::payload::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramParsedInbound, parse_telegram_update,
};
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
    /// If true, the adapter advertises `ExternalProgressPush` and renders
    /// typing indicators on outbound `Progress` envelopes. Default: false
    /// (#3266 progress-opt-in policy).
    pub progress_push_enabled: bool,
}

pub struct TelegramV2Adapter {
    config: TelegramV2AdapterConfig,
    capabilities: ProductAdapterCapabilities,
}

impl TelegramV2Adapter {
    pub fn new(config: TelegramV2AdapterConfig) -> Self {
        let mut capabilities = ProductAdapterCapabilities::external_channel_default();
        if config.progress_push_enabled {
            capabilities = capabilities.with(ProductCapabilityFlag::ExternalProgressPush);
        }
        Self {
            config,
            capabilities,
        }
    }

    pub fn config(&self) -> &TelegramV2AdapterConfig {
        &self.config
    }
}

/// Egress hosts that any Telegram v2 installation may target.
pub fn telegram_declared_egress_hosts() -> Vec<DeclaredEgressHost> {
    vec![DeclaredEgressHost::new(TELEGRAM_API_HOST).expect("static host valid")]
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

    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: ProtocolAuthEvidence,
    ) -> Result<Option<ProductInboundEnvelope>, ProductAdapterError> {
        let parsed = parse_telegram_update(
            raw_payload,
            auth_evidence,
            &self.config.adapter_id,
            &self.config.installation_id,
            &self.config.group_trigger_policy,
        )
        .map_err(|err| match err {
            crate::payload::PayloadParseError::UnauthenticatedPayload => {
                ProductAdapterError::Authentication(
                    ironclaw_product_adapters::ProtocolAuthFailure::Other {
                        detail: RedactedString::new(
                            "telegram parse_inbound called with unverified evidence",
                        ),
                    },
                )
            }
            crate::payload::PayloadParseError::InvalidJson { reason } => {
                ProductAdapterError::MalformedInboundPayload { reason }
            }
            crate::payload::PayloadParseError::MissingUpdateId => {
                ProductAdapterError::MalformedInboundPayload {
                    reason: "telegram update missing update_id".into(),
                }
            }
            crate::payload::PayloadParseError::InvalidExternalRef { kind, reason } => {
                ProductAdapterError::InvalidIdentifier { kind, reason }
            }
        })?;
        match parsed {
            TelegramParsedInbound::Envelope(envelope) => Ok(Some(*envelope)),
            TelegramParsedInbound::NoOp => Ok(None),
        }
    }

    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
    ) -> Result<(), ProductAdapterError> {
        let request = match envelope.payload {
            ProductOutboundPayload::FinalReply(view) => render_final_reply(
                &envelope.target,
                &view,
                self.config.egress_credential_handle.clone(),
            )
            .map_err(|err| ProductAdapterError::Internal {
                detail: RedactedString::new(err.to_string()),
            })?,
            ProductOutboundPayload::Progress(view) => {
                if !self
                    .capabilities
                    .contains(ProductCapabilityFlag::ExternalProgressPush)
                {
                    // Progress not advertised; drop silently. The workflow
                    // would not normally route a Progress envelope to a
                    // capability-ungated adapter, but this defends against a
                    // misrouted envelope reaching us.
                    return Ok(());
                }
                let Some(req) = render_progress_typing(
                    &envelope.target,
                    &view,
                    self.config.egress_credential_handle.clone(),
                )
                .map_err(|err| ProductAdapterError::Internal {
                    detail: RedactedString::new(err.to_string()),
                })?
                else {
                    return Ok(());
                };
                req
            }
            ProductOutboundPayload::GatePrompt(_) | ProductOutboundPayload::AuthPrompt(_) => {
                // Deferred to #3094. The workflow renders a placeholder body
                // via this branch in fake contract tests; real production
                // flows do not produce gate envelopes for Telegram yet.
                return Ok(());
            }
            ProductOutboundPayload::ProjectionSnapshot(_)
            | ProductOutboundPayload::ProjectionUpdate(_) => {
                // Telegram never consumes projection subscriptions; the
                // workflow should not route these to a Telegram installation.
                // Return Ok to keep delivery best-effort.
                return Ok(());
            }
        };

        let response =
            egress
                .send(request)
                .await
                .map_err(|err| ProductAdapterError::EgressDenied {
                    reason: err.to_string(),
                })?;
        if !(200..300).contains(&response.status) {
            return Err(ProductAdapterError::EgressDenied {
                reason: format!("telegram bot api returned status {}", response.status),
            });
        }
        Ok(())
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
            progress_push_enabled: progress,
        }
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
    fn parse_inbound_refuses_unverified_evidence() {
        let adapter = TelegramV2Adapter::new(config(false));
        let unverified = ProtocolAuthEvidence::Failed {
            failure: ironclaw_product_adapters::ProtocolAuthFailure::SharedSecretMismatch,
        };
        let err = adapter
            .parse_inbound(b"{\"update_id\":1}", unverified)
            .expect_err("must fail");
        assert!(matches!(err, ProductAdapterError::Authentication(_)));
    }

    #[tokio::test]
    async fn render_outbound_final_reply_uses_constrained_egress() {
        let adapter = TelegramV2Adapter::new(config(false));
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: crate::render::build_reply_target_binding(-100, Some(7), Some(42)),
            projection_cursor: None,
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
            .render_outbound(envelope, &egress)
            .await
            .expect("render ok");
        let calls = egress.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].host, "api.telegram.org");
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/sendMessage");
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
        let envelope = ProductOutboundEnvelope {
            adapter_id: adapter.adapter_id().clone(),
            installation_id: adapter.installation_id().clone(),
            target: crate::render::build_reply_target_binding(-100, None, None),
            projection_cursor: None,
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
            .render_outbound(envelope, &egress)
            .await
            .expect("ok");
        // Progress is not advertised -> egress NOT called.
        assert!(egress.calls().is_empty());
    }
}
