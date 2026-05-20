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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::{
        RebornBuildInput, RebornReadinessState, RebornRuntimeIdentity, RebornRuntimeInput,
        TurnRunnerSettings, build_reborn_runtime,
    };

    use super::build_webui_services;

    #[tokio::test]
    async fn webui_bundle_reuses_runtime_thread_and_turn_facades() {
        let root = tempfile::tempdir().unwrap();
        let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
            "runtime-webui-owner",
            root.path().join("local-dev"),
        ))
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-tenant".to_string(),
            agent_id: "runtime-webui-agent".to_string(),
            source_binding_id: "runtime-webui-source".to_string(),
            reply_target_binding_id: "runtime-webui-reply".to_string(),
        })
        .with_runner_settings(TurnRunnerSettings {
            heartbeat_interval: Duration::from_secs(60),
            poll_interval: Duration::from_secs(60),
        });

        let runtime = build_reborn_runtime(input).await.unwrap();
        let runtime_turn_coordinator = runtime.webui_turn_coordinator();
        let bundle = build_webui_services(&runtime, None).unwrap();

        let _api = bundle.api.clone();
        assert!(Arc::ptr_eq(
            &runtime_turn_coordinator,
            &runtime.webui_turn_coordinator()
        ));
        assert_eq!(bundle.readiness, runtime.services().readiness);
        assert_eq!(bundle.readiness.state, RebornReadinessState::DevOnly);

        runtime.shutdown().await.unwrap();
    }
}
