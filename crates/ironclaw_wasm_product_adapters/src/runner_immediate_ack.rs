//! Immediate-ACK dispatch path for native ProductAdapter webhooks.
//!
//! Protocols such as Slack must receive a 2xx response before the ProductWorkflow
//! result is available. This extension keeps the verified parse/stamp/admission
//! preparation shared with the synchronous runner path, then schedules bounded
//! workflow dispatch after the protocol ACK is safe to return.

use std::sync::Arc;

use ironclaw_product_adapters::{InboundRetryDisposition, ProtocolAuthEvidence};

use super::{NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome};

impl NativeProductAdapterRunner {
    /// Verify, parse, stamp, and schedule workflow dispatch without waiting for
    /// the workflow result. This is the path for protocols that require an
    /// immediate webhook ACK after authentication and syntactic normalization.
    pub async fn process_webhook_immediate_ack(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let evidence = self.verify_webhook_auth(headers, body)?;
        self.process_verified_webhook_immediate_ack(body, &evidence)
            .await
    }

    /// Schedule a previously verified webhook payload. Exposed for
    /// protocol-specific handlers that must verify once, handle a special
    /// synchronous protocol handshake, and then continue into the normal async
    /// ProductWorkflow dispatch path for ordinary events.
    pub async fn process_verified_webhook_immediate_ack(
        &self,
        body: &[u8],
        evidence: &ProtocolAuthEvidence,
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let (envelope, permit) = self.prepare_inbound_envelope(body, evidence)?;
        let workflow = Arc::clone(&self.workflow);
        let workflow_timeout = self.config.workflow_timeout;
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(workflow_timeout, workflow.accept_inbound(envelope)).await {
                Ok(Ok(ack)) if ack.retry_disposition() == InboundRetryDisposition::Retry => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        "async webhook workflow dispatch requested retry after protocol ack"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        error = %error,
                        "async webhook workflow dispatch failed after protocol ack"
                    );
                }
                Err(_) => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        timeout_ms = workflow_timeout.as_millis() as u64,
                        "async webhook workflow dispatch timed out after protocol ack"
                    );
                }
            }
        });
        Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch)
    }
}
