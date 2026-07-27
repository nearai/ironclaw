use axum::Router;
use ironclaw_host_api::ingress::IngressRouteDescriptor;

/// Operator-owned public route mount payload.
///
/// Composition adapts this into its host-web route vocabulary when assembling
/// listeners; operator code owns the route implementation and descriptors.
#[derive(Clone)]
pub struct OperatorPublicRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
}

impl OperatorPublicRouteMount {
    pub fn new(router: Router, descriptors: Vec<IngressRouteDescriptor>) -> Self {
        Self {
            router,
            descriptors,
        }
    }
}

/// Operator-owned protected route mount payload.
#[derive(Clone)]
pub struct OperatorProtectedRouteMount {
    pub router: Router,
    pub descriptors: Vec<IngressRouteDescriptor>,
}

impl OperatorProtectedRouteMount {
    pub fn new(router: Router, descriptors: Vec<IngressRouteDescriptor>) -> Self {
        Self {
            router,
            descriptors,
        }
    }
}
