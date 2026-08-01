//! The request bodies a transport hands to `ProductSurface` (PROPOSAL §6.1.3,
//! "inbound … product DTOs").
//!
//! Every one of these is a browser/API *body shape*: the JSON a WebUI route or
//! the OpenAI-compatible adapter deserializes before it reaches product. They
//! belong at the boundary because both transports construct them and neither
//! should compile `ironclaw_product` to do it.
//!
//! Deliberately **not** here — the normalization that turns a body into a
//! canonical command:
//!
//! - `ProductInboundCommand` and the `into_command` family stay in
//!   `ironclaw_product`, because the command carries
//!   `ironclaw_turns::CancelRunRequest` and contracts may not name the kernel.
//! - `decode_attachments` and `ProductAttachmentCapabilities` stay there too,
//!   because the byte budgets are `ironclaw_attachments` types. (CHECKLIST WS5
//!   "`attachments` widened … one home for size ceilings" owns that move.)
//!
//! So this module is field shapes plus the `serde` contract, and nothing that
//! validates, decodes, or dispatches.

use ironclaw_host_api::turn::SanitizedCancelReason;
use serde::{Deserialize, Serialize};

/// Browser body for WebUI create-thread mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductCreateThreadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_thread_id: Option<String>,
    /// Optional project the new thread should be scoped to. The browser only
    /// *proposes* it — the service authorizes the caller's access to the project
    /// before adopting it as scope, so the body is never trusted on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// One inline attachment in a browser send-message body.
///
/// `data_base64` is the base64-encoded file bytes; `mime_type` is validated
/// against the shared attachment format registry. This is the only place raw
/// upload bytes enter the workflow — they are decoded, budgeted, and landed in
/// storage, never carried on the (serializable) inbound command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductInboundAttachment {
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub data_base64: String,
}

/// Browser body for WebUI send-message mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductSubmitTurnRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ProductInboundAttachment>,
    /// Caller-selected model for this turn. A hint routed to when the operator
    /// has it configured, otherwise the run falls back to the deployment's
    /// active model. The `"default"` alias and empty values are treated as "no
    /// selection". `None` for clients that don't pick a model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Browser body for WebUI cancel-run mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductCancelRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Browser body for WebUI failed-run retry mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductRetryRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Browser query for WebUI list-threads read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductListThreadsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_thread_id: Option<String>,
    #[serde(default)]
    pub needs_approval: bool,
}

impl ProductListThreadsRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    pub fn set_candidate_thread_id(mut self, candidate_thread_id: impl Into<String>) -> Self {
        self.candidate_thread_id = Some(candidate_thread_id.into());
        self
    }

    pub fn set_needs_approval(mut self, needs_approval: bool) -> Self {
        self.needs_approval = needs_approval;
        self
    }
}

/// Browser query for WebUI automation listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductListAutomationsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_limit: Option<u32>,
    /// When `true`, soft-completed (fire-once) automations are included in the
    /// response alongside active ones. Defaults to `false` (active-only) so
    /// existing callers that do not set this flag are unaffected.
    #[serde(default)]
    pub include_completed: bool,
}

impl ProductListAutomationsRequest {
    pub fn set_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn set_run_limit(mut self, run_limit: u32) -> Self {
        self.run_limit = Some(run_limit);
        self
    }

    pub fn set_include_completed(mut self, include_completed: bool) -> Self {
        self.include_completed = include_completed;
        self
    }
}

/// Browser body for WebUI automation rename mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductRenameAutomationRequest {
    /// Optional at the DTO boundary so `{}` returns the stable field-level
    /// `missing_field` validation error instead of a generic JSON rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Browser body for WebUI extension-setup interaction.
///
/// This is the v2 entrypoint inventory's "extensions onboarding" row.
/// The native service exposes the route surface so callers can
/// inventory the API without v1 dependency. Concrete implementations return a
/// product-safe lifecycle projection; auth, approval, and pairing requirements
/// remain blockers owned by their dedicated Reborn services, not lifecycle
/// phases.
///
/// The package id is not part of the body — it is bound from the route
/// path and lifted into a lifecycle package ref by the handler before
/// it crosses the service boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductSetupExtensionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Browser body for WebUI gate-resolution mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductResolveGateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl From<ProductCancelReason> for SanitizedCancelReason {
    fn from(value: ProductCancelReason) -> Self {
        match value {
            ProductCancelReason::UserRequested => Self::UserRequested,
            ProductCancelReason::Superseded => Self::Superseded,
            ProductCancelReason::Timeout => Self::Timeout,
            ProductCancelReason::OperatorRequested => Self::OperatorRequested,
            ProductCancelReason::Policy => Self::Policy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum ProductGateResolution {
    Approved {
        #[serde(default)]
        always: bool,
    },
    /// Unified decline variant — covers both user-initiated approval denial
    /// and auth-gate cancellation; the wire value is "declined".
    Declined,
    /// A host-stored credential reference, not a raw secret/token.
    CredentialProvided { credential_ref: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list-request builders are the only way a caller narrows a read, and
    /// each setter is the difference between a bounded page and a full-table
    /// scan. They are consumed through `?limit=`/`?cursor=` query decoding, so
    /// a setter writing the wrong field is invisible until a page misbehaves.
    #[test]
    fn list_request_builders_set_exactly_the_field_they_name() {
        let threads = ProductListThreadsRequest::default()
            .set_limit(25)
            .set_cursor("cursor-9")
            .set_candidate_thread_id("thread-1")
            .set_needs_approval(true);
        assert_eq!(threads.limit, Some(25));
        assert_eq!(threads.cursor.as_deref(), Some("cursor-9"));
        assert_eq!(threads.candidate_thread_id.as_deref(), Some("thread-1"));
        assert!(threads.needs_approval);

        let automations = ProductListAutomationsRequest::default()
            .set_limit(10)
            .set_run_limit(3)
            .set_include_completed(true);
        assert_eq!(automations.limit, Some(10));
        assert_eq!(automations.run_limit, Some(3));
        assert!(automations.include_completed);

        // The defaults are the contract for a caller that sets nothing: an
        // unset limit means "service default", and completed automations are
        // excluded unless asked for.
        let untouched = ProductListAutomationsRequest::default();
        assert_eq!(untouched.limit, None);
        assert_eq!(untouched.run_limit, None);
        assert!(!untouched.include_completed);
        assert!(!ProductListThreadsRequest::default().needs_approval);
    }

    /// The wire tag set is the browser contract; a renamed variant silently
    /// breaks every deployed client, so the encoding is pinned here rather than
    /// only where the value is produced.
    #[test]
    fn gate_resolution_and_cancel_reason_keep_their_wire_tags() {
        assert_eq!(
            serde_json::to_value(ProductGateResolution::Approved { always: true })
                .expect("serialize"),
            serde_json::json!({"resolution": "approved", "always": true})
        );
        assert_eq!(
            serde_json::to_value(ProductGateResolution::Declined).expect("serialize"),
            serde_json::json!({"resolution": "declined"})
        );
        assert_eq!(
            serde_json::to_value(ProductGateResolution::CredentialProvided {
                credential_ref: "cred-1".to_string(),
            })
            .expect("serialize"),
            serde_json::json!({"resolution": "credential_provided", "credential_ref": "cred-1"})
        );
        assert_eq!(
            serde_json::to_value(ProductCancelReason::OperatorRequested).expect("serialize"),
            serde_json::json!("operator_requested")
        );
        assert_eq!(
            SanitizedCancelReason::from(ProductCancelReason::Superseded),
            SanitizedCancelReason::Superseded
        );
    }
}
