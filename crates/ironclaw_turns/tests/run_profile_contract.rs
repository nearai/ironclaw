use ironclaw_turns::{
    AgentLoopDriverDescriptor, CapabilitySurfaceProfileId, InMemoryRunProfileResolver,
    ModelProfileId, PrivilegedRunProfileDimension, RunProfileRequest, RunProfileRequestAuthority,
    RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver, RunProfileVersion,
};

#[tokio::test]
async fn default_interactive_profile_resolves_stable_driver_and_redacted_snapshot() {
    let resolver = InMemoryRunProfileResolver::default();

    let snapshot = resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();

    assert_eq!(snapshot.profile_id.as_str(), "interactive_default");
    assert_eq!(snapshot.profile_version, RunProfileVersion::new(1));
    assert_eq!(snapshot.run_class_id.as_str(), "interactive_coding");
    assert_eq!(snapshot.loop_driver.id.as_str(), "lightweight_loop");
    assert_eq!(snapshot.loop_driver.version, RunProfileVersion::new(1));
    assert_eq!(snapshot.model_profile_id.as_str(), "interactive_model");
    assert_eq!(
        snapshot.capability_surface_profile_id.as_str(),
        "interactive_tools"
    );
    assert_eq!(snapshot.context_profile_id.as_str(), "interactive_context");
    assert!(snapshot.steering_policy.allow_steering);
    assert!(!snapshot.steering_policy.allow_driver_specific_nudges);
    assert_eq!(snapshot.provenance.sources.len(), 1);

    let wire = serde_json::to_string(&snapshot).unwrap();
    assert!(!wire.contains("secret"));
    assert!(!wire.contains("api_key"));
    assert!(!wire.contains("raw_config"));
    assert!(!wire.contains("RuntimeDispatcher"));
}

#[tokio::test]
async fn unauthorized_long_running_profile_request_rejects_before_fallback() {
    let resolver = InMemoryRunProfileResolver::default();
    let request = RunProfileResolutionRequest::interactive_default()
        .with_requested_run_profile(RunProfileRequest::new("long_running_mission").unwrap())
        .with_authority(RunProfileRequestAuthority::User);

    let error = resolver.resolve_run_profile(request).await.unwrap_err();

    assert_eq!(
        error,
        RunProfileResolutionError::Unauthorized {
            dimension: PrivilegedRunProfileDimension::LongRunningMission,
        }
    );
}

#[tokio::test]
async fn authorized_long_running_profile_resolves_distinct_driver_and_budget_envelope() {
    let resolver = InMemoryRunProfileResolver::default();
    let request = RunProfileResolutionRequest::interactive_default()
        .with_requested_run_profile(RunProfileRequest::new("long_running_mission").unwrap())
        .with_authority(RunProfileRequestAuthority::ProductSurface);

    let snapshot = resolver.resolve_run_profile(request).await.unwrap();

    assert_eq!(snapshot.profile_id.as_str(), "long_running_mission");
    assert_eq!(snapshot.run_class_id.as_str(), "long_running_mission");
    assert_eq!(snapshot.loop_driver.id.as_str(), "codeact_loop");
    assert_eq!(snapshot.checkpoint_schema_id.as_str(), "durable_mission_v1");
    assert_eq!(snapshot.model_profile_id.as_str(), "mission_model");
    assert_eq!(
        snapshot.resource_budget_policy.tier.as_str(),
        "mission_standard"
    );
    assert_eq!(snapshot.resource_budget_policy.max_model_calls, 128);
    assert_eq!(
        snapshot.resource_budget_policy.max_capability_invocations,
        512
    );
    assert_eq!(snapshot.scheduling_class.as_str(), "background");
    assert_eq!(snapshot.concurrency_class.as_str(), "mission_serial");
    assert_eq!(
        snapshot.provenance.effective_privileges,
        vec![
            PrivilegedRunProfileDimension::LongRunningMission,
            PrivilegedRunProfileDimension::SpecialDriver,
            PrivilegedRunProfileDimension::RunnerPool,
        ]
    );
    assert!(
        snapshot
            .provenance
            .sources
            .iter()
            .any(|source| source.summary
                == "resource budget clamped to mission_standard by policy ceiling")
    );
}

#[tokio::test]
async fn resolution_is_deterministic_and_records_clamped_provenance() {
    let resolver = InMemoryRunProfileResolver::default();
    let request = RunProfileResolutionRequest::interactive_default()
        .with_requested_run_profile(RunProfileRequest::new("long_running_mission").unwrap())
        .with_authority(RunProfileRequestAuthority::ProductSurface);

    let first = resolver.resolve_run_profile(request.clone()).await.unwrap();
    let second = resolver.resolve_run_profile(request).await.unwrap();

    assert_eq!(first.resolution_fingerprint, second.resolution_fingerprint);
    assert_eq!(first.provenance, second.provenance);
    assert!(
        first
            .provenance
            .sources
            .iter()
            .any(|source| source.summary == "requested profile accepted within policy ceiling")
    );

    let unclamped = resolver
        .resolve_run_profile(
            RunProfileResolutionRequest::interactive_default()
                .with_requested_run_profile(RunProfileRequest::new("long_running_mission").unwrap())
                .with_authority(RunProfileRequestAuthority::Admin),
        )
        .await
        .unwrap();
    assert_ne!(
        first.resolution_fingerprint,
        unclamped.resolution_fingerprint
    );
    assert_eq!(
        unclamped.resource_budget_policy.tier.as_str(),
        "mission_high"
    );
}

#[test]
fn profile_ref_types_validate_when_deserializing_resolved_snapshots() {
    assert!(serde_json::from_value::<ModelProfileId>(serde_json::json!("valid_model")).is_ok());
    assert!(serde_json::from_value::<ModelProfileId>(serde_json::json!("BAD MODEL")).is_err());
    assert!(
        serde_json::from_value::<CapabilitySurfaceProfileId>(serde_json::json!(
            "raw\u{0000}surface"
        ))
        .is_err()
    );
}

#[test]
fn host_authority_is_not_part_of_the_wire_request_shape() {
    let request = RunProfileResolutionRequest::interactive_default()
        .with_requested_run_profile(RunProfileRequest::new("long_running_mission").unwrap())
        .with_authority(RunProfileRequestAuthority::ProductSurface);

    let wire = serde_json::to_string(&request).unwrap();

    assert_eq!(wire, r#"{"requested_run_profile":"long_running_mission"}"#);
    assert!(!wire.contains("ProductSurface"));
    assert!(!wire.contains("admin"));
    assert!(!wire.contains("system"));
}

#[test]
fn agent_loop_driver_descriptor_wire_shape_excludes_raw_authority_handles() {
    let descriptor = AgentLoopDriverDescriptor::new("lightweight_loop", RunProfileVersion::new(1))
        .unwrap()
        .with_checkpoint_schema("interactive_checkpoint_v1", RunProfileVersion::new(1))
        .unwrap();

    let wire = serde_json::to_value(&descriptor).unwrap();

    assert_eq!(wire["id"], "lightweight_loop");
    assert_eq!(wire["version"], 1);
    assert!(wire.get("runtime_dispatcher").is_none());
    assert!(wire.get("process_host").is_none());
    assert!(wire.get("raw_provider_client").is_none());
    assert!(wire.get("secrets").is_none());
}
