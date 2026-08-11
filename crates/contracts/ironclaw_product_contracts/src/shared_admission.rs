//! Shared-conversation admission.
//!
//! A shared external conversation (a team channel, a group thread) runs its
//! turns as whoever spoke — a run acts as the user who invoked it — so the
//! only routing question left to installation configuration is whether the
//! conversation is CONNECTED to this deployment at all. Which conversations
//! those are depends on channel configuration the product does not own, so
//! product asks an admission resolver wired beside it.
//!
//! The port is declared here and implemented by the extension host, which
//! reads the channel configuration (PROPOSAL §6.1.3). It replaced the retired
//! `ProductConversationSubjectRouteResolver`: the subject half (which user a
//! shared conversation runs as) is gone with shared-route subject binding;
//! the admission half stays fail-closed.

use async_trait::async_trait;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};

use crate::error::ProductOperationFailure;

/// Stable conversation route key used by hosts to admit shared conversations.
///
/// The key is `(space, conversation)` and intentionally ignores topic/thread
/// ids, so every thread inside one connected conversation is admitted
/// together. Which vendor identifiers those two fields carry is the channel
/// package's business, never this crate's.
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

/// Request passed to host-owned shared-conversation admission resolvers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SharedConversationAdmissionRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub route_key: ProductConversationRouteKey,
}

/// Decide whether a shared conversation route is connected to this
/// deployment.
///
/// `Ok(false)` means "this conversation is not connected" — a routing
/// decision the caller turns into a rejection, never an error. Admission is
/// fail-closed: a shared conversation with no resolver, or one the resolver
/// does not admit, never reaches binding resolution.
#[async_trait]
pub trait SharedConversationAdmission: Send + Sync + std::fmt::Debug {
    async fn shared_conversation_admitted(
        &self,
        request: SharedConversationAdmissionRequest,
    ) -> Result<bool, ProductOperationFailure>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A double that **derives its answer from the whole request** rather than
    /// returning a fixed one — a double that ignored the request would make
    /// every test below pass against a port that dropped its argument on the
    /// floor, which is exactly the shape this port exists to prevent. The
    /// admitted set is keyed by the full `(adapter, installation, route)`
    /// triple, and every field is echoed back for inspection.
    #[derive(Debug, Default)]
    struct RouteKeyedAdmission {
        seen: Mutex<Vec<SharedConversationAdmissionRequest>>,
        admitted: Vec<SharedConversationAdmissionRequest>,
    }

    #[async_trait]
    impl SharedConversationAdmission for RouteKeyedAdmission {
        async fn shared_conversation_admitted(
            &self,
            request: SharedConversationAdmissionRequest,
        ) -> Result<bool, ProductOperationFailure> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.clone());
            Ok(self.admitted.contains(&request))
        }
    }

    fn request_for(
        adapter: &str,
        installation: &str,
        space: Option<&str>,
        conversation: &str,
    ) -> SharedConversationAdmissionRequest {
        SharedConversationAdmissionRequest {
            adapter_id: ProductAdapterId::new(adapter).expect("valid adapter id"),
            installation_id: AdapterInstallationId::new(installation)
                .expect("valid installation id"),
            route_key: ProductConversationRouteKey::new(
                space.map(str::to_string),
                conversation.to_string(),
            )
            .expect("valid route key"),
        }
    }

    /// The port is held as `Arc<dyn SharedConversationAdmission>` by both
    /// product and the extension host, so object safety is a contract, not an
    /// implementation detail.
    ///
    /// **Scope.** This pins the port's *shape* — that a resolver is handed
    /// every field unswapped and that the signature admits a different answer
    /// per route. It does **not** pin any production resolver's policy;
    /// `PresenceSharedAdmission`'s own tests in `ironclaw_extension_host`
    /// own that claim.
    #[tokio::test]
    async fn the_port_is_object_safe_and_answers_differ_by_the_route_it_is_handed() {
        let engineering = request_for("slack-like", "install-1", Some("space-1"), "eng");
        let support = request_for("slack-like", "install-1", Some("space-1"), "support");
        let other_install = request_for("slack-like", "install-2", Some("space-1"), "eng");

        let resolver = Arc::new(RouteKeyedAdmission {
            admitted: vec![engineering.clone()],
            ..RouteKeyedAdmission::default()
        });
        let port: Arc<dyn SharedConversationAdmission> = resolver.clone();

        assert!(
            port.shared_conversation_admitted(engineering.clone())
                .await
                .expect("connected route admits")
        );
        assert!(
            !port
                .shared_conversation_admitted(support)
                .await
                .expect("an unconnected route is not an error")
        );
        // The installation is part of the identity, not decoration: the same
        // conversation under a different install is a different route.
        assert!(
            !port
                .shared_conversation_admitted(other_install)
                .await
                .expect("an unconnected route is not an error")
        );

        // `adapter_id` and `installation_id` are both string-backed newtypes,
        // so a swapped argument would otherwise be silent.
        let seen = resolver
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(seen.len(), 3, "every call reaches the implementation");
        assert_eq!(seen[0].adapter_id.as_str(), "slack-like");
        assert_eq!(seen[0].installation_id.as_str(), "install-1");
        assert_eq!(seen[0].route_key.space_id(), Some("space-1"));
        assert_eq!(seen[0].route_key.conversation_id(), "eng");
        assert_eq!(seen[2].installation_id.as_str(), "install-2");
    }

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
    /// connected conversation is admitted together.
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
