use std::sync::Arc;

use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{RebornServices as ProductRebornServices, RebornServicesApi};

use crate::{RebornBuildError, RebornReadiness, RebornRuntime};

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes only the product facade consumed by
/// WebChat v2 routes. HTTP routing, auth middleware, static assets, and
/// SSE transport stay in the WebUI crate (or, when the `webui-v2-beta`
/// feature is on, the [`crate::webui_serve`] module in this crate);
/// lower runtime handles stay behind the existing Reborn runtime /
/// composition services.
#[derive(Clone)]
pub struct RebornWebuiBundle {
    pub api: Arc<dyn RebornServicesApi>,
    pub readiness: RebornReadiness,
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
pub fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let services = runtime.services();

    let mut api = ProductRebornServices::new(
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    );
    if let Some(event_stream) = event_stream {
        api = api.with_event_stream(event_stream);
    }

    Ok(RebornWebuiBundle {
        api: Arc::new(api),
        readiness: services.readiness,
    })
}
