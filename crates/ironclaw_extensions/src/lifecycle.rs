use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{ExtensionId, ExtensionLifecycleOperation, RuntimeKind};

use crate::{ExtensionError, ExtensionPackage, ExtensionRegistry};

/// Redacted extension lifecycle event emitted by host-composed lifecycle services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLifecycleEvent {
    pub operation: ExtensionLifecycleOperation,
    pub extension_id: ExtensionId,
    pub version: String,
    pub runtime: RuntimeKind,
    pub capability_count: usize,
    pub capability_surface_changed: bool,
}

impl ExtensionLifecycleEvent {
    fn from_package(
        operation: ExtensionLifecycleOperation,
        package: &ExtensionPackage,
        capability_surface_changed: bool,
    ) -> Self {
        Self {
            operation,
            extension_id: package.id.clone(),
            version: package.manifest.version.clone(),
            runtime: package.manifest.runtime_kind(),
            capability_count: package.capabilities.len(),
            capability_surface_changed,
        }
    }
}

/// Host-composed sink for redacted extension lifecycle events.
#[async_trait]
pub trait ExtensionLifecycleEventSink: Send + Sync {
    async fn record_extension_lifecycle_event(
        &self,
        event: ExtensionLifecycleEvent,
    ) -> Result<(), ExtensionError>;
}

/// Host-facing lifecycle wrapper over the deterministic extension registry.
pub struct ExtensionLifecycleService {
    registry: ExtensionRegistry,
    event_sink: Option<Arc<dyn ExtensionLifecycleEventSink>>,
    disabled_extensions: HashSet<ExtensionId>,
}

impl std::fmt::Debug for ExtensionLifecycleService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtensionLifecycleService")
            .field("registry", &self.registry)
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<event_sink>"),
            )
            .field("disabled_extensions", &self.disabled_extensions)
            .finish()
    }
}

impl ExtensionLifecycleService {
    pub fn new(registry: ExtensionRegistry) -> Self {
        Self {
            registry,
            event_sink: None,
            disabled_extensions: HashSet::new(),
        }
    }

    pub fn with_event_sink<S>(mut self, event_sink: Arc<S>) -> Self
    where
        S: ExtensionLifecycleEventSink + 'static,
    {
        let event_sink: Arc<dyn ExtensionLifecycleEventSink> = event_sink;
        self.event_sink = Some(event_sink);
        self
    }

    pub fn registry(&self) -> &ExtensionRegistry {
        &self.registry
    }

    pub fn is_enabled(&self, id: &ExtensionId) -> bool {
        self.registry.get_extension(id).is_some() && !self.disabled_extensions.contains(id)
    }

    pub async fn install(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.registry.validate_insertable(&package)?;
        self.emit_lifecycle_event(ExtensionLifecycleEvent::from_package(
            ExtensionLifecycleOperation::Install,
            &package,
            true,
        ))
        .await?;
        self.registry.insert_validated(package);
        Ok(())
    }

    pub async fn update(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        let current = self.registry.existing_package(&package.id)?.clone();
        self.registry.validate_replacement(&package)?;
        let capability_surface_changed = current.capabilities != package.capabilities;
        self.emit_lifecycle_event(ExtensionLifecycleEvent::from_package(
            ExtensionLifecycleOperation::Update,
            &package,
            capability_surface_changed,
        ))
        .await?;
        self.registry.replace_validated(package);
        Ok(())
    }

    pub async fn remove(&mut self, id: &ExtensionId) -> Result<(), ExtensionError> {
        let package = self.registry.existing_package(id)?.clone();
        self.emit_lifecycle_event(ExtensionLifecycleEvent::from_package(
            ExtensionLifecycleOperation::Remove,
            &package,
            !package.capabilities.is_empty(),
        ))
        .await?;
        self.registry.remove(id);
        self.disabled_extensions.remove(id);
        Ok(())
    }

    pub async fn enable(&mut self, id: &ExtensionId) -> Result<(), ExtensionError> {
        let package = self.registry.existing_package(id)?.clone();
        let capability_surface_changed = self.disabled_extensions.contains(id);
        self.emit_lifecycle_event(ExtensionLifecycleEvent::from_package(
            ExtensionLifecycleOperation::Enable,
            &package,
            capability_surface_changed,
        ))
        .await?;
        self.disabled_extensions.remove(id);
        Ok(())
    }

    pub async fn disable(&mut self, id: &ExtensionId) -> Result<(), ExtensionError> {
        let package = self.registry.existing_package(id)?.clone();
        let capability_surface_changed = !self.disabled_extensions.contains(id);
        self.emit_lifecycle_event(ExtensionLifecycleEvent::from_package(
            ExtensionLifecycleOperation::Disable,
            &package,
            capability_surface_changed,
        ))
        .await?;
        self.disabled_extensions.insert(id.clone());
        Ok(())
    }

    async fn emit_lifecycle_event(
        &self,
        event: ExtensionLifecycleEvent,
    ) -> Result<(), ExtensionError> {
        if let Some(event_sink) = &self.event_sink {
            let extension_id = event.extension_id.clone();
            let operation = event.operation;
            event_sink
                .record_extension_lifecycle_event(event)
                .await
                .map_err(|_| ExtensionError::LifecycleEventSink {
                    extension_id,
                    operation,
                })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::Mutex;

    use ironclaw_host_api::{HostPortCatalog, VirtualPath};

    use super::*;
    use crate::{ExtensionManifest, ManifestSource};

    #[tokio::test]
    async fn enable_and_disable_events_report_surface_change_only_on_state_transition() {
        let sink = Arc::new(RecordingSink::default());
        let mut service = ExtensionLifecycleService::new(ExtensionRegistry::new())
            .with_event_sink(Arc::clone(&sink));
        let package = test_package("fixture");
        let extension_id = package.id.clone();
        service.install(package).await.expect("install");

        service.disable(&extension_id).await.expect("first disable");
        service
            .disable(&extension_id)
            .await
            .expect("second disable");
        service.enable(&extension_id).await.expect("first enable");
        service.enable(&extension_id).await.expect("second enable");

        let events = sink.events.lock().await;
        let surface_changes = events
            .iter()
            .filter(|event| {
                matches!(
                    event.operation,
                    ExtensionLifecycleOperation::Disable | ExtensionLifecycleOperation::Enable
                )
            })
            .map(|event| event.capability_surface_changed)
            .collect::<Vec<_>>();
        assert_eq!(surface_changes, vec![true, false, true, false]);
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<ExtensionLifecycleEvent>>,
    }

    #[async_trait]
    impl ExtensionLifecycleEventSink for RecordingSink {
        async fn record_extension_lifecycle_event(
            &self,
            event: ExtensionLifecycleEvent,
        ) -> Result<(), ExtensionError> {
            self.events.lock().await.push(event);
            Ok(())
        }
    }

    fn test_package(extension_id: &str) -> ExtensionPackage {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id}"
name = "{extension_id}"
version = "0.1.0"
description = "test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"

[[capabilities]]
id = "{extension_id}.read"
description = "read"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/read.input.json"
output_schema_ref = "schemas/read.output.json"
"#
        );
        let manifest = ExtensionManifest::parse(
            &manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("manifest parses");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{extension_id}")).expect("root"),
        )
        .expect("package builds")
    }
}
