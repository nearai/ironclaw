//! Host-supplied route-mount vocabulary shared between composition's own
//! route builders (nearai login, OpenAI-compat) and the host-owned WebChat v2
//! gateway assembly in `ironclaw_webui`.
//!
//! These types carry only `axum::Router` + `IngressRouteDescriptor`; they do
//! not depend on the WebChat v2 route surface, so they stay in composition
//! (where nearai/openai/runtime construct them) while `webui_v2_app` and its
//! middleware live one layer up in the ingress crate.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use ironclaw_host_api::ingress::IngressRouteDescriptor;

/// Async drain hook for public route mounts that schedule work outside the
/// request/response future.
pub trait PublicRouteDrain: Send + Sync {
    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// A host-supplied public sub-router plus the descriptors composition
/// needs to install the per-route policy middleware around it.
/// Mirrors the shape `ProductAuthRouteMount` uses internally so the
/// two public surfaces ride on the same machinery.
#[derive(Clone)]
pub struct PublicRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
    pub drain: Option<Arc<dyn PublicRouteDrain>>,
}

/// A host-supplied protected sub-router plus the descriptors composition
/// needs to install the shared per-route policy middleware around it.
#[derive(Clone)]
pub struct ProtectedRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
}

impl ProtectedRouteMount {
    pub fn new(router: Router, descriptors: Vec<IngressRouteDescriptor>) -> Self {
        Self {
            router,
            descriptors,
        }
    }
}

impl PublicRouteMount {
    pub fn new(router: Router, descriptors: Vec<IngressRouteDescriptor>) -> Self {
        Self {
            router,
            descriptors,
            drain: None,
        }
    }

    pub fn with_drain(mut self, drain: Arc<dyn PublicRouteDrain>) -> Self {
        self.drain = Some(drain);
        self
    }
}

#[derive(Clone, Default)]
pub struct PublicRouteDrains {
    drains: Arc<Vec<Arc<dyn PublicRouteDrain>>>,
}

impl PublicRouteDrains {
    pub fn new(drains: Vec<Arc<dyn PublicRouteDrain>>) -> Self {
        Self {
            drains: Arc::new(drains),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.drains.is_empty()
    }

    pub async fn drain(&self) {
        for drain in self.drains.iter() {
            drain.drain().await;
        }
    }
}
