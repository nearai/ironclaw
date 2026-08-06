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
    use std::sync::{Arc, Mutex};

    /// A double that **derives its answer from the whole request** rather than
    /// returning a fixed one.
    ///
    /// That matters more than it looks: a double that ignored the request and
    /// always answered the same way would make every test below vacuous — they
    /// would pass just as happily against a port that dropped its argument on
    /// the floor, which is exactly the shape this port exists to prevent. So
    /// the answer is looked up by the full `(adapter, installation, route)`
    /// triple, and every field is echoed back for inspection.
    #[derive(Debug, Default)]
    struct RouteKeyedResolver {
        seen: Mutex<Vec<ProductConversationSubjectRouteResolutionRequest>>,
        subjects: Vec<(ProductConversationSubjectRouteResolutionRequest, UserId)>,
    }

    #[async_trait]
    impl ProductConversationSubjectRouteResolver for RouteKeyedResolver {
        async fn resolve_product_conversation_subject_route(
            &self,
            request: ProductConversationSubjectRouteResolutionRequest,
        ) -> Result<Option<UserId>, ProductOperationFailure> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.clone());
            Ok(self
                .subjects
                .iter()
                .find(|(configured, _)| configured == &request)
                .map(|(_, subject)| subject.clone()))
        }
    }

    fn request_for(
        adapter: &str,
        installation: &str,
        space: Option<&str>,
        conversation: &str,
    ) -> ProductConversationSubjectRouteResolutionRequest {
        ProductConversationSubjectRouteResolutionRequest {
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

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("valid user")
    }

    /// The port is held as `Arc<dyn ProductConversationSubjectRouteResolver>`
    /// by both product and the extension host, so object safety is a contract,
    /// not an implementation detail: a non-dispatchable method added here would
    /// break both callers at once.
    ///
    /// **Scope.** This pins the port's *shape* — that a resolver is handed every
    /// field unswapped and that the signature admits a different answer per
    /// route. It does **not** pin that any production resolver looks the route
    /// up correctly; `ChannelConfigSubjectRouteResolver`'s own tests in
    /// `ironclaw_extension_host` own that claim.
    #[tokio::test]
    async fn the_port_is_object_safe_and_answers_differ_by_the_route_it_is_handed() {
        let engineering = request_for("slack-like", "install-1", Some("space-1"), "eng");
        let support = request_for("slack-like", "install-1", Some("space-1"), "support");
        let other_install = request_for("slack-like", "install-2", Some("space-1"), "eng");

        let resolver = Arc::new(RouteKeyedResolver {
            subjects: vec![
                (engineering.clone(), user("eng-subject")),
                (support.clone(), user("support-subject")),
            ],
            ..RouteKeyedResolver::default()
        });
        let port: Arc<dyn ProductConversationSubjectRouteResolver> = resolver.clone();

        // Both directions: two configured routes resolve to *different*
        // subjects, so a resolver that ignored the route could not pass.
        assert_eq!(
            port.resolve_product_conversation_subject_route(engineering.clone())
                .await
                .expect("configured route resolves"),
            Some(user("eng-subject"))
        );
        assert_eq!(
            port.resolve_product_conversation_subject_route(support)
                .await
                .expect("configured route resolves"),
            Some(user("support-subject"))
        );

        // The installation is part of the identity, not decoration: the same
        // conversation under a different install is a different route.
        assert_eq!(
            port.resolve_product_conversation_subject_route(other_install)
                .await
                .expect("unconfigured route is not an error"),
            None
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

    /// "No subject is configured for this route" is a routing decision the
    /// caller turns into a rejection — never an error. If this ever became
    /// `Err`, an unconfigured shared channel would report a backend failure
    /// instead of an unroutable conversation. Paired with the test above, which
    /// shows the same resolver *can* answer `Some`, so this is absence rather
    /// than a resolver that never answers at all.
    #[tokio::test]
    async fn an_unconfigured_route_is_absence_not_failure() {
        let port: Arc<dyn ProductConversationSubjectRouteResolver> =
            Arc::new(RouteKeyedResolver::default());
        let resolved = port
            .resolve_product_conversation_subject_route(request_for(
                "slack-like",
                "install-1",
                Some("space-1"),
                "eng",
            ))
            .await
            .expect("an unconfigured route is not an error");
        assert_eq!(resolved, None);
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
