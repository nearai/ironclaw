//! Durable references for retractable run-delivery messages.
//!
//! A provider-issued message reference is the evidence needed to retract a
//! working or gate notification after restart. These records contain only that
//! opaque reference plus the sealed run/source identities needed to reopen and
//! revalidate the route; they never contain message content or credentials.

use ironclaw_turns::{ReplyTargetBindingRef, RunOriginAdapter, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

/// Hard bound for one run/source cleanup snapshot.
pub const MAX_RUN_DELIVERY_CLEANUP_RECORDS: usize = 64;

const MAX_VENDOR_MESSAGE_REF_BYTES: usize = 2_048;
const MAX_CONVERSATION_FINGERPRINT_BYTES: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDeliveryCleanupRequest {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    pub adapter: RunOriginAdapter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDeliveryCleanupRecord {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    pub adapter: RunOriginAdapter,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub conversation_fingerprint: String,
    pub vendor_message_ref: String,
}

impl RunDeliveryCleanupRecord {
    pub fn new(
        scope: TurnScope,
        run_id: TurnRunId,
        adapter: RunOriginAdapter,
        reply_target_binding_ref: ReplyTargetBindingRef,
        conversation_fingerprint: String,
        vendor_message_ref: String,
    ) -> Result<Self, &'static str> {
        if conversation_fingerprint.is_empty()
            || conversation_fingerprint.len() > MAX_CONVERSATION_FINGERPRINT_BYTES
            || conversation_fingerprint.chars().any(char::is_control)
        {
            return Err("run delivery cleanup conversation fingerprint is invalid");
        }
        if vendor_message_ref.is_empty()
            || vendor_message_ref.len() > MAX_VENDOR_MESSAGE_REF_BYTES
            || vendor_message_ref.chars().any(char::is_control)
        {
            return Err("run delivery cleanup vendor message reference is invalid");
        }
        Ok(Self {
            scope,
            run_id,
            adapter,
            reply_target_binding_ref,
            conversation_fingerprint,
            vendor_message_ref,
        })
    }

    pub fn request(&self) -> RunDeliveryCleanupRequest {
        RunDeliveryCleanupRequest {
            scope: self.scope.clone(),
            run_id: self.run_id,
            adapter: self.adapter.clone(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        Self::new(
            self.scope.clone(),
            self.run_id,
            self.adapter.clone(),
            self.reply_target_binding_ref.clone(),
            self.conversation_fingerprint.clone(),
            self.vendor_message_ref.clone(),
        )
        .map(|_| ())
    }
}
