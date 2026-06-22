#![cfg(all(feature = "github-issue-workflow-beta", feature = "test-support"))]

mod github_issue_workflow_runtime {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use ironclaw_host_api::runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    };
    use ironclaw_host_api::{
        CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
        ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy, Principal,
        ResourceEstimate, RuntimeKind, TrustClass,
    };
    use ironclaw_host_runtime::{
        CapabilitySurfacePolicy, HostRuntime, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
        RuntimeCapabilityRequest, RuntimeFailureKind, SurfaceKind, VisibleCapabilityRequest,
        WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
    };
    use ironclaw_reborn_composition::{
        GithubIssueWorkflowSettings, RebornBuildInput, RebornRuntimeError, RebornRuntimeIdentity,
        RebornRuntimeInput, build_reborn_runtime,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::{Value, json};

    #[tokio::test]
    async fn runtime_disables_github_issue_workflow_by_default() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(local_dev_input(root.path().join("local-dev")))
            .await
            .expect("runtime builds");

        assert!(runtime.services().readiness.workers.turn_runner);
        assert!(!runtime.services().readiness.workers.github_issue_workflow);

        runtime.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn runtime_starts_github_issue_workflow_when_enabled() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            local_dev_input(root.path().join("local-dev")).with_github_issue_workflow_settings(
                GithubIssueWorkflowSettings::enabled_for_tests(),
            ),
        )
        .await
        .expect("runtime builds");

        assert!(runtime.services().readiness.workers.github_issue_workflow);

        runtime.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn runtime_shutdown_cancels_github_issue_workflow_poller() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            local_dev_input(root.path().join("local-dev")).with_github_issue_workflow_settings(
                GithubIssueWorkflowSettings::enabled_for_tests(),
            ),
        )
        .await
        .expect("runtime builds");

        assert!(runtime.services().readiness.workers.github_issue_workflow);
        tokio::time::timeout(Duration::from_secs(2), runtime.shutdown())
            .await
            .expect("shutdown returns before timeout")
            .expect("shutdown");
    }

    #[tokio::test]
    async fn runtime_enabled_workflow_registers_result_sink_handler() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            local_dev_input(root.path().join("local-dev")).with_github_issue_workflow_settings(
                GithubIssueWorkflowSettings::enabled_for_tests(),
            ),
        )
        .await
        .expect("runtime builds");
        let host_runtime = runtime
            .services()
            .host_runtime
            .as_deref()
            .expect("host runtime");

        let failure = invoke_failure_with_context(
            host_runtime,
            WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
            json!({}),
            execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
        )
        .await;

        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
        runtime.shutdown().await.expect("shutdown");
    }

    #[cfg(not(feature = "libsql"))]
    #[tokio::test]
    async fn production_enabled_workflow_requires_durable_storage() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = match build_reborn_runtime(
            local_dev_input(root.path().join("local-dev"))
                .with_github_issue_workflow_settings(GithubIssueWorkflowSettings::enabled()),
        )
        .await
        {
            Ok(_) => {
                panic!("production-shaped workflow enablement must fail without durable storage")
            }
            Err(err) => err,
        };

        assert!(
            matches!(err, RebornRuntimeError::InvalidArgument { ref reason } if reason.contains("durable storage")),
            "unexpected error: {err:?}"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn production_enabled_workflow_requires_project_access_checker() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = match build_reborn_runtime(
            local_dev_input(root.path().join("local-dev"))
                .with_github_issue_workflow_settings(GithubIssueWorkflowSettings::enabled()),
        )
        .await
        {
            Ok(_) => panic!(
                "non-test workflow enablement must fail closed without project access checker"
            ),
            Err(err) => err,
        };

        assert!(
            matches!(err, RebornRuntimeError::InvalidArgument { ref reason } if reason.contains("project access checker")),
            "unexpected error: {err:?}"
        );
    }

    fn local_dev_input(root: std::path::PathBuf) -> RebornRuntimeInput {
        RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("workflow-runtime-owner", root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "workflow-runtime-tenant".to_string(),
            agent_id: "workflow-runtime-agent".to_string(),
            source_binding_id: "workflow-runtime-source".to_string(),
            reply_target_binding_id: "workflow-runtime-reply".to_string(),
        })
    }

    async fn invoke_failure_with_context<R: HostRuntime + ?Sized>(
        runtime: &R,
        capability: &str,
        input: Value,
        context: ExecutionContext,
    ) -> RuntimeCapabilityFailure {
        let outcome = runtime
            .invoke_capability(RuntimeCapabilityRequest::new(
                context,
                CapabilityId::new(capability).expect("capability id"),
                ResourceEstimate::default(),
                input,
                trust_decision(),
            ))
            .await
            .expect("capability invocation returns outcome");
        match outcome {
            RuntimeCapabilityOutcome::Failed(failure) => failure,
            other => panic!("unexpected capability outcome: {other:?}"),
        }
    }

    fn execution_context<I>(grants: I) -> ExecutionContext
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let capability_set = CapabilitySet {
            grants: grants
                .into_iter()
                .map(|grant| dispatch_grant(grant.as_ref()))
                .collect(),
        };
        ExecutionContext::local_default(
            ironclaw_host_api::UserId::new("workflow-caller").expect("user"),
            ExtensionId::new("workflow-caller").expect("extension"),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            capability_set,
            MountView::default(),
        )
        .expect("execution context")
    }

    fn dispatch_grant(capability: &str) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(capability).expect("capability id"),
            grantee: Principal::Extension(ExtensionId::new("workflow-caller").expect("extension")),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::DispatchCapability],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    #[allow(dead_code)]
    fn visible_request() -> VisibleCapabilityRequest {
        VisibleCapabilityRequest::new(
            execution_context([WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID]),
            SurfaceKind::new("agent_loop").expect("surface kind"),
        )
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(BTreeMap::from([(
            ExtensionId::new("builtin").expect("builtin extension"),
            trust_decision(),
        )]))
    }

    fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }
}
