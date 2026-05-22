use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::RebornServicesErrorCode;
use ironclaw_product_workflow::{
    RebornServices as ProductRebornServices, RebornServicesApi, RebornServicesError,
};

use crate::{RebornBuildError, RebornReadiness, RebornRuntime};

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes only the product facade consumed by WebChat
/// v2 routes. HTTP routing, auth middleware, static assets, and SSE transport
/// stay in the WebUI crate; lower runtime handles stay behind the existing
/// Reborn runtime/composition services.
#[allow(dead_code)] // Private follow-up hook for WebUI route mounting.
#[derive(Clone)]
pub(crate) struct RebornWebuiBundle {
    pub(crate) api: Arc<dyn RebornServicesApi>,
    pub(crate) readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornWebuiBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiBundle")
            .field("api", &"Arc<dyn RebornServicesApi>")
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// Compose the WebUI-facing product facade from an already-built Reborn runtime.
///
/// This function does not create a second turn coordinator, thread service,
/// host runtime, route server, or event stream. It reuses the runtime's existing
/// task-level composition and accepts an optional projection stream owned by the
/// caller's event-stream composition layer.
#[allow(dead_code)] // Private follow-up hook for WebUI route mounting.
pub(crate) fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let services = runtime.services();

    let mut api = ProductRebornServices::new(
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    );
    if let Some(skill_activation_source) = runtime.webui_skill_activation_source() {
        api = api.with_skill_activation_recorder(move |scope, run_id, message| {
            skill_activation_source
                .record_user_message(scope.clone(), run_id, message)
                .map_err(|_| RebornServicesError {
                    code: RebornServicesErrorCode::Internal,
                    status_code: 500,
                    retryable: false,
                    field: None,
                    validation_code: None,
                })
        });
    }
    if let Some(event_stream) = event_stream {
        api = api.with_event_stream(event_stream);
    }

    Ok(RebornWebuiBundle {
        api: Arc::new(api),
        readiness: services.readiness,
    })
}
