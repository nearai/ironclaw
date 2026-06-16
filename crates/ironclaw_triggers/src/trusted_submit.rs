use crate::{TriggerFire, TriggerInboundContentRef};

pub const TRIGGER_TRUSTED_ADAPTER_KIND: &str = "trigger";
pub const TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID: &str = "reborn-trigger-poller";
pub const TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE: &str = "user";

/// Returns `true` when `kind` is the trusted-trigger adapter kind string.
///
/// This is the trigger-owned authority on the predicate — callers in other
/// crates must use this function rather than comparing to `TRIGGER_TRUSTED_ADAPTER_KIND`
/// directly or carrying the check in a generic identifier type.
pub fn is_trusted_trigger_adapter_kind(kind: &str) -> bool {
    kind == TRIGGER_TRUSTED_ADAPTER_KIND
}

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
            adapter_kind: TRIGGER_TRUSTED_ADAPTER_KIND.to_string(),
            adapter_installation_id: TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID.to_string(),
            external_actor_namespace: TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE.to_string(),
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

#[cfg(test)]
mod tests {
    use super::is_trusted_trigger_adapter_kind;

    #[test]
    fn is_trusted_trigger_adapter_kind_matches_only_canonical_kind() {
        // The canonical value must match.
        assert!(is_trusted_trigger_adapter_kind("trigger"));

        // Non-canonical values must not match.
        assert!(!is_trusted_trigger_adapter_kind("telegram"));
        assert!(!is_trusted_trigger_adapter_kind("slack"));
        assert!(!is_trusted_trigger_adapter_kind("Trigger")); // wrong case
        assert!(!is_trusted_trigger_adapter_kind("")); // empty string
        assert!(!is_trusted_trigger_adapter_kind("trigger ")); // trailing space
    }
}
