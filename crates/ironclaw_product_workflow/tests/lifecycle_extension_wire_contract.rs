use ironclaw_host_api::{CapabilitySurfaceKind, RuntimeKind};
use ironclaw_product_workflow::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
};

#[test]
fn lifecycle_extension_summary_uses_canonical_runtime_wire() {
    for (runtime, expected) in [
        (RuntimeKind::Wasm, "wasm"),
        (RuntimeKind::FirstParty, "first_party"),
    ] {
        let summary = LifecycleExtensionSummary {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .expect("valid package ref"),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            description: "fixture extension".to_string(),
            source: LifecycleExtensionSource::HostBundled,
            runtime,
            surface_kinds: vec![CapabilitySurfaceKind::Tool],
            channel_directions: None,
            channel_connection: None,
            visible_capability_ids: vec!["fixture.run".to_string()],
            visible_read_only_capability_ids: Vec::new(),
            credential_requirements: Vec::new(),
            onboarding: None,
        };

        let wire = serde_json::to_value(summary).expect("summary serializes");
        assert_eq!(wire.get("runtime"), Some(&serde_json::json!(expected)));
        assert!(wire.get("runtime_kind").is_none());
    }
}
