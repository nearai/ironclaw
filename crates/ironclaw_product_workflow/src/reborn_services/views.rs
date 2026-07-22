//! Generic read conduit for descriptor-declared product views.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{RebornServicesError, WebUiAuthenticatedCaller};

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
/// `RebornServicesApi` with feature-specific read methods.
#[async_trait]
pub trait RebornViewProvider: Send + Sync {
    fn descriptor(&self) -> RebornViewDescriptor;

    async fn query(
        &self,
        caller: WebUiAuthenticatedCaller,
        params: serde_json::Value,
        cursor: Option<String>,
    ) -> Result<RebornViewPage, RebornServicesError>;
}

/// Fail-closed static default for compositions without an additional view.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableRebornViewProvider;

#[async_trait]
impl RebornViewProvider for UnavailableRebornViewProvider {
    fn descriptor(&self) -> RebornViewDescriptor {
        RebornViewDescriptor {
            id: "__unavailable_product_view",
            paginated: false,
        }
    }

    async fn query(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _params: serde_json::Value,
        _cursor: Option<String>,
    ) -> Result<RebornViewPage, RebornServicesError> {
        Err(RebornServicesError::service_unavailable(false))
    }
}
