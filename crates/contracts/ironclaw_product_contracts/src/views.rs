//! The generic product-view conduit's descriptor types and provider port
//! (PROPOSAL §6.1.3).
//!
//! Product features register a read-only view instead of growing
//! `ProductSurface` with a feature-specific query method. The *inventory* of
//! concrete views is product's frozen surface and stays there; the descriptor,
//! the page envelope, and the port a view provider implements are boundary
//! vocabulary, because providers legitimately sit outside product — the
//! admin-configuration view is implemented by `ironclaw_extension_manager`
//! (WS2.4).
//!
//! Never here: any concrete view id, or any provider implementation. The typed
//! `ProductView` declaration wrapper lives beside the other two operation
//! shapes in [`crate::descriptors`] — WS1.4 left it in product because it
//! carried product-defined request/response DTOs, and the WS5 port inversion
//! removed that reason (PROPOSAL §6.1.3 names it as a descriptor *type*).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::surface::{ProductSurfaceCaller, ProductSurfaceError};

/// Stable metadata for one read-only product view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebornViewDescriptor {
    pub id: &'static str,
    pub paginated: bool,
}

/// One registered, read-only product view invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornViewQuery {
    pub view_id: String,
    pub params: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// One page returned by the generic product view conduit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornViewPage {
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One composition-supplied implementation behind the generic view conduit.
///
/// Product features register descriptors and providers instead of growing
/// `ProductSurface` with feature-specific read methods.
#[async_trait]
pub trait RebornViewProvider: Send + Sync {
    fn descriptor(&self) -> RebornViewDescriptor;

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        params: serde_json::Value,
        cursor: Option<String>,
    ) -> Result<RebornViewPage, ProductSurfaceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_obj_safe!(RebornViewProvider);

    /// Echoes all three conduit arguments back into the page so the test can
    /// assert each one arrived. A provider that silently dropped the caller
    /// could not scope its read, and one that dropped the params could not
    /// answer the question that was asked — both are invisible to a double
    /// that only echoes the cursor.
    struct EchoingView;

    #[async_trait]
    impl RebornViewProvider for EchoingView {
        fn descriptor(&self) -> RebornViewDescriptor {
            RebornViewDescriptor {
                id: "test_view",
                paginated: false,
            }
        }

        async fn query(
            &self,
            caller: ProductSurfaceCaller,
            params: serde_json::Value,
            cursor: Option<String>,
        ) -> Result<RebornViewPage, ProductSurfaceError> {
            Ok(RebornViewPage {
                payload: serde_json::json!({
                    "tenant": caller.tenant_id.as_str(),
                    "user": caller.user_id.as_str(),
                    "params": params,
                    "echoed_cursor": cursor,
                }),
                next_cursor: None,
            })
        }
    }

    #[test]
    fn a_provider_declares_its_own_id_and_pagination_shape() {
        let provider: std::sync::Arc<dyn RebornViewProvider> = std::sync::Arc::new(EchoingView);
        let descriptor = provider.descriptor();
        assert_eq!(descriptor.id, "test_view");
        assert!(!descriptor.paginated);
    }

    #[test]
    fn an_unpaginated_page_omits_next_cursor_on_the_wire() {
        // `next_cursor: None` must not serialize: the browser treats a present
        // cursor as "there is more", so emitting `null` would make every
        // unpaginated view look paginated.
        let page = RebornViewPage {
            payload: serde_json::json!({"rows": []}),
            next_cursor: None,
        };
        assert_eq!(
            serde_json::to_value(&page).expect("serialize"),
            serde_json::json!({"payload": {"rows": []}})
        );
    }

    #[tokio::test]
    async fn the_conduit_hands_the_provider_its_caller_params_and_cursor() {
        // The three arguments are the whole conduit: the caller is the
        // authorization subject a provider scopes its read by, the params are
        // the view's typed request, and the cursor is the pagination position.
        // Asserting all three is what makes this test fail if a future
        // signature change drops one on the floor.
        let provider: std::sync::Arc<dyn RebornViewProvider> = std::sync::Arc::new(EchoingView);
        let caller = ProductSurfaceCaller::new(
            ironclaw_host_api::ids::TenantId::new("tenant-alpha").expect("tenant"),
            ironclaw_host_api::ids::UserId::new("user-alpha").expect("user"),
            None,
            None,
        );
        let page = provider
            .query(
                caller,
                serde_json::json!({"limit": 10}),
                Some("c1".to_string()),
            )
            .await
            .expect("provider answers");
        assert_eq!(
            page.payload,
            serde_json::json!({
                "tenant": "tenant-alpha",
                "user": "user-alpha",
                "params": {"limit": 10},
                "echoed_cursor": "c1",
            })
        );
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn a_view_query_round_trips_its_params_and_optional_cursor() {
        let query = RebornViewQuery {
            view_id: "test_view".to_string(),
            params: serde_json::json!({"limit": 10}),
            cursor: Some("c1".to_string()),
        };
        let encoded = serde_json::to_value(&query).expect("serialize");
        let decoded: RebornViewQuery = serde_json::from_value(encoded).expect("round trip");
        assert_eq!(decoded, query);
    }
}
