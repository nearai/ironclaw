//! Host-owned HTTP route mount vocabulary.
//!
//! These types carry prebuilt Axum routers plus their ingress descriptors. They
//! are intentionally neutral: runtime/composition code may build route mounts,
//! while host ingress code decides how to layer authentication, rate limits,
//! body limits, and static routing around them.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use ironclaw_host_api::IngressRouteDescriptor;

/// Async drain hook for public route mounts that schedule work outside the
/// request/response future.
pub trait PublicRouteDrain: Send + Sync {
    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// A host-supplied public sub-router plus the descriptors ingress uses to
/// install per-route policy middleware around it.
#[derive(Clone)]
pub struct PublicRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
    pub drain: Option<Arc<dyn PublicRouteDrain>>,
}

/// A host-supplied protected sub-router plus the descriptors ingress uses to
/// install shared bearer-auth and per-route policy middleware around it.
#[derive(Clone)]
pub struct ProtectedRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
}

/// A route mount with both protected and public sub-routers sharing one
/// descriptor inventory.
#[derive(Clone)]
pub struct SplitRouteMount {
    pub protected: Router,
    pub public: Router,
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

impl SplitRouteMount {
    pub fn new(
        protected: Router,
        public: Router,
        descriptors: Vec<IngressRouteDescriptor>,
    ) -> Self {
        Self {
            protected,
            public,
            descriptors,
        }
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
