//! Shared-route subject resolution.
//!
//! A shared external conversation (a team channel, a group thread) runs its
//! turns under a configured *subject* user rather than under whoever spoke.
//! Which subject that is depends on installation configuration the product
//! does not own, so product asks a resolver wired beside it.
//!
//! The port is declared here and implemented by the extension host, which
//! reads the channel configuration (PROPOSAL §6.1.3). It became declarable
//! here once its error stopped being product's workflow type — see
//! [`crate::error::ProductOperationFailure`].

use async_trait::async_trait;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};

use crate::error::ProductOperationFailure;

/// Stable conversation route key used by hosts to assign shared-route subjects.
///
/// The key is `(space, conversation)` and intentionally ignores topic/thread
/// ids, so every thread inside one configured conversation runs under the same
/// shared subject while retaining its own conversation context. Which vendor
/// identifiers those two fields carry is the channel package's business, never
/// this crate's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductConversationRouteKey {
    space_id: Option<String>,
    conversation_id: String,
}

impl ProductConversationRouteKey {
    pub fn new(
        space_id: Option<String>,
        conversation_id: String,
    ) -> Result<Self, ProductOperationFailure> {
        ExternalConversationRef::new(space_id.as_deref(), conversation_id.as_str(), None, None)
            .map_err(|error| ProductOperationFailure::InvalidBindingRequest {
                reason: format!("invalid conversation route key: {error}"),
            })?;
        Ok(Self {
            space_id,
            conversation_id,
        })
    }

    /// Derive the key from an already-validated external conversation ref.
    ///
    /// Infallible by construction: the ref was validated when it was built, so
    /// re-running [`Self::new`]'s check could only fail on an already-broken
    /// value.
    pub fn from_external_conversation_ref(conversation_ref: &ExternalConversationRef) -> Self {
        Self {
            space_id: conversation_ref.space_id().map(str::to_string),
            conversation_id: conversation_ref.conversation_id().to_string(),
        }
    }

    pub fn space_id(&self) -> Option<&str> {
        self.space_id.as_deref()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
}

/// Request passed to host-owned shared-route subject resolvers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductConversationSubjectRouteResolutionRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub route_key: ProductConversationRouteKey,
}

/// Resolve the subject a shared conversation route runs under.
///
/// `Ok(None)` means "no subject is configured for this route" — a routing
/// decision the caller turns into a rejection, never an error.
#[async_trait]
pub trait ProductConversationSubjectRouteResolver: Send + Sync + std::fmt::Debug {
    async fn resolve_product_conversation_subject_route(
        &self,
        request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductOperationFailure>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_key_rejects_a_conversation_id_that_is_not_a_valid_external_ref() {
        let error = ProductConversationRouteKey::new(None, String::new())
            .expect_err("a blank conversation id is not a valid route key");
        assert!(matches!(
            error,
            ProductOperationFailure::InvalidBindingRequest { .. }
        ));
    }

    /// The key deliberately drops topic/thread identity so every thread in a
    /// configured channel resolves to the same subject.
    #[test]
    fn route_key_from_external_ref_keeps_space_and_conversation_only() {
        let conversation_ref =
            ExternalConversationRef::new(Some("T123"), "C456", Some("1700000000.1"), None)
                .expect("valid external conversation ref");
        let key = ProductConversationRouteKey::from_external_conversation_ref(&conversation_ref);
        assert_eq!(key.space_id(), Some("T123"));
        assert_eq!(key.conversation_id(), "C456");
        assert_eq!(
            key,
            ProductConversationRouteKey::new(Some("T123".to_string()), "C456".to_string())
                .expect("validated key")
        );
    }
}
