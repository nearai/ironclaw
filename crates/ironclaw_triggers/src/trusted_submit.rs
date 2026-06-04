use crate::{TriggerFire, TriggerInboundContentRef};

/// Canonical conversation identity for a trusted trigger fire.
///
/// Composition computes this once while materializing the trigger prompt, uses
/// the same values for prompt recording, and carries them in the sealed trusted
/// submit request. Downstream submitters must not re-derive these binding keys
/// from `TriggerFire`, because drift would split idempotency across bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerTrustedInboundBinding {
    adapter_kind: String,
    adapter_installation_id: String,
    external_actor_namespace: String,
    external_actor_id: String,
    external_conversation_id: String,
    route_thread_id: String,
    external_event_id: String,
}

impl TriggerTrustedInboundBinding {
    pub fn for_fire(fire: &TriggerFire) -> Self {
        Self {
            adapter_kind: "trigger".to_string(),
            adapter_installation_id: "reborn-trigger-poller".to_string(),
            external_actor_namespace: "user".to_string(),
            external_actor_id: fire.creator_user_id.as_str().to_string(),
            external_conversation_id: format!("trigger-{}", fire.identity.trigger_id()),
            route_thread_id: fire.identity.route_thread_id().as_str().to_string(),
            external_event_id: fire.identity.external_event_id().as_str().to_string(),
        }
    }

    pub fn adapter_kind(&self) -> &str {
        &self.adapter_kind
    }

    pub fn adapter_installation_id(&self) -> &str {
        &self.adapter_installation_id
    }

    pub fn external_actor_namespace(&self) -> &str {
        &self.external_actor_namespace
    }

    pub fn external_actor_id(&self) -> &str {
        &self.external_actor_id
    }

    pub fn external_conversation_id(&self) -> &str {
        &self.external_conversation_id
    }

    pub fn route_thread_id(&self) -> &str {
        &self.route_thread_id
    }

    pub fn external_event_id(&self) -> &str {
        &self.external_event_id
    }
}

/// Materialized prompt content plus the canonical trusted inbound binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMaterializedPrompt {
    content_ref: TriggerInboundContentRef,
    trusted_inbound_binding: TriggerTrustedInboundBinding,
}

impl TriggerMaterializedPrompt {
    /// Pair materialized trigger prompt content with the canonical trusted
    /// inbound binding computed for the same fire.
    ///
    /// Concrete materializers are responsible for ensuring `content_ref` was
    /// produced from the `TriggerFire` that also produced
    /// `trusted_inbound_binding`.
    pub fn new(
        content_ref: TriggerInboundContentRef,
        trusted_inbound_binding: TriggerTrustedInboundBinding,
    ) -> Self {
        Self {
            content_ref,
            trusted_inbound_binding,
        }
    }

    /// Create a materialized prompt result for a specific fire.
    ///
    /// `content_ref` must identify content materialized from the exact `fire`
    /// passed here. The worker carries this paired value into
    /// `TrustedTriggerSubmitRequest` without exposing request construction to
    /// downstream crates.
    pub fn for_fire(fire: &TriggerFire, content_ref: TriggerInboundContentRef) -> Self {
        Self::new(content_ref, TriggerTrustedInboundBinding::for_fire(fire))
    }

    pub fn content_ref(&self) -> &TriggerInboundContentRef {
        &self.content_ref
    }

    pub fn trusted_inbound_binding(&self) -> &TriggerTrustedInboundBinding {
        &self.trusted_inbound_binding
    }

    pub fn into_parts(self) -> (TriggerInboundContentRef, TriggerTrustedInboundBinding) {
        (self.content_ref, self.trusted_inbound_binding)
    }
}
