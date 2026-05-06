//! Native ProductAdapter runner.
//!
//! `NativeProductAdapterRunner` is the integration point that turns a single
//! webhook request into the full Reborn pipeline:
//!
//! 1. Authenticate the protocol payload with a [`WebhookAuthVerifier`].
//! 2. On success, mint a `Verified` evidence via the public `mark_*_verified`
//!    helpers in `ironclaw_product_adapters::auth`.
//! 3. Hand the verified evidence + raw payload to the adapter's
//!    [`ironclaw_product_adapters::ProductAdapter::parse_inbound`].
//! 4. Forward the resulting envelope to the [`ironclaw_product_adapters::ProductWorkflow`]
//!    facade and return the structured outcome.
//!
//! The runner is deliberately not wasmtime-bound — the v2 component-model
//! plumbing lands in a follow-up. Telegram v2 today implements
//! `ProductAdapter` natively in Rust; the runner enforces the same auth /
//! dedupe / facade-only contract a wasmtime instance would.

use std::sync::Arc;

use ironclaw_product_adapters::auth::{
    mark_bearer_token_verified, mark_request_signature_verified, mark_session_verified,
    mark_shared_secret_header_verified,
};
use ironclaw_product_adapters::{
    ProductAdapter, ProductAdapterError, ProductInboundAck, ProductWorkflow, ProtocolAuthEvidence,
    ProtocolAuthFailure,
};
use thiserror::Error;

use crate::auth_verifier::{
    HmacWebhookAuth, SharedSecretHeaderAuth, VerificationOutcome, WebhookAuthVerifier,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunnerError {
    #[error("webhook authentication failed: {failure}")]
    AuthenticationFailed { failure: ProtocolAuthFailure },
    #[error(transparent)]
    Adapter(#[from] ProductAdapterError),
}

impl RunnerError {
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, RunnerError::AuthenticationFailed { .. })
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            RunnerError::AuthenticationFailed { .. } => false,
            RunnerError::Adapter(err) => err.is_retryable(),
        }
    }
}

/// What the protocol layer should do with the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookProcessOutcome {
    /// Auth succeeded, adapter parsed an envelope, workflow accepted it.
    Acknowledged { ack: ProductInboundAck },
    /// Auth succeeded but the adapter chose to drop the message (group
    /// ambient, edited message, unsupported event kind, ...). The protocol
    /// layer should respond 200 OK no-op.
    NoOp,
}

/// Webhook auth strategy.
pub enum WebhookAuth {
    Hmac(HmacWebhookAuth),
    SharedSecretHeader(SharedSecretHeaderAuth),
}

impl WebhookAuth {
    fn verify(&self, headers: &http::HeaderMap, body: &[u8]) -> VerificationOutcome {
        match self {
            WebhookAuth::Hmac(v) => v.verify(headers, body),
            WebhookAuth::SharedSecretHeader(v) => v.verify(headers, body),
        }
    }

    fn mint_evidence(&self, subject: String) -> ProtocolAuthEvidence {
        match self {
            WebhookAuth::Hmac(v) => mark_request_signature_verified(
                v.signature_header.clone(),
                Some(v.timestamp_header.clone()),
                subject,
            ),
            WebhookAuth::SharedSecretHeader(v) => {
                mark_shared_secret_header_verified(v.header_name.clone(), subject)
            }
        }
    }
}

/// Convenience constructor for synchronous-API or CLI auth bridges.
pub fn evidence_from_session_subject(subject: impl Into<String>) -> ProtocolAuthEvidence {
    mark_session_verified("ironclaw_session", subject)
}

pub fn evidence_from_bearer_subject(subject: impl Into<String>) -> ProtocolAuthEvidence {
    mark_bearer_token_verified(subject)
}

pub struct NativeProductAdapterRunner {
    adapter: Arc<dyn ProductAdapter>,
    workflow: Arc<dyn ProductWorkflow>,
    auth: WebhookAuth,
}

impl NativeProductAdapterRunner {
    pub fn new(
        adapter: Arc<dyn ProductAdapter>,
        workflow: Arc<dyn ProductWorkflow>,
        auth: WebhookAuth,
    ) -> Self {
        Self {
            adapter,
            workflow,
            auth,
        }
    }

    pub async fn process_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let evidence = match self.auth.verify(headers, body) {
            VerificationOutcome::Verified { subject } => self.auth.mint_evidence(subject),
            VerificationOutcome::Failed { failure } => {
                return Err(RunnerError::AuthenticationFailed { failure });
            }
        };
        let Some(envelope) = self.adapter.parse_inbound(body, evidence)? else {
            return Ok(WebhookProcessOutcome::NoOp);
        };
        let ack = self.workflow.accept_inbound(envelope).await?;
        Ok(WebhookProcessOutcome::Acknowledged { ack })
    }
}
