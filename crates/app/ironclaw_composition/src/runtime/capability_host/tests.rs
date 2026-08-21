//! Capability-host assembly tests.

#[cfg(test)]
mod tests {
    #![allow(clippy::module_inception)]

    mod display_preview;

    use super::super::*;

    use ironclaw_assistant::{
        LifecyclePackageKind, LifecyclePackageRef, OutboundPreferencesProductService,
        RebornOutboundDeliveryTargetId,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::turn::{
        AcceptedMessageRef, ReplyTargetBindingRef, TurnActor, TurnId, TurnRunId, TurnScope,
    };
    use ironclaw_host_api::{
        action::NetworkPolicy,
        artifact::{
            AccountedArtifactPersister, ArtifactAccessPort, ArtifactOwnerScope,
            ArtifactReadRequest, ArtifactReadTarget, ArtifactRef, ArtifactSelector,
            ArtifactWriteMetadata,
        },
        capability::{EXTENSION_SEARCH_CAPABILITY_ID, EffectKind},
        ids::{
            AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName,
            ResourceReservationId, TenantId, ThreadId, UserId,
        },
        mount::{MountPermissions, MountView},
        resolution::Resolution,
        resource::{ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceUsage},
        result_meta::FailureKind,
    };
    use ironclaw_host_runtime::{
        CODING_BASH_CAPABILITY_ID, CODING_EDIT_CAPABILITY_ID, CODING_READ_CAPABILITY_ID,
        CODING_WRITE_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID,
        HTTP_SAVE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID, OUTBOUND_DELIVER_CAPABILITY_ID,
        SHELL_CAPABILITY_ID, SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID,
        SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID, SKILL_UPDATE_CAPABILITY_ID,
        SPAWN_SUBAGENT_CAPABILITY_ID,
    };
    use ironclaw_loop_contracts::{
        CapabilityCallCandidate, CapabilityInputRef, InMemoryLoopHostMilestoneSink,
        InMemoryRunProfileResolver, LoopRequest, RegisterProviderToolCallRequest,
        RunProfileResolutionRequest, RunProfileResolver, VisibleCapabilityRequest,
    };
    use ironclaw_loop_host::{
        CapabilityWriteResult, DurablePersistence, HostManagedModelError,
        HostManagedModelErrorKind, HostManagedModelRequest, HostManagedModelResponse,
        HostSkillContextSource,
    };
    use ironclaw_outbound::{
        CommunicationPreferenceKey, DeliveryTargetCapabilities, OutboundDeliveryTargetId,
        OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary, OutboundError,
    };
    use ironclaw_threads::{
        AppendToolResultReferenceRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageKind, SessionThreadService, ThreadHistoryRequest, ThreadScope,
        ToolResultSafeSummary,
    };

    use crate::outbound::{
        OutboundDeliveryTargetEntry, OutboundDeliveryTargetOwner, OutboundDeliveryTargetProvider,
        OutboundDeliveryTargetRegistry,
    };
    use crate::runtime::filesystem_skill_context_source;
    use ironclaw_assistant::RebornOutboundPreferencesService;
    use ironclaw_extension_manager::extension_lifecycle_capabilities::{
        EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID,
        EXTENSION_REMOVE_CAPABILITY_ID,
    };

    #[derive(Default)]
    struct RecordingToolDiagnosticSink {
        result: std::sync::Mutex<Option<HostManagedToolResultDiagnosticCapture>>,
    }

    impl HostManagedPromptDiagnosticSink for RecordingToolDiagnosticSink {
        fn record_prompt(&self, _capture: ironclaw_loop_host::HostManagedPromptDiagnosticCapture) {}

        fn record_tool_result(&self, capture: HostManagedToolResultDiagnosticCapture) {
            *self.result.lock().expect("diagnostic result lock") = Some(capture);
        }
    }

    /// The §5.3 flip collapsed `CapabilityOutcome::Completed` into
    /// `Resolution::Done(Outcome)`; the minted `refs.result` is an opaque uuid,
    /// while the originating loop result ref the capability io staged the output
    /// under is preserved on `refs.origin`. Tests look results up by that
    /// preserved loop ref, exactly as they did with the old
    /// `CapabilityResultMessage::result_ref`.
    fn completed_loop_result_ref(done: &ironclaw_host_api::resolution::Outcome) -> String {
        done.refs
            .origin
            .as_ref()
            .expect("completed capability outcome preserves the originating loop result ref")
            .as_str()
            .to_string()
    }

    async fn run_context(label: &str) -> LoopRunContext {
        run_context_with_scope(TurnScope::new(
            TenantId::new(format!("tenant-{label}")).expect("tenant id"),
            Some(AgentId::new(format!("agent-{label}")).expect("agent id")),
            Some(ProjectId::new(format!("project-{label}")).expect("project id")),
            ThreadId::new(format!("thread-{label}")).expect("thread id"),
        ))
        .await
    }

    async fn run_context_with_scope(scope: TurnScope) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    async fn ensure_thread_for_run(
        thread_service: &dyn SessionThreadService,
        run_context: &LoopRunContext,
        fallback_user_id: &UserId,
    ) {
        let scope =
            thread_scope_for_run(run_context, fallback_user_id).expect("run scope has an agent");
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope,
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "test-actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
    }

    /// Turn on the global auto-approve switch for the `(tenant, user)` a run
    /// dispatches under so a scripted tool call exercises the dispatch path
    /// instead of stopping at the per-tool approval gate. The Tools-settings
    /// switch is authoritative for first-party tool dispatch; enabling
    /// it here mirrors the operator having flipped it on before letting the
    /// agent run tools.
    async fn enable_global_auto_approve_for_run(
        services: &crate::factory::RebornRuntimeStores,
        run_context: &LoopRunContext,
        user_id: &UserId,
    ) {
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let mut scope = run_context.scope.to_resource_scope();
        scope.user_id = user_id.clone();
        ironclaw_approvals::AutoApproveSettingStorePort::set(
            runtime_surfaces.auto_approve_settings_for_test().as_ref(),
            ironclaw_approvals::AutoApproveSettingInput {
                updated_by: ironclaw_host_api::scope::Principal::User(user_id.clone()),
                scope,
                enabled: true,
            },
        )
        .await
        .expect("enabling global auto-approve should succeed");
    }

    fn local_host_minimal_approval_policy()
    -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
        let mut policy = crate::standalone_runtime_policy().expect("standalone policy resolves");
        policy.requested_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
        policy.resolved_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
        policy.approval_policy = ironclaw_host_api::runtime_policy::ApprovalPolicy::Minimal;
        policy
    }

    #[derive(Debug)]
    struct UnusedSandboxTransport;

    #[async_trait::async_trait]
    impl ironclaw_host_api::process::SandboxCommandTransport for UnusedSandboxTransport {
        async fn run_command(
            &self,
            _request: ironclaw_host_api::process::CommandExecutionRequest,
        ) -> Result<
            ironclaw_host_api::process::CommandExecutionOutput,
            ironclaw_host_api::process::RuntimeProcessError,
        > {
            panic!("filesystem-only extension lifecycle calls must not start a sandbox process")
        }
    }

    /// A multi-user WebChat run carries an actor but NO explicit thread owner
    /// (`ActorFallback`): its runtime scope — grants, mounts, gate dance — must
    /// follow that actor (the authenticated caller), never the host fallback.
    /// The actor-first rung of `LoopRunContext::acting_user_id` is what keeps a
    /// caller's grants scoped to the caller and not the operator; this is
    /// legitimate run-user resolution, not owner-vs-actor divergence.
    #[tokio::test]
    async fn visible_capability_request_uses_run_actor_for_runtime_scope() {
        let run_context = run_context("actor-runtime-scope")
            .await
            .with_actor(TurnActor::new(
                UserId::new("sso-user").expect("actor user id"),
            ));
        let fallback_user_id = UserId::new("env-operator").expect("fallback user id");
        let request = visible_request_for_runtime_scope(&run_context, &fallback_user_id);

        assert_eq!(request.context.user_id.as_str(), "sso-user");
        assert_eq!(request.context.resource_scope.user_id.as_str(), "sso-user");
    }

    // Note: `visible_capability_request_uses_acting_user_for_runtime_scope`
    // retired with the ephemeral-per-ping remodel (#7377). Its whole point was
    // that the runtime scope followed the ACTOR over a DIFFERENT explicit
    // thread owner (owner ≠ actor); that divergence can no longer occur. The
    // legitimate actor-derived case is covered by
    // `visible_capability_request_uses_run_actor_for_runtime_scope` above.

    #[tokio::test]
    async fn visible_capability_request_keeps_fallback_user_without_actor() {
        let run_context = run_context("fallback-runtime-scope").await;
        let fallback_user_id = UserId::new("env-operator").expect("fallback user id");
        let request = visible_request_for_runtime_scope(&run_context, &fallback_user_id);

        assert_eq!(request.context.user_id.as_str(), "env-operator");
        assert_eq!(
            request.context.resource_scope.user_id.as_str(),
            "env-operator"
        );
    }

    /// `thread_scope_for_run` resolves the run's user for durable thread I/O:
    /// an explicit-owner run (host/trigger creator) uses its explicit owner, a
    /// multi-user WebChat run (actor, no explicit owner) uses its actor, and an
    /// ownerless run falls back to the host owner. Owner == actor since the
    /// ephemeral-per-ping remodel, so the explicit-owner and actor paths yield
    /// the same user for a normal run — they differ only for triggers (explicit
    /// creator, no actor) and system runs (fallback).
    #[tokio::test]
    async fn standalone_durable_thread_scope_resolves_the_runs_user() {
        let fallback_user_id = UserId::new("durable-fallback-owner").expect("fallback user id");

        // Explicit-owner run (host/trigger creator, no TurnActor): the thread
        // uses the explicit owner.
        let explicit_owner = UserId::new("durable-explicit-owner").expect("explicit owner");
        let explicit_context = run_context_with_scope(TurnScope::new_with_owner(
            TenantId::new("tenant-durable-scope").expect("tenant id"),
            Some(AgentId::new("agent-durable-scope").expect("agent id")),
            Some(ProjectId::new("project-durable-scope").expect("project id")),
            ThreadId::new("thread-durable-scope").expect("thread id"),
            Some(explicit_owner.clone()),
        ))
        .await;
        let scope = thread_scope_for_run(&explicit_context, &fallback_user_id)
            .expect("agent-scoped run produces a thread scope");
        assert_eq!(scope.owner_user_id, Some(explicit_owner));

        // Multi-user WebChat run (actor, no explicit owner): the thread uses
        // the actor.
        let actor_owner = UserId::new("durable-run-actor").expect("actor user id");
        let actor_context = run_context("durable-actor-scope")
            .await
            .with_actor(TurnActor::new(actor_owner.clone()));
        let actor_scope = thread_scope_for_run(&actor_context, &fallback_user_id)
            .expect("agent-scoped run produces a thread scope");
        assert_eq!(actor_scope.owner_user_id, Some(actor_owner));

        // No actor, no explicit owner: fall back to the host owner.
        let fallback_context = run_context("durable-fallback-scope").await;
        let fallback_scope = thread_scope_for_run(&fallback_context, &fallback_user_id)
            .expect("agent-scoped run produces a thread scope");
        assert_eq!(fallback_scope.owner_user_id, Some(fallback_user_id));
    }

    fn visible_request_for_runtime_scope(
        run_context: &LoopRunContext,
        fallback_user_id: &UserId,
    ) -> HostVisibleCapabilityRequest {
        let policy =
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses");
        let empty_mounts = MountView::default();

        visible_capability_request(
            run_context,
            fallback_user_id,
            VisibleCapabilityInputs {
                workspace_mounts: &empty_mounts,
                skill_mounts: &empty_mounts,
                memory_mounts: &empty_mounts,
                system_extensions_lifecycle_mounts: &empty_mounts,
                policy: &policy,
                surface_policy: &CapabilitySurfacePolicy::allow_all(),
                extension_surface: &ExtensionCapabilitySurface::default(),
            },
        )
        .expect("visible request")
    }

    fn provider_tool_call_with_name(name: &str, arguments: serde_json::Value) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn-1".to_string()),
            id: "call-1".to_string(),
            name: ProviderToolName::new(name).expect("provider tool name"),
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn provider_tool_call(arguments: serde_json::Value) -> ProviderToolCall {
        provider_tool_call_with_name("builtin_echo", arguments)
    }

    fn invocation_for_candidate(candidate: &CapabilityCallCandidate) -> LoopRequest {
        LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: candidate.surface_version.clone(),
            capability_id: candidate.capability_id.clone(),
            input_ref: candidate.input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        }
    }

    struct StaticOutboundDeliveryTargetProvider {
        entry: OutboundDeliveryTargetEntry,
        expected_caller: std::sync::Mutex<Option<OutboundDeliveryTargetScope>>,
        observed_callers: std::sync::Mutex<Vec<OutboundDeliveryTargetScope>>,
    }

    impl StaticOutboundDeliveryTargetProvider {
        fn new(entry: OutboundDeliveryTargetEntry) -> Self {
            Self {
                entry,
                expected_caller: std::sync::Mutex::new(None),
                observed_callers: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn expect_caller(&self, caller: OutboundDeliveryTargetScope) {
            *self.expected_caller.lock().expect("caller lock") = Some(caller);
        }

        fn observed_callers(&self) -> Vec<OutboundDeliveryTargetScope> {
            self.observed_callers
                .lock()
                .expect("observed caller lock")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl OutboundDeliveryTargetProvider for StaticOutboundDeliveryTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            self.observed_callers
                .lock()
                .expect("observed caller lock")
                .push(caller.clone());
            if self
                .expected_caller
                .lock()
                .expect("caller lock")
                .as_ref()
                .is_some_and(|expected| expected != caller)
            {
                return Ok(Vec::new());
            }
            // Fixture answers a single expected caller; claim that caller as
            // owner so the entry survives the registry caller-scoping filter.
            let mut entry = self.entry.clone();
            entry.owner = OutboundDeliveryTargetOwner::for_scope(caller);
            Ok(vec![entry])
        }
    }

    fn expected_outbound_delivery_caller(
        run_context: &LoopRunContext,
        user_id: UserId,
    ) -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(run_context.scope.tenant_id.clone(), user_id)
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        )
    }

    #[derive(Debug, Default)]
    struct UnavailableModelGateway;

    #[async_trait::async_trait]
    impl HostManagedModelGateway for UnavailableModelGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "test gateway is not wired",
            ))
        }
    }

    async fn assert_github_capabilities_visible(
        wiring: &CapabilityPortWiring,
        run_context: &LoopRunContext,
    ) {
        let port = wiring
            .capability_factory
            .create_capability_port(run_context)
            .await
            .expect("capability port");
        let initial_tool_definition_ids = port
            .tool_definitions()
            .expect("initial tool definitions")
            .into_iter()
            .map(|definition| definition.capability_id.as_str().to_string())
            .collect::<Vec<_>>();
        assert!(
            initial_tool_definition_ids
                .iter()
                .any(|id| id == "github.search_issues"),
            "fresh capability ports must initialize active extension tools for auth-resume replay"
        );
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let capability_ids = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<Vec<_>>();

        assert!(capability_ids.contains(&"github.search_issues"));
        assert!(capability_ids.contains(&"github.get_issue"));
        assert!(capability_ids.contains(&"github.comment_issue"));
        assert!(!capability_ids.contains(&SPAWN_SUBAGENT_CAPABILITY_ID));
    }

    async fn assert_gsuite_capabilities_visibility(
        wiring: &CapabilityPortWiring,
        run_context: &LoopRunContext,
        expected: GsuiteCapabilityVisibility,
    ) {
        let (descriptor_ids, tool_definition_ids) =
            visible_capability_ids(wiring, run_context).await;

        for capability_id in gsuite_capability_ids() {
            let descriptor_visible = descriptor_ids.iter().any(|id| id == capability_id);
            let tool_visible = tool_definition_ids.iter().any(|id| id == capability_id);
            match expected {
                GsuiteCapabilityVisibility::Visible => {
                    assert!(
                        descriptor_visible,
                        "{capability_id} should be visible on the capability surface"
                    );
                    assert!(
                        tool_visible,
                        "{capability_id} should be advertised to the model as a provider tool"
                    );
                }
                GsuiteCapabilityVisibility::HiddenUntilActivated => {
                    assert!(
                        !descriptor_visible,
                        "{capability_id} should not be visible before activation"
                    );
                    assert!(
                        !tool_visible,
                        "{capability_id} should not be advertised before activation"
                    );
                }
            }
        }
    }

    async fn visible_capability_ids(
        wiring: &CapabilityPortWiring,
        run_context: &LoopRunContext,
    ) -> (Vec<String>, Vec<String>) {
        let port = wiring
            .capability_factory
            .create_capability_port(run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let descriptor_ids = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str().to_string())
            .collect::<Vec<_>>();
        let tool_definitions = port.tool_definitions().expect("tool definitions");
        let tool_definition_ids = tool_definitions
            .iter()
            .map(|definition| definition.capability_id.as_str().to_string())
            .collect::<Vec<_>>();

        (descriptor_ids, tool_definition_ids)
    }

    #[tokio::test]
    async fn disabled_trigger_poller_hides_only_manual_run_capability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                "disabled-trigger-run-capability",
                dir.path().join("standalone"),
            )
            .with_runtime_policy(local_host_minimal_approval_policy()),
        )
        .await
        .expect("standalone services build");
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new("disabled-trigger-run-user").expect("user id"),
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            false,
        )
        .expect("standalone capability wiring");

        let definitions = wiring
            .capability_factory
            .create_capability_port(&run_context("disabled-trigger-run-capability").await)
            .await
            .expect("capability port")
            .tool_definitions()
            .expect("tool definitions");

        assert!(definitions.iter().any(|definition| {
            definition.capability_id.as_str() == ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID
        }));
        assert!(!definitions.iter().any(|definition| {
            definition.capability_id.as_str() == ironclaw_host_runtime::TRIGGER_RUN_CAPABILITY_ID
        }));
    }

    #[tokio::test]
    async fn extension_remove_tool_discloses_generic_unpair_disconnect_semantics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                "extension-remove-generic-unpair-tool-copy",
                dir.path().join("standalone"),
            )
            .with_runtime_policy(local_host_minimal_approval_policy()),
        )
        .await
        .expect("standalone services build");
        let run_context = run_context("extension-remove-generic-unpair-tool-copy").await;
        let user_id = UserId::new("extension-remove-unpair-user").expect("user id");
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            user_id,
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");

        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let remove_tool = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id.as_str() == EXTENSION_REMOVE_CAPABILITY_ID)
            .expect("extension_remove tool definition");
        let description = remove_tool.description.to_ascii_lowercase();

        for required in [
            "uninstall",
            "remove",
            "disconnect",
            "unpair",
            "unlink",
            "revoke",
            "external channel",
            "current external chat",
            "extension_id",
            "identity",
            "channel binding",
        ] {
            assert!(
                description.contains(required),
                "extension_remove description must tell the model how to handle generic unpair/disconnect requests; missing {required:?} in: {}",
                remove_tool.description
            );
        }
        assert!(
            !description.contains("slack"),
            "extension_remove is a generic lifecycle tool and should not hard-code provider-specific examples: {}",
            remove_tool.description
        );
    }

    fn gsuite_capability_ids() -> [&'static str; 15] {
        [
            "gmail.list_messages",
            "gmail.get_message",
            "gmail.send_message",
            "gmail.create_draft",
            "gmail.reply_to_message",
            "gmail.trash_message",
            "google-calendar.list_calendars",
            "google-calendar.list_events",
            "google-calendar.get_event",
            "google-calendar.find_free_slots",
            "google-calendar.create_event",
            "google-calendar.update_event",
            "google-calendar.delete_event",
            "google-calendar.add_attendees",
            "google-calendar.set_reminder",
        ]
    }

    struct GsuiteSurfaceHarness {
        _dir: tempfile::TempDir,
        wiring: CapabilityPortWiring,
        run_context: LoopRunContext,
    }

    #[derive(Clone, Copy)]
    enum GsuiteCapabilityVisibility {
        Visible,
        HiddenUntilActivated,
    }

    #[derive(Clone, Copy)]
    enum GsuiteExtensionState {
        Installed,
        Activated,
    }

    async fn gsuite_surface_harness(
        owner: &str,
        label: &str,
        user: &str,
        extension_state: GsuiteExtensionState,
    ) -> GsuiteSurfaceHarness {
        let dir = tempfile::tempdir().expect("tempdir");
        // Dummy but well-formed Google OAuth backend: this harness exercises
        // GSuite/Gmail activation and dispatch (per-account credential
        // gating), not the provider-instance readiness map — without this,
        // google-family activation below fails closed with
        // `ProviderInstanceNotConfigured` before it ever reaches the
        // per-account gate these tests target.
        let google_oauth_backend = crate::OAuthClientConfig::new(
            "itest-google-client-id.apps.googleusercontent.com",
            "http://127.0.0.1/oauth/callback/google",
            None,
        )
        .expect("valid test google oauth client config");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                owner,
                dir.path().join("standalone"),
            )
            .with_runtime_policy(local_host_minimal_approval_policy())
            .with_vendor_oauth_client(ironclaw_auth::GOOGLE_PROVIDER_ID, google_oauth_backend),
        )
        .await
        .expect("standalone services build");
        let run_context = run_context(label).await;
        install_gsuite_extensions(
            &services,
            &run_context,
            &UserId::new(user).expect("surface user id"),
            extension_state,
        )
        .await;
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new(user).expect("user id"),
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");

        enable_global_auto_approve_for_run(
            &services,
            &run_context,
            &UserId::new(user).expect("user id"),
        )
        .await;

        GsuiteSurfaceHarness {
            _dir: dir,
            wiring,
            run_context,
        }
    }

    /// Seed a Configured credential account + its access secret for one
    /// vendor in the caller's scope — the #6520 caller-phase surface shows an
    /// extension's tools only when the caller's readiness is Active, which
    /// requires the manifest-declared credentials to resolve.
    async fn seed_configured_account_and_secret_with_scopes(
        services: &crate::factory::RebornRuntimeStores,
        scope: &ironclaw_host_api::resource::ResourceScope,
        provider: &str,
        scopes: &[&str],
    ) {
        use ironclaw_auth::{
            AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
            CredentialAccountStatus, CredentialOwnership, NewCredentialAccount, ProviderScope,
        };
        services
            .product_auth
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::credential_owner(scope, AuthSurface::Api),
                provider: AuthProviderId::new(provider).expect("provider"),
                label: CredentialAccountLabel::new(provider).expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::ids::SecretHandle::new(format!("{provider}-test-token"))
                        .expect("secret handle"),
                ),
                refresh_secret: None,
                scopes: scopes
                    .iter()
                    .map(|scope| ProviderScope::new((*scope).to_string()).expect("valid scope"))
                    .collect(),
            })
            .await
            .expect("create configured account");
        let owner_scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        services
            .secret_store()
            .put(
                owner_scope.resource,
                ironclaw_host_api::ids::SecretHandle::new(format!("{provider}-test-token"))
                    .expect("secret handle"),
                ironclaw_secrets::SecretMaterial::from(format!("{provider}-access-token")),
                None,
            )
            .await
            .expect("seed access token");
    }

    async fn seed_configured_account_and_secret(
        services: &crate::factory::RebornRuntimeStores,
        scope: &ironclaw_host_api::resource::ResourceScope,
        provider: &str,
    ) {
        seed_configured_account_and_secret_with_scopes(services, scope, provider, &[]).await;
    }

    /// Account WITHOUT secret material: satisfies caller-phase readiness (the
    /// tool surfaces) while dispatch-time injection still raises the OAuth
    /// gate for the missing secret.
    async fn seed_configured_account_without_secret_with_scopes(
        services: &crate::factory::RebornRuntimeStores,
        scope: &ironclaw_host_api::resource::ResourceScope,
        provider: &str,
        scopes: &[&str],
    ) {
        use ironclaw_auth::{
            AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
            CredentialAccountStatus, CredentialOwnership, NewCredentialAccount, ProviderScope,
        };
        services
            .product_auth
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::credential_owner(scope, AuthSurface::Api),
                provider: AuthProviderId::new(provider).expect("provider"),
                label: CredentialAccountLabel::new(provider).expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::ids::SecretHandle::new(format!("{provider}-test-token"))
                        .expect("secret handle"),
                ),
                refresh_secret: None,
                scopes: scopes
                    .iter()
                    .map(|scope| ProviderScope::new((*scope).to_string()).expect("valid scope"))
                    .collect(),
            })
            .await
            .expect("create configured account");
    }

    async fn install_gsuite_extensions(
        services: &crate::factory::RebornRuntimeStores,
        run_context: &LoopRunContext,
        surface_user: &UserId,
        extension_state: GsuiteExtensionState,
    ) {
        // Caller-phase readiness (#6520): the surface shows an extension's
        // tools only when the caller's google account resolves, so Activated
        // seeds a Configured account under the run scope. Material is
        // deliberately withheld — surface visibility keys on the account,
        // dispatch-time injection keys on the secret, letting the gmail
        // auth-gate test drive an OAuth gate on a visible tool.
        if matches!(extension_state, GsuiteExtensionState::Activated) {
            let seed_scope =
                crate::runtime::capability_host::resource_scope_for_run(run_context, surface_user);
            seed_configured_account_without_secret_with_scopes(
                services,
                &seed_scope,
                "google",
                ironclaw_extension_support::GSUITE_PROVIDER_SCOPES,
            )
            .await;
        }
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let extension_management = runtime_surfaces.extension_management.clone();
        // #6520 membership: every install is private to its caller
        // (`derive_owner`), so install AS the run's surface user — an
        // operator install would be invisible to that user. #6520 also
        // removed the public Activate action; a bare install seeds the
        // pre-readiness row, and the Activated state drives the port's
        // prechecked activation directly (creds treated as present).
        for extension_id in ["gmail", "google-calendar"] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id)
                    .expect("valid extension ref");
            extension_management
                .install(package_ref.clone(), surface_user)
                .await
                .expect("install GSuite extension");
            if matches!(extension_state, GsuiteExtensionState::Activated) {
                extension_management
                    .activate_with_prechecked_credentials_for_user_for_test(
                        package_ref,
                        surface_user,
                    )
                    .await
                    .expect("activate GSuite extension");
            }
        }
    }

    #[allow(
        dead_code,
        reason = "kept as a standalone runtime credential-account test double"
    )]
    struct ConfiguredRuntimeCredentialAccounts;

    #[async_trait::async_trait]
    impl ironclaw_auth::RuntimeCredentialAccountSelectionService
        for ConfiguredRuntimeCredentialAccounts
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: ironclaw_auth::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let now = chrono::Utc::now();
            Ok(ironclaw_auth::CredentialAccount {
                id: ironclaw_auth::CredentialAccountId::new(),
                scope: ironclaw_auth::AuthProductScope::new(
                    ironclaw_host_api::resource::ResourceScope::local_default(
                        UserId::new("configured-credential-user").expect("user id"),
                        ironclaw_host_api::ids::InvocationId::new(),
                    )
                    .expect("resource scope"),
                    ironclaw_auth::AuthSurface::Api,
                ),
                provider: ironclaw_auth::AuthProviderId::new("test-provider").expect("provider id"),
                label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                    .expect("account label"),
                status: ironclaw_auth::CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::ids::SecretHandle::new("test-secret")
                        .expect("secret handle"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
                provider_identity: None,
                link_revision: 0,
                created_at: now,
                updated_at: now,
            })
        }
    }

    #[tokio::test]
    async fn capability_io_writes_durable_preview_message_and_live_upsert_id() {
        let run_context = run_context("durable-preview").await;
        let fallback_user_id = UserId::new("durable-preview-owner").expect("fallback user id");
        // The durable preview sink derives the thread scope from the run context
        // (matching where the run's thread was registered), not a fixed
        // composition-time scope. Register the thread under that derived scope.
        let thread_scope =
            thread_scope_for_run(&run_context, &fallback_user_id).expect("run scope has an agent");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service.clone(),
            fallback_user_id.clone(),
            None,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();

        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");
        let CapabilityWriteResult { result_ref, .. } = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output: serde_json::json!({"content": "hello"}),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("result stages");

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: run_context.thread_id.clone(),
            })
            .await
            .expect("history loads");
        let preview_message = history
            .messages
            .iter()
            .find(|message| message.kind == MessageKind::CapabilityDisplayPreview)
            .expect("durable preview message");
        let run_id = run_context.run_id.to_string();
        assert_eq!(
            preview_message.turn_run_id.as_deref(),
            Some(run_id.as_str())
        );
        assert_eq!(
            preview_message.tool_result_ref.as_deref(),
            Some(result_ref.as_str())
        );
        assert!(preview_message.tool_result_provider_call.is_none());
        let preview_record = display_previews
            .record_for_invocation(invocation_id)
            .expect("live preview record");
        assert_eq!(
            preview_record.timeline_message_id,
            Some(preview_message.message_id)
        );
    }

    /// Regression: the durable preview sink must write under the RUN's own
    /// thread scope, not a fixed composition-time/fallback scope. A run with an
    /// explicit owner, whose thread is registered under that owner, must still
    /// get its durable preview even when the sink's fallback user differs — the
    /// prior fixed-scope sink produced a spurious `UnknownThread` here, which is
    /// the "thread is unknown to the durable store" symptom seen in the field.
    #[tokio::test]
    async fn durable_preview_uses_run_scope_not_fixed_fallback() {
        let owner = UserId::new("run-owner").expect("owner user id");
        let run_context = run_context_with_scope(TurnScope::new_with_owner(
            TenantId::new("tenant-scope-fix").expect("tenant id"),
            Some(AgentId::new("agent-scope-fix").expect("agent id")),
            Some(ProjectId::new("project-scope-fix").expect("project id")),
            ThreadId::new("thread-scope-fix").expect("thread id"),
            Some(owner.clone()),
        ))
        .await;
        // Register the thread under the RUN's scope (owner = the run owner).
        let thread_scope =
            thread_scope_for_run(&run_context, &owner).expect("run scope has an agent");
        assert_eq!(thread_scope.owner_user_id.as_ref(), Some(&owner));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        // Sink built with a DIFFERENT fallback user. The old fixed-scope sink
        // would have appended under a mismatched scope and failed; the run-scope
        // derivation must ignore this fallback because the run carries an owner.
        let unrelated_fallback = UserId::new("env-operator-unrelated").expect("fallback user id");
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service.clone(),
            unrelated_fallback,
            None,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");
        capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output: serde_json::json!({"content": "hello"}),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("result stages");

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: run_context.thread_id.clone(),
            })
            .await
            .expect("history loads");
        assert!(
            history
                .messages
                .iter()
                .any(|message| message.kind == MessageKind::CapabilityDisplayPreview),
            "durable preview must be written under the run's own scope, not the fallback"
        );
        let preview_record = display_previews
            .record_for_invocation(invocation_id)
            .expect("live preview record");
        assert!(
            preview_record.timeline_message_id.is_some(),
            "durable append should have linked a timeline message id under the run scope"
        );
    }

    #[tokio::test]
    async fn capability_io_writes_durable_preview_under_run_actor_owner() {
        let actor_user_id = UserId::new("preview-actor").expect("actor user id");
        let runtime_owner_id = UserId::new("runtime-owner").expect("runtime owner id");
        let run_context = run_context("durable-preview-actor-owner")
            .await
            .with_actor(TurnActor::new(actor_user_id.clone()));
        let base_thread_scope = ThreadScope {
            tenant_id: run_context.scope.tenant_id.clone(),
            agent_id: run_context.scope.agent_id.clone().expect("agent id"),
            project_id: run_context.scope.project_id.clone(),
            owner_user_id: Some(runtime_owner_id.clone()),
            mission_id: None,
        };
        let actor_thread_scope = ThreadScope {
            owner_user_id: Some(actor_user_id.clone()),
            ..base_thread_scope.clone()
        };
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: actor_thread_scope.clone(),
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: format!("user:{}", actor_user_id.as_str()),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("actor-owned thread exists");
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service.clone(),
            runtime_owner_id,
            None,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();

        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");
        let CapabilityWriteResult { result_ref, .. } = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output: serde_json::json!({"content": "hello"}),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("result stages");

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: actor_thread_scope,
                thread_id: run_context.thread_id.clone(),
            })
            .await
            .expect("actor-owned history loads");
        let preview_message = history
            .messages
            .iter()
            .find(|message| message.kind == MessageKind::CapabilityDisplayPreview)
            .expect("durable preview message under actor owner");
        assert_eq!(
            preview_message.tool_result_ref.as_deref(),
            Some(result_ref.as_str())
        );
        let preview_record = display_previews
            .record_for_invocation(invocation_id)
            .expect("live preview record");
        assert_eq!(
            preview_record.timeline_message_id,
            Some(preview_message.message_id)
        );
    }

    #[tokio::test]
    async fn capability_io_writes_inline_result_without_durable_thread() {
        let run_context = run_context("durable-preview-failure").await;
        let fallback_user_id = UserId::new("durable-preview-owner").expect("fallback user id");
        // No thread is registered: the durable artifact cutover makes the
        // result retrievable through the staged/inline result path, so the
        // write must still succeed; only the durable preview append is
        // skipped (best-effort).
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service,
            fallback_user_id,
            None,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();

        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");
        let write_result = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output: serde_json::json!({"content": "hello"}),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("missing thread must not reject a result that stays retrievable inline");
        assert!(
            write_result
                .model_observation
                .as_ref()
                .is_some_and(|observation| {
                    matches!(
                        observation.detail,
                        ironclaw_loop_contracts::ToolObservationDetail::InlineResult { .. }
                    )
                }),
            "inline result observation expected without a durable thread: {write_result:?}"
        );
    }

    #[tokio::test]
    async fn capability_io_persists_result_beyond_legacy_storage_limit_as_artifact() {
        let run_context = run_context("artifact-result-over-legacy-limit").await;
        let fallback_user_id = UserId::new("durable-result-owner").expect("fallback user id");
        let thread_scope =
            thread_scope_for_run(&run_context, &fallback_user_id).expect("run scope has an agent");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let artifact_store = Arc::new(
            ironclaw_threads::DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new()))
                .expect("artifact store"),
        );
        let persistence: Arc<dyn AccountedArtifactPersister> = artifact_store;
        let governor: Arc<dyn ironclaw_resources::ResourceGovernor> =
            Arc::new(ironclaw_resources::InMemoryResourceGovernor::new());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::new(CapabilityDisplayPreviewStore::default()),
            thread_service,
            fallback_user_id,
            None,
        )
        .with_artifact_services(persistence, governor);
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");

        let written = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &capability_id,
                output: serde_json::Value::String("x".repeat(4 * 1024 * 1024)),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("artifact-backed result writes");
        let observation = written.model_observation.expect("artifact observation");
        assert!(matches!(
            observation.detail,
            ToolObservationDetail::ArtifactReference { .. }
        ));
        assert!(written.byte_len > 4 * 1024 * 1024);
    }

    /// Subagent settlement replaces the transcript's artifact observation
    /// while both immutable result artifacts remain durable. Deleting the
    /// transient staging entry must not delete either artifact.
    #[tokio::test]
    async fn update_and_delete_capability_result_preserve_immutable_artifacts() {
        let run_context = run_context("durable-update-delete").await;
        let fallback_user_id = UserId::new("durable-update-delete-owner").expect("user id");
        let thread_scope =
            thread_scope_for_run(&run_context, &fallback_user_id).expect("run scope has an agent");
        let resource_scope = resource_scope_for_run(&run_context, &fallback_user_id);
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let artifact_store = Arc::new(
            ironclaw_threads::DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new()))
                .expect("artifact store"),
        );
        let persistence: Arc<dyn AccountedArtifactPersister> = artifact_store.clone();
        let governor: Arc<dyn ironclaw_resources::ResourceGovernor> =
            Arc::new(ironclaw_resources::InMemoryResourceGovernor::new());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::new(CapabilityDisplayPreviewStore::default()),
            thread_service.clone(),
            fallback_user_id,
            None,
        )
        .with_artifact_services(persistence, governor);
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let capability_id = CapabilityId::new("builtin.spawn_subagent").expect("capability id");
        let original_output =
            serde_json::json!({"content": "o".repeat(ARTIFACT_INLINE_PREVIEW_MAX_BYTES + 1)});
        let write = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &capability_id,
                output: original_output.clone(),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("initial artifact write succeeds");
        let original_observation = write
            .model_observation
            .clone()
            .expect("initial write exposes artifact observation");
        let original_artifact_uri = match &original_observation.detail {
            ToolObservationDetail::ArtifactReference { artifact_ref, .. } => artifact_ref.clone(),
            _ => panic!("initial large result must expose an artifact reference"),
        };
        thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                intrinsic_outcome: None,
                scope: thread_scope.clone(),
                thread_id: run_context.thread_id.clone(),
                turn_run_id: run_context.run_id.to_string(),
                result_ref: write.result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent running").expect("safe summary"),
                provider_call: None,
                model_observation: Some(
                    serde_json::to_value(original_observation)
                        .expect("initial observation serializes"),
                ),
            })
            .await
            .expect("initial transcript reference appends");

        let updated_output = serde_json::json!({"content": "updated"});
        let first_update = capability_io
            .update_capability_result(&run_context, &write.result_ref, updated_output.clone())
            .await
            .expect("subagent settlement update succeeds");
        let replayed_update = capability_io
            .update_capability_result(&run_context, &write.result_ref, updated_output.clone())
            .await
            .expect("subagent settlement replay succeeds");
        assert_eq!(
            replayed_update.completed_artifact, first_update.completed_artifact,
            "a settlement replay must adopt the same immutable artifact",
        );
        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: run_context.thread_id.clone(),
            })
            .await
            .expect("thread history loads");
        let envelope = history
            .messages
            .iter()
            .find(|message| message.kind == MessageKind::ToolResultReference)
            .and_then(|message| message.content.as_deref())
            .map(ironclaw_threads::ToolResultReferenceEnvelope::from_json_str)
            .expect("transcript result reference exists")
            .expect("transcript result reference decodes");
        let updated_observation: ModelVisibleToolObservation = serde_json::from_value(
            envelope
                .model_observation
                .expect("settlement replaces model observation"),
        )
        .expect("settled observation decodes");
        let ToolObservationDetail::ArtifactReference {
            artifact_ref: updated_artifact_uri,
            ..
        } = updated_observation.detail
        else {
            panic!("settled result must expose its new artifact reference");
        };
        assert_ne!(original_artifact_uri, updated_artifact_uri);

        capability_io
            .delete_capability_result(&run_context, &write.result_ref)
            .await
            .expect("transient delete succeeds");
        assert!(
            capability_io
                .result_output(write.result_ref.as_str())
                .expect("staging lookup succeeds")
                .is_none(),
            "delete must clear only the transient staging copy"
        );

        for (artifact_uri, expected_output) in [
            (original_artifact_uri.as_str(), &original_output),
            (updated_artifact_uri.as_str(), &updated_output),
        ] {
            let artifact_ref = artifact_uri
                .parse::<ArtifactRef>()
                .expect("artifact URI parses");
            let expected = serde_json::to_vec(expected_output).expect("output serializes");
            let stored = artifact_store
                .read(ArtifactReadRequest {
                    owner_scope: ArtifactOwnerScope::from_resource_scope(&resource_scope),
                    namespace: run_context.effective_artifact_namespace(),
                    target: ArtifactReadTarget {
                        artifact_id: artifact_ref.id(),
                        selector: ArtifactSelector::Full,
                        max_output_bytes: u64::try_from(expected.len())
                            .expect("expected byte length"),
                    },
                })
                .await
                .expect("artifact read succeeds")
                .expect("immutable artifact remains readable");
            assert_eq!(stored.content, expected);
        }
    }

    /// Issue #5838: a result under the preview cap gets an inline first-look
    /// preview covering the whole serialized output, with no truncation
    /// markers, so the model does not need a follow-up `result_read` call.
    #[tokio::test]
    async fn write_capability_result_observation_carries_full_preview_when_under_cap() {
        let run_context = run_context("first-look-preview-full").await;
        let fallback_user_id =
            UserId::new("first-look-preview-full-owner").expect("fallback user id");
        let thread_scope =
            thread_scope_for_run(&run_context, &fallback_user_id).expect("run scope has an agent");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service,
            fallback_user_id,
            None,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("builtin.echo").expect("capability id");
        let output = serde_json::json!({"content": "hello"});
        let full_text = serde_json::to_string(&output).expect("serialize reference output");

        let write_result = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output: output.clone(),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("small result stages");

        let observation = write_result
            .model_observation
            .as_ref()
            .expect("write result carries a first-look observation");
        match &observation.detail {
            ironclaw_loop_contracts::ToolObservationDetail::InlineResult {
                content,
                byte_len,
                item_count,
            } => {
                assert_eq!(
                    content, &full_text,
                    "inline content must cover the whole output"
                );
                assert_eq!(*byte_len, full_text.len() as u64);
                assert_eq!(*item_count, None);
            }
            detail => panic!("expected complete inline result content, got {detail:?}"),
        }
        assert!(
            observation.artifacts.is_empty(),
            "inline content must not advertise a false result-reference recovery target"
        );
        assert!(
            !observation.summary.contains("artifact://"),
            "a complete preview must not advertise an unnecessary artifact read"
        );
    }

    /// Issue: a truncated preview that slices mid-JSON-array leaves the model
    /// unable to tell how many items the full result contains. When the
    /// capability output is a top-level JSON array, the truncated-branch
    /// observation carries `item_count` and mentions it in the summary.
    #[tokio::test]
    async fn write_capability_result_truncated_array_preview_reports_item_count() {
        let run_context = run_context("first-look-preview-array").await;
        let fallback_user_id =
            UserId::new("first-look-preview-array-owner").expect("fallback user id");
        let thread_scope =
            thread_scope_for_run(&run_context, &fallback_user_id).expect("run scope has an agent");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope,
                thread_id: Some(run_context.thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread exists");
        let artifact_store = Arc::new(
            ironclaw_threads::DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new()))
                .expect("artifact store"),
        );
        let persistence: Arc<dyn AccountedArtifactPersister> = artifact_store;
        let governor: Arc<dyn ironclaw_resources::ResourceGovernor> =
            Arc::new(ironclaw_resources::InMemoryResourceGovernor::new());
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service,
            fallback_user_id,
            None,
        )
        .with_artifact_services(persistence, governor);
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"query": "items"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("ironclaw.memory.search").expect("capability id");

        // Short strings serialize well over the preview cap. The truncation
        // branch keys on `ARTIFACT_INLINE_PREVIEW_MAX_BYTES`, so the fixture
        // must exceed THAT bound — a fixture sized against the smaller
        // `TOOL_RESULT_RECORD_READ_MAX_BYTES` stays a complete inline result
        // and never reaches the artifact-reference path under test.
        const ITEM_COUNT: usize = 8000;
        let items: Vec<String> = (0..ITEM_COUNT).map(|i| format!("item-{i:04}")).collect();
        let output = serde_json::json!(items);
        let full_text = serde_json::to_string(&output).expect("serialize reference output");
        assert!(
            full_text.len() > ARTIFACT_INLINE_PREVIEW_MAX_BYTES,
            "fixture must exceed the inline preview cap"
        );

        let write_result = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("large array result stages");

        let observation = write_result
            .model_observation
            .as_ref()
            .expect("write result carries a first-look observation");
        assert!(
            observation.summary.contains(&format!("{ITEM_COUNT} items")),
            "truncated summary must state the array's element count: {}",
            observation.summary
        );
        let artifact_uri = observation
            .artifacts
            .first()
            .expect("artifact metadata")
            .artifact_ref
            .clone();
        match &observation.detail {
            ironclaw_loop_contracts::ToolObservationDetail::ArtifactReference {
                artifact_ref,
                item_count: Some(count),
                total_bytes,
                ..
            } => {
                assert_eq!(artifact_ref, &artifact_uri);
                assert_eq!(*count, ITEM_COUNT as u64);
                assert_eq!(*total_bytes, write_result.byte_len);
            }
            detail => panic!("expected an artifact-backed array preview, got {detail:?}"),
        }
        let encoded = serde_json::to_value(observation).expect("observation serializes");
        assert!(
            !encoded["detail"]
                .as_object()
                .expect("detail object")
                .contains_key("next_offset")
        );

        // Singleton boundary: one oversized element still counts as an array
        // of 1, not a scalar.
        let singleton_input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"query": "one big item"})),
            )
            .await
            .expect("singleton input stages");
        let singleton_output =
            serde_json::json!(["x".repeat(ARTIFACT_INLINE_PREVIEW_MAX_BYTES + 1000)]);
        let singleton_write = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &singleton_input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &capability_id,
                output: singleton_output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("singleton array result stages");
        let singleton_observation = singleton_write
            .model_observation
            .as_ref()
            .expect("singleton write carries a first-look observation");
        assert!(
            singleton_observation.summary.contains("1 items"),
            "singleton summary must state the element count: {}",
            singleton_observation.summary
        );
        let singleton_artifact_uri = singleton_observation
            .artifacts
            .first()
            .expect("singleton artifact metadata")
            .artifact_ref
            .clone();
        match &singleton_observation.detail {
            ironclaw_loop_contracts::ToolObservationDetail::ArtifactReference {
                artifact_ref,
                item_count: Some(count),
                total_bytes,
                ..
            } => {
                assert_eq!(artifact_ref, &singleton_artifact_uri);
                assert_eq!(*count, 1);
                assert_eq!(*total_bytes, singleton_write.byte_len);
            }
            detail => panic!("expected an artifact-backed singleton array, got {detail:?}"),
        }
        let singleton_encoded =
            serde_json::to_value(singleton_observation).expect("observation serializes");
        assert!(
            !singleton_encoded["detail"]
                .as_object()
                .expect("detail object")
                .contains_key("next_offset")
        );
    }

    #[tokio::test]
    async fn capability_io_resolves_input_refs_repeatedly() {
        let capability_io = StagedCapabilityIo::default();
        let run_context = run_context("repeat-input").await;
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");

        let first = capability_io
            .resolve_capability_input(&run_context, &input_ref)
            .await
            .expect("first resolve succeeds");
        let second = capability_io
            .resolve_capability_input(&run_context, &input_ref)
            .await
            .expect("second resolve succeeds");

        assert_eq!(first, serde_json::json!({"message": "hello"}));
        assert_eq!(second, serde_json::json!({"message": "hello"}));
    }

    #[tokio::test]
    async fn capability_io_rejects_cross_run_and_unstaged_input_refs() {
        let capability_io = StagedCapabilityIo::default();
        let current_context = run_context("input-scope-a").await;
        let other_context = run_context("input-scope-b").await;
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &current_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");

        let cross_run = capability_io
            .resolve_capability_input(&other_context, &input_ref)
            .await
            .expect_err("foreign run should fail");
        assert_eq!(cross_run.kind, AgentLoopHostErrorKind::ScopeMismatch);

        let missing_ref =
            CapabilityInputRef::new(format!("input:{}:missing", current_context.run_id))
                .expect("missing ref");
        let missing = capability_io
            .resolve_capability_input(&current_context, &missing_ref)
            .await
            .expect_err("unstaged ref should fail");
        assert_eq!(missing.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn result_store_evicts_oldest_entries_to_stay_under_byte_cap() {
        let mut store = StagedValueStore::default();
        let first = serde_json::Value::String("a".repeat(3 * 1024 * 1024));
        let first_bytes = serialized_result_output(&first)
            .expect("first result serializes")
            .len();
        store
            .insert_with_oldest_eviction("result:first".to_string(), first, first_bytes)
            .expect("first result stages");
        let second = serde_json::Value::String("b".repeat(2 * 1024 * 1024));
        let second_bytes = serialized_result_output(&second)
            .expect("second result serializes")
            .len();
        store
            .insert_with_oldest_eviction("result:second".to_string(), second, second_bytes)
            .expect("second result stages");

        assert!(store.get("result:first").is_none());
        assert!(store.get("result:second").is_some());
        assert!(store.total_bytes <= CAPABILITY_IO_MAX_STAGED_BYTES);
    }

    #[tokio::test]
    async fn capability_io_sends_only_bounded_output_to_the_diagnostic_sink() {
        let sink = Arc::new(RecordingToolDiagnosticSink::default());
        let capability_io = StagedCapabilityIo {
            tool_diagnostics: HostManagedToolDiagnosticEmitter::new(Some(
                Arc::clone(&sink) as Arc<dyn HostManagedPromptDiagnosticSink>
            )),
            ..StagedCapabilityIo::default()
        };
        let run_context = run_context("bounded-tool-diagnostic").await;
        let input_ref = CapabilityInputRef::new(format!(
            "input:{}:bounded-tool-diagnostic",
            run_context.run_id
        ))
        .expect("input ref");
        let secret = format!("Bearer {}", "s".repeat(80));
        let retained_prefix =
            "x".repeat(ironclaw_product_contracts::inspector::TOOL_RESULT_MAX_BYTES - secret.len());
        let output = serde_json::Value::String(format!(
            "{retained_prefix}{secret}{}",
            "y".repeat(TOOL_RESULT_DIAGNOSTIC_CAPTURE_MAX_BYTES * 2)
        ));
        let serialized_bytes = serialized_result_output(&output)
            .expect("result serializes")
            .len();
        let invocation_id = InvocationId::new();
        capability_io.record_running_invocation(&run_context, invocation_id, &input_ref);

        capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: None,
                completed_artifact: None,
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id,
                capability_id: &CapabilityId::new("builtin.echo").expect("capability id"),
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::InlineOnly,
                canonical_item_count: None,
            })
            .await
            .expect("result writes");

        let capture = sink
            .result
            .lock()
            .expect("diagnostic result lock")
            .take()
            .expect("tool diagnostic captured");
        let retained = capture
            .result
            .expect("successful result has diagnostic text");
        assert_eq!(retained.len(), TOOL_RESULT_DIAGNOSTIC_CAPTURE_MAX_BYTES);
        assert!(retained.contains(&secret));
        assert!(
            ironclaw_safety::LeakDetector::new()
                .redact_all_secrets(&retained)
                .1,
            "boundary-crossing secret must remain detectable"
        );
        assert_eq!(
            capture.result_original_bytes,
            Some(u64::try_from(serialized_bytes).expect("serialized size fits u64"))
        );
        assert!(
            capture.duration_ms.is_some(),
            "the capability writer must forward the measured invocation duration"
        );
    }

    #[test]
    fn standalone_builtin_surface_grants_capability_classes() {
        let policy =
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses");
        let capability_ids = policy
            .capability_ids()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>();

        assert!(capability_ids.contains(&CODING_WRITE_CAPABILITY_ID));
        assert!(capability_ids.contains(&CODING_EDIT_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_LIST_CAPABILITY_ID));
        // SKILL_ACTIVATE_CAPABILITY_ID is a synthetic capability added by
        // wrap_synthetic_capabilities, not a policy capability.
        assert!(!capability_ids.contains(&SKILL_ACTIVATE_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_UPDATE_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_REMOVE_CAPABILITY_ID));
        assert!(capability_ids.contains(&SHELL_CAPABILITY_ID));
        assert!(capability_ids.contains(&CODING_BASH_CAPABILITY_ID));
        assert!(capability_ids.contains(&HTTP_CAPABILITY_ID));
        assert!(capability_ids.contains(&HTTP_SAVE_CAPABILITY_ID));
        let local_host_allowed_effects = vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ];
        let local_host_shell_network_policy =
            crate::builtin_capability_policy::dev_wildcard_network_policy();
        assert_eq!(
            local_host_allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem
            ]
        );
        assert_eq!(
            policy.provider.authority_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ModifyApproval,
                EffectKind::ExternalWrite
            ]
        );

        let workspace_mounts =
            crate::runtime_mounts::workspace_mount_view(MountPermissions::read_write(), &[])
                .expect("workspace mounts build");
        let skill_mounts = crate::runtime_mounts::db_backed_skill_management_mount_view(
            &ironclaw_host_api::resource::ResourceScope::local_default(
                ironclaw_host_api::ids::UserId::new("grant-coverage-user").expect("user id"),
                ironclaw_host_api::ids::InvocationId::new(),
            )
            .expect("scope"),
        )
        .expect("skill mounts build");
        let memory_mounts =
            crate::runtime_mounts::memory_mount_view(MountPermissions::read_write_list_delete())
                .expect("memory mounts build");
        let system_extensions_lifecycle_mounts =
            crate::runtime_mounts::system_extensions_lifecycle_mount_view()
                .expect("system extensions lifecycle mounts build");
        assert!(workspace_mounts.mounts.iter().all(|mount| {
            mount.alias.as_str() != "/skills" && mount.alias.as_str() != "/system/skills"
        }));
        let mount_for = |alias: &str| {
            skill_mounts
                .mounts
                .iter()
                .find(|mount| mount.alias.as_str() == alias)
                .expect("mount exists")
        };
        assert_eq!(
            mount_for("/skills").permissions,
            MountPermissions::read_write_list_delete()
        );
        assert_eq!(
            mount_for("/system/skills").permissions,
            MountPermissions::read_only()
        );
        let grants = policy.builtin_grants(
            &ExtensionId::new("loop-driver").expect("valid extension id"),
            &workspace_mounts,
            &skill_mounts,
            &memory_mounts,
            &system_extensions_lifecycle_mounts,
        );
        let grant_for = |capability_id: &str| {
            grants
                .grants
                .iter()
                .find(|grant| grant.capability.as_str() == capability_id)
                .expect("capability grant exists")
        };

        let shell_grant = grant_for(SHELL_CAPABILITY_ID);
        assert_eq!(
            shell_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network
            ]
        );
        assert!(shell_grant.constraints.mounts.mounts.is_empty());
        assert_eq!(
            shell_grant.constraints.network,
            local_host_shell_network_policy
        );

        let bash_grant = grant_for(CODING_BASH_CAPABILITY_ID);
        assert_eq!(
            bash_grant.constraints, shell_grant.constraints,
            "bash and shell use the same process runtime authority"
        );

        let http_grant = grant_for(HTTP_CAPABILITY_ID);
        assert_eq!(
            http_grant.constraints.allowed_effects,
            vec![EffectKind::DispatchCapability, EffectKind::Network]
        );
        assert!(http_grant.constraints.mounts.mounts.is_empty());
        assert_eq!(
            http_grant.constraints.network,
            local_host_shell_network_policy
        );

        let http_save_grant = grant_for(HTTP_SAVE_CAPABILITY_ID);
        assert_eq!(
            http_save_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::WriteFilesystem
            ]
        );
        assert_eq!(http_save_grant.constraints.mounts, workspace_mounts);
        assert_eq!(
            http_save_grant.constraints.network,
            local_host_shell_network_policy
        );

        let memory_write_grant = grant_for(MEMORY_WRITE_CAPABILITY_ID);
        assert_eq!(
            memory_write_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem
            ]
        );
        assert_eq!(memory_write_grant.constraints.mounts, memory_mounts);
        assert_eq!(
            memory_write_grant.constraints.network,
            NetworkPolicy::default()
        );

        let extension_search_grant = grant_for(EXTENSION_SEARCH_CAPABILITY_ID);
        assert_eq!(
            extension_search_grant.constraints.allowed_effects,
            vec![EffectKind::DispatchCapability, EffectKind::ReadFilesystem]
        );
        assert_eq!(
            extension_search_grant.constraints.mounts,
            system_extensions_lifecycle_mounts
        );
        assert_eq!(
            extension_search_grant.constraints.network,
            NetworkPolicy::default()
        );

        let extension_register_grant = grant_for(EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID);
        assert_eq!(
            extension_register_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network
            ]
        );
        assert_eq!(
            extension_register_grant.constraints.mounts,
            system_extensions_lifecycle_mounts
        );
        assert_eq!(
            extension_register_grant
                .constraints
                .network
                .allowed_targets
                .iter()
                .map(|target| target.host_pattern.as_str())
                .collect::<Vec<_>>(),
            vec!["*"]
        );
        assert!(
            extension_register_grant
                .constraints
                .network
                .deny_private_ip_ranges
        );

        let extension_remove_grant = grant_for(EXTENSION_REMOVE_CAPABILITY_ID);
        assert_eq!(
            extension_remove_grant.constraints.allowed_effects,
            local_host_allowed_effects
        );
        assert_eq!(
            extension_remove_grant.constraints.mounts,
            system_extensions_lifecycle_mounts
        );
        assert_eq!(
            extension_remove_grant.constraints.network,
            NetworkPolicy::default()
        );

        // #6520 removed the separate activate capability; install drives
        // readiness and carries activate's wider grant (discovery network).
        let extension_install_grant = grant_for(EXTENSION_INSTALL_CAPABILITY_ID);
        assert_eq!(
            extension_install_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network
            ]
        );
        assert_eq!(
            extension_install_grant.constraints.mounts,
            system_extensions_lifecycle_mounts
        );
        assert_eq!(
            extension_install_grant
                .constraints
                .network
                .allowed_targets
                .iter()
                .map(|target| target.host_pattern.as_str())
                .collect::<Vec<_>>(),
            vec!["*"]
        );
        assert!(
            extension_install_grant
                .constraints
                .network
                .deny_private_ip_ranges
        );

        let read_grant = grant_for(CODING_READ_CAPABILITY_ID);
        assert_eq!(
            read_grant.constraints.allowed_effects,
            vec![EffectKind::DispatchCapability, EffectKind::ReadFilesystem]
        );
        assert_eq!(read_grant.constraints.mounts, workspace_mounts);
        assert_eq!(read_grant.constraints.network, NetworkPolicy::default());

        let skill_install_grant = grant_for(SKILL_INSTALL_CAPABILITY_ID);
        assert_eq!(
            skill_install_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network
            ]
        );
        assert_eq!(skill_install_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_install_grant.constraints.network,
            local_host_shell_network_policy
        );

        let skill_update_grant = grant_for(SKILL_UPDATE_CAPABILITY_ID);
        assert_eq!(
            skill_update_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ]
        );
        assert_eq!(skill_update_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_update_grant.constraints.network,
            NetworkPolicy::default()
        );

        let skill_auto_activate_grant = grant_for(SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID);
        assert_eq!(
            skill_auto_activate_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ]
        );
        assert_eq!(skill_auto_activate_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_auto_activate_grant.constraints.network,
            NetworkPolicy::default()
        );

        let skill_remove_grant = grant_for(SKILL_REMOVE_CAPABILITY_ID);
        assert_eq!(
            skill_remove_grant.constraints.allowed_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem
            ]
        );
        assert_eq!(skill_remove_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_remove_grant.constraints.network,
            NetworkPolicy::default()
        );
        assert!(
            !grants
                .grants
                .iter()
                .any(|grant| { grant.capability.as_str() == SKILL_ACTIVATE_CAPABILITY_ID }),
            "skill activation is a capability-host synthetic capability, not a host-runtime grant"
        );
    }

    #[tokio::test]
    async fn standalone_skill_activate_tool_loads_selected_skill_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-skill-activate-owner",
                storage_root.clone(),
            ),
        )
        .await
        .expect("standalone services build");
        // Seeded into the DATABASE, which is where the runtime reads skills. A disk-seeded skill is
        // correctly invisible now, so seeding to disk would make this test pass on nothing
        // (nearai/ironclaw#7168).
        crate::filesystem_assembly::write_database_file_for_test(
            &storage_root,
            "/tenants/tenant-skill-activate-tool/users/skill-activate-user/skills/unit-activate-helper/SKILL.md",
            skill_md(
                "unit-activate-helper",
                "Unit activation helper",
                "UNIT_ACTIVATE_SENTINEL",
            )
            .as_bytes(),
        )
        .await;
        let runtime = services.host_runtime.clone();
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let mut run_context = run_context("skill-activate-tool").await;
        run_context = run_context
            .with_accepted_message_ref(
                AcceptedMessageRef::new("msg:skill-activate-tool").expect("message ref"),
            )
            .with_actor(TurnActor::new(
                UserId::new("skill-activate-user").expect("user id"),
            ));
        let skill_context =
            filesystem_skill_context_source(runtime_surfaces, &run_context.scope.tenant_id, false)
                .expect("skill context source");
        let activation_source = skill_context.activation_source;
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("skill-activate-user").expect("user id"),
            policy,
            workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: Some(Arc::clone(&activation_source)),
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let descriptor = surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.capability_id.as_str() == SKILL_ACTIVATE_CAPABILITY_ID)
            .expect("skill_activate descriptor");
        assert!(descriptor.provider.is_none());
        assert!(
            descriptor
                .safe_description
                .contains("When the task at hand is one a listed skill covers, call this FIRST"),
            "skill_activate description must tell the model when to use the capability"
        );
        // The clause that actually moved the metric. With only a statement of what the tool
        // does, the model solved tasks with `shell` and never activated: measured 0% correct
        // activation over a 227-skill catalog, with refusals at 0% -- it was not blocked, it
        // had no reason to ask. Telling it that a skill SUPERSEDES its own plan took that to
        // 50%, matching claude-code's precision exactly.
        assert!(
            descriptor
                .safe_description
                .contains("instead of your own default approach"),
            "skill_activate description must say a skill replaces the model's default approach"
        );
        assert!(
            descriptor
                .safe_description
                .contains("An ambiguous name fails without loading anything"),
            "skill_activate description must not imply every visible bare name is actionable"
        );
        // Per-skill relevance gate. Telling the model to activate FIRST lifts activation and
        // over-reach together: measured, the ported build activated `docx` for a task whose only
        // deliverable is an .xlsx file. The old guard said "do not activate skills unrelated to
        // the task", which is too vague to stop an adjacent guess. This makes the test concrete
        // and evidence-based -- does the task EXPLICITLY involve what the description names --
        // and it gates each skill individually rather than capping the set size, which is what
        // the reverted "smallest relevant set" wording did wrong.
        assert!(
            descriptor
                .safe_description
                .contains("only when the task EXPLICITLY involves what its description names"),
            "skill_activate description must gate each skill on explicit task relevance"
        );
        assert!(
            descriptor
                .safe_description
                .contains("at most eight active per run"),
            "skill_activate description must advertise the selector's activation limit"
        );
        // One skill per call, which is claude-code's `Skill` tool shape. The array form invited
        // over-reach: measured over 29 runs, single-skill calls were 12 correct and 0 wrong while
        // multi-skill calls were 10 correct and 1 wrong -- every wrong activation came from a
        // submitted list. Several skills stay reachable by calling again, so this bounds
        // commitment per call, not the total.
        assert!(
            descriptor.safe_description.contains("one skill per call"),
            "skill_activate must ask for one skill per call, as claude-code's Skill tool does"
        );
        assert_eq!(
            descriptor
                .parameters_schema
                .get("properties")
                .and_then(|p| p.get("skill"))
                .and_then(|sk| sk.get("type"))
                .and_then(serde_json::Value::as_str),
            Some("string"),
            "the advertised input must be a single skill name, not an array"
        );
        // `names` must NOT be advertised. `parse_skill_activate_names` still ACCEPTS a legacy
        // `names` array so an in-flight caller or a recorded trace does not hard-fail, but
        // advertising it is what invited the multi-skill calls the measurement above counted.
        // This assertion previously required the opposite and contradicted the `skill`-is-a-string
        // one directly above it -- a leftover from before the schema was narrowed.
        assert!(
            descriptor
                .parameters_schema
                .get("properties")
                .and_then(|properties| properties.get("names"))
                .is_none(),
            "a legacy `names` array is accepted but must not be advertised, or the model is \
             invited back into the shape that produced every wrong activation"
        );
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id.as_str() == SKILL_ACTIVATE_CAPABILITY_ID)
            .expect("skill_activate tool definition");
        assert_eq!(tool_definition.description, descriptor.safe_description);
        let call = ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn-skill-activate".to_string()),
            id: "call-skill-activate".to_string(),
            name: tool_definition.name,
            arguments: serde_json::json!({"names": ["unit-activate-helper"]}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect("provider call stages");
        assert_eq!(
            candidate.capability_id.as_str(),
            SKILL_ACTIVATE_CAPABILITY_ID
        );
        let outcome = port
            .invoke_capability(invocation_for_candidate(&candidate))
            .await
            .expect("skill activation invokes");
        assert!(matches!(outcome, Resolution::Done(_)));

        let selected = activation_source
            .load_skill_context_candidates(&run_context)
            .await
            .expect("selected skill context loads");
        // Default injection mode is `listing`: the activated skill's full body
        // loads, and every other visible skill (the bundled system skills)
        // collapses into one `available-skills` one-line listing candidate.
        assert!(
            selected.iter().any(|candidate| {
                candidate
                    .loaded_skill_md()
                    .is_some_and(|skill_md| skill_md.contains("UNIT_ACTIVATE_SENTINEL"))
            }),
            "activated skill body must load into context"
        );
        let listing = selected
            .iter()
            .filter_map(|candidate| candidate.discoverable_metadata())
            .find(|(name, _)| *name == "available-skills")
            .map(|(_, listing)| listing.to_string())
            .expect("available-skills listing candidate");
        assert!(
            !listing.contains("UNIT_ACTIVATE_SENTINEL"),
            "non-activated listing must not carry skill bodies"
        );
        assert!(
            listing.contains("builtin.skill_activate"),
            "listing header must point at skill_activate"
        );
    }

    #[tokio::test]
    async fn capability_wiring_with_skill_activation_source_exposes_skill_activate_capability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-skill-activate-wiring-owner",
                storage_root.clone(),
            ),
        )
        .await
        .expect("standalone services build");
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let run_context = run_context("skill-activate-wiring").await;
        let skill_context =
            filesystem_skill_context_source(runtime_surfaces, &run_context.scope.tenant_id, false)
                .expect("skill context source");
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new("skill-activate-wiring-user").expect("user id"),
            policy,
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            Some(skill_context.activation_source),
            None,
            None,
            None,
            true,
        )
        .expect("capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");

        assert!(
            surface
                .descriptors
                .iter()
            .any(|descriptor| descriptor.capability_id.as_str() == SKILL_ACTIVATE_CAPABILITY_ID)
        );
    }

    #[tokio::test]
    async fn standalone_external_tools_are_advertised_as_provider_tool_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-external-tool-owner",
                storage_root,
            ),
        )
        .await
        .expect("standalone services build");
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let runtime = services.host_runtime.clone();
        let run_context = run_context("external-tool-provider-name").await;
        let catalog = Arc::new(ironclaw_turns::InMemoryExternalToolCatalog::new());
        catalog
            .register(
                run_context.run_id,
                vec![
                    ironclaw_turns::ExternalToolSpec::new(
                        "client_lookup",
                        "Look up client-side data",
                        serde_json::json!({
                            "type": "object",
                            "properties": {
                                "query": { "type": "string" }
                            }
                        }),
                    )
                    .expect("external tool spec"),
                ],
            )
            .await
            .expect("external tool catalog registers");
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("external-tool-provider-name-user").expect("user id"),
            policy,
            workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: catalog,
            unavailable_capability_ids: HashSet::new(),
        };
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.name.as_str() == "client_lookup")
            .expect("external tool definition");

        assert_eq!(
            tool_definition.capability_id.as_str(),
            "external_tool.client_lookup"
        );

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    tool_definition.name.as_str(),
                    serde_json::json!({"query": "status"}),
                ),
            ))
            .await
            .expect("external provider tool call stages");

        assert_eq!(
            candidate.capability_id.as_str(),
            "external_tool.client_lookup"
        );
    }

    #[tokio::test]
    async fn standalone_project_create_tool_persists_project_visible_to_owner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-project-create-owner",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");
        let runtime = services.host_runtime.clone();
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("project-create-fallback-user").expect("user id"),
            policy: Arc::clone(runtime_surfaces.capability_policy_for_test()),
            workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            project_service: Arc::clone(&runtime_surfaces.project_service),
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };

        let tenant_id = TenantId::new("tenant-project-create").expect("tenant id");
        let owner_user_id = UserId::new("project-create-owner").expect("user id");
        let run_context = run_context_with_scope(TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(AgentId::new("agent-project-create").expect("agent id")),
            Some(ProjectId::new("project-project-create").expect("project id")),
            ThreadId::new("thread-project-create").expect("thread id"),
            Some(owner_user_id.clone()),
        ))
        .await;

        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id.as_str()
                    == PROJECT_CREATE_CAPABILITY_ID),
            "project_create should be an exposed synthetic capability"
        );

        // The name deliberately contains payload/path delimiters (`/ < >`), which
        // are valid in a project name but forbidden in a tool-result safe summary.
        // A summary that interpolated the raw name would fail validation in
        // `append_capability_result_ref` and terminate the whole run; this locks
        // that regression — the capability must still complete.
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__project_create",
                    serde_json::json!({
                        "name": "Build /api <svc>",
                        "description": "Ship the project feature"
                    }),
                ),
            ))
            .await
            .expect("project_create call stages");
        let outcome = port
            .invoke_capability(invocation_for_candidate(&candidate))
            .await
            .expect("project_create invokes");
        let done = match outcome {
            Resolution::Done(done) => done,
            other => panic!("project_create should complete, got {other:?}"),
        };
        // The executor passes this safe summary to `append_capability_result_ref`,
        // which validates it through `LoopSafeSummary`/`ToolResultSafeSummary`
        // before writing the result ref; an unsafe summary there is mapped to a
        // terminal `HostUnavailable` that kills the whole run. Re-run that exact
        // validation here so a summary that interpolated the delimiter-bearing
        // project name (the regression) fails this test.
        ironclaw_loop_contracts::LoopSafeSummary::new(done.summary.as_str().to_string())
            .expect("capability safe summary must pass result-ref validation");
        let result_ref = completed_loop_result_ref(&done);
        let output = capability_io
            .result_output(result_ref.as_str())
            .expect("result read succeeds")
            .expect("result output exists");
        assert_eq!(output["name"], "Build /api <svc>");
        assert!(
            output["project_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "tool output should carry the new project id"
        );

        // The capability writes a real control-plane entity, not a workspace
        // file: the owner can now see the project through the same
        // access-controlled `ProjectService` facade the WebUI lists from.
        let listed = runtime_surfaces
            .project_service
            .list_projects(
                ironclaw_assistant::ProjectCaller {
                    tenant_id: tenant_id.clone(),
                    user_id: owner_user_id.clone(),
                },
                ironclaw_assistant::RebornListProjectsRequest { limit: None },
            )
            .await
            .expect("list projects for owner");
        assert!(
            listed
                .projects
                .iter()
                .any(|project| project.name == "Build /api <svc>"),
            "agent-created project must be visible to its owner"
        );
    }

    #[tokio::test]

    async fn standalone_outbound_delivery_targets_list_uses_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-outbound-delivery-owner",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");
        let runtime = services.host_runtime.clone();
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let slack_target_id =
            RebornOutboundDeliveryTargetId::new("slack:test-dm").expect("target id");
        let slack_target_summary = OutboundDeliveryTargetSummary::new(
            OutboundDeliveryTargetId::new(slack_target_id.as_str()).expect("target id"),
            "slack",
            "Slack DM",
            Some("Personal Slack direct message".to_string()),
        )
        .expect("target summary");
        let slack_target_capabilities = DeliveryTargetCapabilities {
            final_replies: true,
            progress: false,
            gate_prompts: false,
            auth_prompts: false,
            notifications: true,
            modalities: Vec::new(),
        };
        let slack_reply_target =
            ReplyTargetBindingRef::new("reply:test:slack-dm").expect("reply target");
        let slack_provider = Arc::new(StaticOutboundDeliveryTargetProvider::new(
            OutboundDeliveryTargetEntry {
                summary: slack_target_summary,
                capabilities: slack_target_capabilities,
                destination: slack_reply_target.clone(),
                // Overwritten with the querying caller at list-time.
                owner: OutboundDeliveryTargetOwner::new(
                    TenantId::new("tenant-outbound-delivery").expect("tenant id"),
                    UserId::new("outbound-delivery-owner").expect("user id"),
                ),
            },
        ));
        let slack_provider_delegate: Arc<dyn OutboundDeliveryTargetProvider> =
            slack_provider.clone();
        let target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
            Arc::new(OutboundDeliveryTargetRegistry::new(vec![
                slack_provider_delegate,
            ]));
        let outbound_preferences_service: Arc<dyn OutboundPreferencesProductService> =
            Arc::new(RebornOutboundPreferencesService::new(
                Arc::clone(runtime_surfaces.outbound_preferences_for_test()),
                target_provider,
            ));
        let policy = Arc::clone(runtime_surfaces.capability_policy_for_test());
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let fallback_user_id = UserId::new("outbound-delivery-fallback-user").expect("user id");
        let tool_permission_overrides: Arc<
            dyn ironclaw_approvals::CapabilityPermissionOverrideStorePort,
        > = runtime_surfaces
            .tool_permission_overrides_for_test()
            .clone();
        let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort> =
            runtime_surfaces.auto_approve_settings_for_test().clone();
        let approval_settings = Arc::new(
            crate::capability_authorization::StoreApprovalSettingsProvider::new(
                tool_permission_overrides,
                auto_approve_settings,
                runtime_surfaces
                    .persistent_approval_policies_for_test()
                    .clone(),
            ),
        );
        // The durable gate-record store this factory wires. Its raise-path save
        // (§5.3 Stage 0, keyed by the canonical `GateRef::for_approval_request`)
        // is asserted at the integration tier by
        // `notification_channels_set_approval_gate_approve_applies_channels`;
        // this test only needs the store present so the port builds.
        let gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort> =
            Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            ));
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: fallback_user_id.clone(),
            policy,
            workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: Some(outbound_preferences_service),
            outbound_preference_write_requires_approval: true,
            approval_settings,
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::clone(&gate_record_store),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };

        // owner == actor since the ephemeral-per-ping remodel: one run user.
        let owner_user_id = UserId::new("outbound-delivery-user").expect("user id");
        let actor_user_id = owner_user_id.clone();
        let run_context = run_context_with_scope(TurnScope::new_with_owner(
            TenantId::new("tenant-outbound-delivery").expect("tenant id"),
            Some(AgentId::new("agent-outbound-delivery").expect("agent id")),
            Some(ProjectId::new("project-outbound-delivery").expect("project id")),
            ThreadId::new("thread-outbound-delivery").expect("thread id"),
            Some(owner_user_id.clone()),
        ))
        .await
        .with_actor(TurnActor::new(actor_user_id.clone()));
        let expected_provider_caller =
            // owner == actor since the ephemeral-per-ping remodel, so the
            // outbound capabilities resolve as the single run user.
            expected_outbound_delivery_caller(&run_context, actor_user_id.clone());
        slack_provider.expect_caller(expected_provider_caller.clone());
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let descriptor_ids = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<Vec<_>>();
        assert!(descriptor_ids.contains(&OUTBOUND_DELIVER_CAPABILITY_ID));
        assert!(descriptor_ids.contains(&OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID));
        assert!(descriptor_ids.contains(&OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID));
        let tool_definitions = port.tool_definitions().expect("tool definitions");
        let tool_definition_names = tool_definitions
            .iter()
            .map(|definition| definition.name.as_str().to_string())
            .collect::<Vec<_>>();
        assert!(tool_definition_names.contains(&"builtin__outbound_deliver".to_string()));
        assert!(
            tool_definition_names.contains(&"builtin__outbound_delivery_targets_list".to_string())
        );
        assert!(tool_definition_names.contains(&"builtin__notification_channels_set".to_string()));
        let list_tool = tool_definitions
            .iter()
            .find(|definition| {
                definition.name.as_str() == "builtin__outbound_delivery_targets_list"
            })
            .expect("list tool definition should exist");
        assert!(
            list_tool
                .description
                .contains("before builtin__outbound_deliver"),
            "list tool description should steer delivery requests before delivering"
        );
        assert!(
            list_tool.description.contains("cannot read conversations"),
            "list tool description must distinguish delivery routing from integration reads"
        );
        assert!(
            list_tool
                .description
                .contains("corresponding integration's read capabilities"),
            "list tool description must route reads through the owning integration"
        );
        let malformed_list = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__outbound_delivery_targets_list",
                    serde_json::Value::Null,
                ),
            ))
            .await
            .expect_err("malformed list input should fail validation");
        assert_eq!(
            malformed_list.kind,
            AgentLoopHostErrorKind::InvalidInvocation
        );

        let list_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__outbound_delivery_targets_list",
                    serde_json::json!({ "channel": "slack" }),
                ),
            ))
            .await
            .expect("list call stages");
        let list_outcome = port
            .invoke_capability(invocation_for_candidate(&list_candidate))
            .await
            .expect("list call invokes");
        let list_result_ref = match list_outcome {
            Resolution::Done(done) => completed_loop_result_ref(&done),
            other => panic!("list should complete, got {other:?}"),
        };
        let list_output = capability_io
            .result_output(list_result_ref.as_str())
            .expect("result read succeeds")
            .expect("result output exists");
        assert_eq!(
            list_output["targets"][0]["target"]["target_id"],
            slack_target_id.as_str()
        );
        assert_eq!(list_output["targets"][0]["target"]["channel"], "slack");
        assert_eq!(
            slack_provider.observed_callers(),
            vec![expected_provider_caller.clone()]
        );

        let observed_provider_callers = slack_provider.observed_callers();
        assert!(
            observed_provider_callers
                .iter()
                .all(|caller| caller == &expected_provider_caller),
            "outbound target provider should be scoped to the run-user caller: {observed_provider_callers:?}"
        );
        assert!(
            !observed_provider_callers.is_empty(),
            "list target resolution should call the outbound target provider"
        );
    }

    #[tokio::test]
    async fn standalone_yolo_notification_channels_set_bypasses_approval_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "local-yolo-outbound-delivery-owner",
                dir.path().join("standalone"),
            )
            .with_runtime_policy(local_host_minimal_approval_policy()),
        )
        .await
        .expect("standalone-unrestricted services build");
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let slack_target_id =
            RebornOutboundDeliveryTargetId::new("slack:yolo-dm").expect("target id");
        let slack_target_summary = OutboundDeliveryTargetSummary::new(
            OutboundDeliveryTargetId::new(slack_target_id.as_str()).expect("target id"),
            "slack",
            "Slack DM",
            Some("Personal Slack direct message".to_string()),
        )
        .expect("target summary");
        let slack_reply_target =
            ReplyTargetBindingRef::new("reply:test:yolo-slack-dm").expect("reply target");
        let slack_provider = Arc::new(StaticOutboundDeliveryTargetProvider::new(
            OutboundDeliveryTargetEntry {
                summary: slack_target_summary,
                capabilities: DeliveryTargetCapabilities {
                    final_replies: true,
                    progress: false,
                    gate_prompts: false,
                    auth_prompts: false,
                    notifications: true,
                    modalities: Vec::new(),
                },
                destination: slack_reply_target.clone(),
                // Overwritten with the querying caller at list-time.
                owner: OutboundDeliveryTargetOwner::new(
                    TenantId::new("tenant-outbound-delivery").expect("tenant id"),
                    UserId::new("outbound-delivery-owner").expect("user id"),
                ),
            },
        ));
        let slack_provider_delegate: Arc<dyn OutboundDeliveryTargetProvider> =
            slack_provider.clone();
        let target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
            Arc::new(OutboundDeliveryTargetRegistry::new(vec![
                slack_provider_delegate,
            ]));
        let outbound_preferences_service: Arc<dyn OutboundPreferencesProductService> =
            Arc::new(RebornOutboundPreferencesService::new(
                Arc::clone(runtime_surfaces.outbound_preferences_for_test()),
                target_provider,
            ));
        // owner == actor since the ephemeral-per-ping remodel: one run user.
        let owner_user_id = UserId::new("local-yolo-outbound-user").expect("user id");
        let actor_user_id = owner_user_id.clone();
        let run_context = run_context_with_scope(TurnScope::new_with_owner(
            TenantId::new("tenant-local-yolo-outbound").expect("tenant id"),
            Some(AgentId::new("agent-local-yolo-outbound").expect("agent id")),
            Some(ProjectId::new("project-local-yolo-outbound").expect("project id")),
            ThreadId::new("thread-local-yolo-outbound").expect("thread id"),
            Some(owner_user_id.clone()),
        ))
        .await
        .with_actor(TurnActor::new(actor_user_id.clone()));
        let expected_provider_caller =
            // owner == actor since the ephemeral-per-ping remodel, so the
            // outbound capabilities resolve as the single run user.
            expected_outbound_delivery_caller(&run_context, actor_user_id.clone());
        slack_provider.expect_caller(expected_provider_caller.clone());
        let fallback_user_id = UserId::new("local-yolo-outbound-fallback").expect("user id");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        ensure_thread_for_run(thread_service.as_ref(), &run_context, &fallback_user_id).await;
        let wiring = capability_wiring(
            &services,
            thread_service,
            fallback_user_id,
            Arc::clone(runtime_surfaces.capability_policy_for_test()),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            Some(outbound_preferences_service),
            None,
            None,
            true,
        )
        .expect("capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");

        let actor_preference_key = CommunicationPreferenceKey::personal(
            run_context.scope.tenant_id.clone(),
            actor_user_id.clone(),
        );
        let missing_target_id =
            RebornOutboundDeliveryTargetId::new("slack:missing-dm").expect("target id");
        let missing_set_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__notification_channels_set",
                    serde_json::json!({ "target_ids": [missing_target_id.as_str()] }),
                ),
            ))
            .await
            .expect("missing-target set call stages");
        let missing_set_outcome = port
            .invoke_capability(invocation_for_candidate(&missing_set_candidate))
            .await
            .expect("missing-target set call returns a capability outcome");
        match missing_set_outcome {
            Resolution::Done(failure) => {
                // Missing target routes through `outbound_delivery_outcome`
                // (recoverable, model-visible InvalidInput); the disposition
                // function gives a fixed, host-authored summary naming the
                // operation the model can correct — the notification-channel
                // set, not the retired delivery-target write.
                assert_eq!(
                    failure.verdict.error_kind(),
                    Some(&FailureKind::InputEncode)
                );
                assert_eq!(
                    failure.summary.as_str(),
                    "invalid notification channel request"
                );
            }
            other => panic!("missing target should fail non-terminally, got {other:?}"),
        }
        assert!(
            runtime_surfaces
                .outbound_preferences_for_test()
                .load_communication_preference(actor_preference_key.clone())
                .await
                .expect("run-user preference read after missing-target set")
                .is_none()
        );

        let set_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__notification_channels_set",
                    serde_json::json!({ "target_ids": [slack_target_id.as_str()] }),
                ),
            ))
            .await
            .expect("set call stages");
        let set_outcome = port
            .invoke_capability(invocation_for_candidate(&set_candidate))
            .await
            .expect("set call invokes");
        assert!(
            matches!(set_outcome, Resolution::Done(_)),
            "standalone-unrestricted should bypass approval gate, got {set_outcome:?}"
        );
        let observed_provider_callers = slack_provider.observed_callers();
        assert!(
            !observed_provider_callers.is_empty(),
            "set target should resolve through the outbound target provider"
        );
        assert!(
            observed_provider_callers
                .iter()
                .all(|caller| caller == &expected_provider_caller),
            "outbound target provider should be scoped to the run-user caller: {observed_provider_callers:?}"
        );
        let run_preference = runtime_surfaces
            .outbound_preferences_for_test()
            .load_communication_preference(actor_preference_key)
            .await
            .expect("run-user preference read after direct set")
            .expect("run-user preference persisted");
        // The bypassed-gate dispatch writes the notification-channel set, not a
        // final-reply route: `notification_channels_set` replaces the whole set.
        assert_eq!(
            run_preference
                .record
                .notification_targets
                .iter()
                .map(|target| target.as_str())
                .collect::<Vec<_>>(),
            vec![slack_target_id.as_str()]
        );
        // Note: the retired owner-does-not-see-it isolation assertion is gone —
        // owner == actor since the ephemeral-per-ping remodel, so there is no
        // separate owner key to prove empty.
    }

    /// The full `builtin.notification_channels_set` approval-gate dance —
    /// raise → replay payload → user approve (store + lease mint) → approved
    /// resume → lease claim → dispatch → lease consume — on an ordinary run.
    /// Since the ephemeral-per-ping remodel a run has a single user (owner ==
    /// actor), so the value here is that the raise and resume halves agree on
    /// the scope, not any owner-vs-actor split.
    ///
    /// Two properties are pinned:
    ///
    /// 1. **Raise and resume derive the same scope.** Every store the dance
    ///    touches (approval request, replay payload, gate record, lease) is
    ///    scope-keyed; if the raise persists under one identity and the resume
    ///    recomputes another, the resume finds nothing and the approved
    ///    capability never runs. This test drives both halves through the real
    ///    port, so any half-unified derivation change fails it.
    /// 2. **Whose identity that scope carries.** Deliberately asserted so a
    ///    derivation change is a recorded decision, not drift — and that the
    ///    gate is isolated from unrelated identities.
    ///
    /// Tier note: this lives at the capability-host tier rather than
    /// `tests/integration/` so it can drive the raise and resume halves
    /// directly against the real port and pin that both derive the gate scope
    /// the SAME way. The approve-applies-channels flow stays covered end-to-end
    /// at the integration tier
    /// (`outbound_target.rs::notification_channels_set_approval_gate_approve_applies_channels`).
    #[tokio::test]
    async fn notification_channels_set_approval_raise_and_resume_stay_scope_matched() {
        use ironclaw_approvals::ApprovalRequestStorePort as _;
        use ironclaw_authorization::CapabilityLeaseStorePort as _;

        let dir = tempfile::tempdir().expect("tempdir");
        // Default standalone policy — NOT the yolo/minimal override the
        // sibling test uses — so the ExternalWrite effect requires approval
        // and the first invoke raises a real gate.
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "local-outbound-gate-scope",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let slack_target_id =
            RebornOutboundDeliveryTargetId::new("slack:gate-scope-dm").expect("target id");
        let slack_target_summary = OutboundDeliveryTargetSummary::new(
            OutboundDeliveryTargetId::new(slack_target_id.as_str()).expect("target id"),
            "slack",
            "Slack DM",
            Some("Personal Slack direct message".to_string()),
        )
        .expect("target summary");
        let slack_reply_target =
            ReplyTargetBindingRef::new("reply:test:gate-scope-dm").expect("reply target");
        let slack_provider = Arc::new(StaticOutboundDeliveryTargetProvider::new(
            OutboundDeliveryTargetEntry {
                summary: slack_target_summary,
                capabilities: DeliveryTargetCapabilities {
                    final_replies: true,
                    progress: false,
                    gate_prompts: false,
                    auth_prompts: false,
                    notifications: true,
                    modalities: Vec::new(),
                },
                destination: slack_reply_target,
                // Overwritten with the querying caller at list-time.
                owner: OutboundDeliveryTargetOwner::new(
                    TenantId::new("tenant-gate-scope").expect("tenant id"),
                    UserId::new("gate-scope-placeholder").expect("user id"),
                ),
            },
        ));
        let slack_provider_delegate: Arc<dyn OutboundDeliveryTargetProvider> =
            slack_provider.clone();
        let target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
            Arc::new(OutboundDeliveryTargetRegistry::new(vec![
                slack_provider_delegate,
            ]));
        let outbound_preferences_service: Arc<dyn OutboundPreferencesProductService> =
            Arc::new(RebornOutboundPreferencesService::new(
                Arc::clone(runtime_surfaces.outbound_preferences_for_test()),
                target_provider,
            ));
        // Owner == actor since the ephemeral-per-ping remodel: one run user,
        // bound as both the scope owner and the actor. `other_user_id` is an
        // unrelated identity, used only to prove the raised gate is isolated
        // from users it was not raised for.
        let owner_user_id = UserId::new("gate-scope-user").expect("user id");
        let actor_user_id = owner_user_id.clone();
        let other_user_id = UserId::new("gate-scope-other").expect("user id");
        let fallback_user_id = UserId::new("gate-scope-fallback").expect("user id");
        let run_context = run_context_with_scope(TurnScope::new_with_owner(
            TenantId::new("tenant-gate-scope").expect("tenant id"),
            Some(AgentId::new("agent-gate-scope").expect("agent id")),
            Some(ProjectId::new("project-gate-scope").expect("project id")),
            ThreadId::new("thread-gate-scope").expect("thread id"),
            Some(owner_user_id.clone()),
        ))
        .await
        .with_actor(TurnActor::new(actor_user_id.clone()));
        // The authorization identity is the run user: the target provider must
        // be queried as that user on both the raise-side validation and the
        // post-approval dispatch.
        let expected_provider_caller =
            expected_outbound_delivery_caller(&run_context, actor_user_id.clone());
        slack_provider.expect_caller(expected_provider_caller.clone());
        // Local-dev defaults global auto-approve ON, which would bypass the
        // gate. Disable it for the run user (whom the settings-scope derivation
        // follows) and the unrelated `other_user_id`, so the gate raises and
        // the isolation check below is not confounded by a stray auto-approve.
        for settings_user in [&owner_user_id, &other_user_id] {
            let mut settings_scope = run_context.scope.to_resource_scope();
            settings_scope.user_id = (*settings_user).clone();
            ironclaw_approvals::AutoApproveSettingStorePort::set(
                runtime_surfaces.auto_approve_settings_for_test().as_ref(),
                ironclaw_approvals::AutoApproveSettingInput {
                    updated_by: ironclaw_host_api::scope::Principal::User((*settings_user).clone()),
                    scope: settings_scope,
                    enabled: false,
                },
            )
            .await
            .expect("disabling global auto-approve should succeed");
        }
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        ensure_thread_for_run(thread_service.as_ref(), &run_context, &fallback_user_id).await;
        let wiring = capability_wiring(
            &services,
            thread_service,
            fallback_user_id.clone(),
            Arc::clone(runtime_surfaces.capability_policy_for_test()),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            Some(outbound_preferences_service),
            None,
            None,
            true,
        )
        .expect("capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");

        let set_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "builtin__notification_channels_set",
                    serde_json::json!({ "target_ids": [slack_target_id.as_str()] }),
                ),
            ))
            .await
            .expect("set call stages");
        let raise_outcome = port
            .invoke_capability(invocation_for_candidate(&set_candidate))
            .await
            .expect("gated set call returns a capability outcome");
        let waypoint = match raise_outcome {
            Resolution::Blocked(ironclaw_host_api::resolution::Blocked::Approval(waypoint)) => {
                waypoint
            }
            other => panic!("default policy must raise an approval gate, got {other:?}"),
        };
        let origin_gate_ref = waypoint
            .origin
            .as_ref()
            .expect("approval waypoint preserves the loop gate ref")
            .as_str()
            .to_string();
        let approval_request_id = origin_gate_ref
            .strip_prefix("gate:approval-")
            .expect("loop gate ref has the approval prefix")
            .parse::<uuid::Uuid>()
            .map(ironclaw_host_api::ids::ApprovalRequestId::from_uuid)
            .expect("approval request id parses");
        let resume_token = ironclaw_loop_contracts::CapabilityResumeToken::new(
            waypoint
                .resume
                .as_ref()
                .expect("approval waypoint carries the resume token")
                .as_str(),
        )
        .expect("resume token converts");
        let raise_invocation_id =
            super::super::outbound_delivery::invocation_id_from_resume_token(&resume_token)
                .expect("resume token encodes the raise invocation id");
        // Recompute the raise scope EXACTLY as the production raise did.
        let raise_scope = super::super::outbound_delivery::resource_scope_for_run(
            &run_context,
            &fallback_user_id,
            raise_invocation_id,
        );
        // PINNED IDENTITY: a run acts as its user, so the approval-gate raise
        // (and therefore the lease) is scoped to that user — who sees and
        // approves the gate. Raise and resume derive this identity the same
        // way; the test drives both halves so a one-sided change fails it.
        assert_eq!(
            raise_scope.user_id, owner_user_id,
            "the approval-gate scope follows the run user"
        );
        let approval_requests = runtime_surfaces.approval_requests_for_test();
        let raise_record = approval_requests
            .get(&raise_scope, approval_request_id)
            .await
            .expect("approval store read succeeds")
            .expect("the raise persisted the approval request under the raise scope");
        assert_eq!(
            raise_record.status,
            ironclaw_approvals::ApprovalStatus::Pending
        );
        let fingerprint = raise_record
            .request
            .invocation_fingerprint
            .clone()
            .expect("the raise fingerprints the invocation");
        // Scope isolation: an unrelated identity must not see the gate.
        let mut other_scope = raise_scope.clone();
        other_scope.user_id = other_user_id.clone();
        assert!(
            approval_requests
                .get(&other_scope, approval_request_id)
                .await
                .expect("approval store read succeeds")
                .is_none(),
            "the gate must be visible only under the identity it was raised for"
        );

        // The user approves: mark the stored request approved and mint the
        // single-use lease FROM THE STORED ROW (its scope, its grantee, its
        // fingerprint) — the same material the production click-approval
        // resolution uses, never a re-derivation.
        approval_requests
            .approve(&raise_scope, approval_request_id)
            .await
            .expect("approval request approves");
        let capability_leases = runtime_surfaces.capability_leases_for_test();
        let lease = ironclaw_authorization::CapabilityLease {
            scope: raise_scope.clone(),
            grant: ironclaw_host_api::capability::CapabilityGrant {
                id: ironclaw_host_api::ids::CapabilityGrantId::new(),
                capability: CapabilityId::new(OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID)
                    .expect("capability id"),
                grantee: raise_record.request.requested_by.clone(),
                issued_by: ironclaw_host_api::scope::Principal::HostRuntime,
                constraints: ironclaw_host_api::capability::GrantConstraints {
                    allowed_effects: vec![EffectKind::ExternalWrite],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
            invocation_fingerprint: Some(fingerprint),
            status: ironclaw_authorization::CapabilityLeaseStatus::Active,
        };
        let lease_id = lease.grant.id;
        capability_leases
            .issue(lease)
            .await
            .expect("approval lease issues");

        // Resume exactly as the executor reconstructs it from the waypoint.
        let resume_request = LoopRequest {
            activity_id: set_candidate.activity_id,
            surface_version: set_candidate.surface_version.clone(),
            capability_id: set_candidate.capability_id.clone(),
            input_ref: set_candidate.input_ref.clone(),
            approval_resume: Some(ironclaw_loop_contracts::CapabilityApprovalResume {
                approval_request_id,
                resume_token,
                correlation_id: ironclaw_host_api::ids::CorrelationId::new(),
                input_ref: set_candidate.input_ref.clone(),
            }),
            auth_resume: None,
        };
        let resume_outcome = port
            .invoke_capability(resume_request)
            .await
            .expect("approved resume returns a capability outcome");
        assert!(
            matches!(resume_outcome, Resolution::Done(_)),
            "an approved resume must complete the set, got {resume_outcome:?}"
        );

        // The applied set persisted under the run user's preference key.
        let actor_preference = runtime_surfaces
            .outbound_preferences_for_test()
            .load_communication_preference(CommunicationPreferenceKey::personal(
                run_context.scope.tenant_id.clone(),
                actor_user_id.clone(),
            ))
            .await
            .expect("run-user preference read")
            .expect("run-user preference persisted after the approved resume");
        assert_eq!(
            actor_preference
                .record
                .notification_targets
                .iter()
                .map(|target| target.as_str())
                .collect::<Vec<_>>(),
            vec![slack_target_id.as_str()]
        );
        // The provider was queried as the run user on every leg.
        let observed_provider_callers = slack_provider.observed_callers();
        assert!(
            !observed_provider_callers.is_empty()
                && observed_provider_callers
                    .iter()
                    .all(|caller| caller == &expected_provider_caller),
            "the outbound target provider must be queried as the run user: {observed_provider_callers:?}"
        );
        // The lease was claimed and consumed under the raise scope: the dance
        // closed on the same identity it opened on.
        let consumed_lease = capability_leases
            .get(&raise_scope, lease_id)
            .await
            .expect("the consumed lease remains readable under the raise scope");
        assert_eq!(
            consumed_lease.status,
            ironclaw_authorization::CapabilityLeaseStatus::Consumed,
            "the single-use approval lease must be consumed by the resumed dispatch"
        );
    }

    #[tokio::test]
    async fn standalone_outbound_delivery_capabilities_hidden_without_provider_facade() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-no-outbound-provider-owner",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");
        let runtime = services.host_runtime.clone();
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io;
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("outbound-delivery-fallback-user").expect("user id"),
            policy,
            workspace_mounts: runtime_surfaces.workspace_mount_policy_for_test().clone(),
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };
        let run_context = run_context("outbound-delivery-hidden")
            .await
            .with_actor(TurnActor::new(
                UserId::new("outbound-delivery-actor").expect("user id"),
            ));
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let descriptor_ids = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<Vec<_>>();

        assert!(!descriptor_ids.contains(&OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID));
        assert!(!descriptor_ids.contains(&OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID));
        let tool_definition_names = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .map(|definition| definition.name.as_str().to_string())
            .collect::<Vec<_>>();
        assert!(
            !tool_definition_names.contains(&"builtin__outbound_delivery_targets_list".to_string())
        );
        assert!(!tool_definition_names.contains(&"builtin__notification_channels_set".to_string()));
    }

    #[tokio::test]
    async fn local_yolo_capability_port_reads_confirmed_host_mount() {
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
        let storage_root = dir.path().join("standalone");
        let workspace_root = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("workspace root"); // safety: test-only setup in #[cfg(test)] module.
        std::fs::write(workspace_root.join("note.txt"), "safe workspace file\n")
            .expect("workspace file"); // safety: test-only setup in #[cfg(test)] module.
        let host_home = dir.path().join("home");
        std::fs::create_dir_all(&host_home).expect("host home"); // safety: test-only setup in #[cfg(test)] module.
        std::fs::write(host_home.join("safe.txt"), "safe host file\n").expect("host file"); // safety: test-only setup in #[cfg(test)] module.
        let raw_host_home = host_home
            .canonicalize()
            .expect("canonical host home")
            .to_string_lossy()
            .into_owned();

        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                "standalone-unrestricted-host-owner",
                storage_root,
            )
            .with_runtime_policy(
                crate::standalone_unrestricted_runtime_policy(true)
                    .expect("local-yolo policy resolves"), // safety: test-only helper in #[cfg(test)] module.
            )
            .with_local_runtime_workspace_root(workspace_root.clone())
            .with_local_runtime_confirmed_host_home_root(host_home.clone()),
        )
        .await
        .expect("standalone-unrestricted services build"); // safety: test-only assertion in #[cfg(test)] module.
        let runtime = services.host_runtime.clone(); // safety: test-only assertion in #[cfg(test)] module.
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
        let workspace_mounts = runtime_surfaces.workspace_mount_policy_for_test().clone();
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("local-yolo-host-user").expect("user id"), // safety: literal test id is valid.
            policy,
            workspace_mounts,
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };
        let run_context = run_context("host-mount-read").await;
        enable_global_auto_approve_for_run(
            &services,
            &run_context,
            &UserId::new("local-yolo-host-user").expect("user id"),
        )
        .await;
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port"); // safety: test-only assertion in #[cfg(test)] module.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface"); // safety: test-only assertion in #[cfg(test)] module.
        for capability_id in [
            CODING_READ_CAPABILITY_ID,
            CODING_WRITE_CAPABILITY_ID,
            CODING_EDIT_CAPABILITY_ID,
            GLOB_CAPABILITY_ID,
            GREP_CAPABILITY_ID,
        ] {
            let descriptor = surface
                .descriptors
                .iter()
                .find(|descriptor| descriptor.capability_id.as_str() == capability_id)
                .unwrap_or_else(|| panic!("{capability_id} descriptor visible"));
            assert!(
                descriptor.safe_description.contains("/host"),
                "{capability_id} description should disclose confirmed host mount: {}",
                descriptor.safe_description
            );
            assert!(
                !descriptor.safe_description.contains(&raw_host_home),
                "model-visible description must not disclose raw host home path"
            );
        }
        // The coding `read`/`write`/`glob`/`grep` engines pin a `path` parameter,
        // so the confirmed host mount is re-disclosed inside its schema
        // description. The coding hashline `edit` engine pins the exact `input`
        // schema — a single hashline-grammar string property with NO `path`
        // parameter — so its schema carries no path description and is asserted
        // by its real shape instead.
        for capability_id in [
            CODING_READ_CAPABILITY_ID,
            CODING_WRITE_CAPABILITY_ID,
            GLOB_CAPABILITY_ID,
            GREP_CAPABILITY_ID,
        ] {
            let descriptor = surface
                .descriptors
                .iter()
                .find(|descriptor| descriptor.capability_id.as_str() == capability_id)
                .unwrap_or_else(|| panic!("{capability_id} descriptor visible"));
            let path_description =
                descriptor.parameters_schema["properties"]["path"]["description"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{capability_id} path description"));
            assert!(
                path_description.contains("/host"),
                "{capability_id} path schema should disclose confirmed host mount: {path_description}"
            );
            assert!(
                !path_description.contains(&raw_host_home),
                "model-visible schema must not disclose raw host home path"
            );
        }
        let edit_descriptor = surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.capability_id.as_str() == CODING_EDIT_CAPABILITY_ID)
            .expect("coding edit descriptor visible");
        assert_eq!(
            edit_descriptor.parameters_schema["properties"]["input"]["type"].as_str(),
            Some("string"),
            "coding edit schema must pin the single hashline `input` property: {}",
            edit_descriptor.parameters_schema
        );
        assert_eq!(
            edit_descriptor.parameters_schema["required"],
            serde_json::json!(["input"]),
            "coding edit schema must require the hashline `input` property: {}",
            edit_descriptor.parameters_schema
        );
        assert!(
            edit_descriptor.parameters_schema["properties"]
                .get("path")
                .is_none(),
            "coding edit schema must not advertise a retired `path` property: {}",
            edit_descriptor.parameters_schema
        );
        assert!(
            surface
                .descriptors
                .iter()
                .all(|descriptor| descriptor.capability_id.as_str() != SHELL_CAPABILITY_ID),
            "legacy shell must not be model-visible"
        );
        let bash_descriptor = surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.capability_id.as_str() == CODING_BASH_CAPABILITY_ID)
            .expect("bash descriptor visible");
        assert_eq!(
            bash_descriptor.parameters_schema["required"],
            serde_json::json!(["command"])
        );
        let tool_definitions = port.tool_definitions().expect("tool definitions");
        for capability_id in [
            CODING_READ_CAPABILITY_ID,
            CODING_WRITE_CAPABILITY_ID,
            CODING_EDIT_CAPABILITY_ID,
            GLOB_CAPABILITY_ID,
            GREP_CAPABILITY_ID,
        ] {
            let tool = tool_definitions
                .iter()
                .find(|definition| definition.capability_id.as_str() == capability_id)
                .unwrap_or_else(|| panic!("{capability_id} tool definition visible"));
            assert!(
                tool.description.contains("/host"),
                "{capability_id} provider tool description should disclose confirmed host mount: {}",
                tool.description
            );
            assert!(
                !tool.description.contains(&raw_host_home),
                "provider-visible tool surface must not disclose raw host home path"
            );
        }
        // Mirrors the capability-descriptor loop above: path-bearing coding tools
        // disclose the confirmed host mount inside their `path` schema
        // description; the hashline `edit` tool pins the exact `input` schema
        // with no `path` property.
        for capability_id in [
            CODING_READ_CAPABILITY_ID,
            CODING_WRITE_CAPABILITY_ID,
            GLOB_CAPABILITY_ID,
            GREP_CAPABILITY_ID,
        ] {
            let tool = tool_definitions
                .iter()
                .find(|definition| definition.capability_id.as_str() == capability_id)
                .unwrap_or_else(|| panic!("{capability_id} tool definition visible"));
            let tool_path_description = tool.parameters["properties"]["path"]["description"]
                .as_str()
                .unwrap_or_else(|| panic!("{capability_id} tool path description"));
            assert!(
                tool_path_description.contains("/host"),
                "{capability_id} provider tool path schema should disclose confirmed host mount: {tool_path_description}"
            );
            assert!(
                !tool_path_description.contains(&raw_host_home),
                "provider-visible tool surface must not disclose raw host home path"
            );
        }
        let edit_tool = tool_definitions
            .iter()
            .find(|definition| definition.capability_id.as_str() == CODING_EDIT_CAPABILITY_ID)
            .expect("coding edit tool definition visible");
        assert_eq!(
            edit_tool.parameters["properties"]["input"]["type"].as_str(),
            Some("string"),
            "coding edit tool schema must pin the single hashline `input` property: {}",
            edit_tool.parameters
        );
        assert_eq!(
            edit_tool.parameters["required"],
            serde_json::json!(["input"]),
            "coding edit tool schema must require the hashline `input` property: {}",
            edit_tool.parameters
        );
        assert!(
            edit_tool.parameters["properties"].get("path").is_none(),
            "coding edit tool schema must not advertise a retired `path` property: {}",
            edit_tool.parameters
        );
        assert!(
            tool_definitions
                .iter()
                .all(|definition| definition.capability_id.as_str() != SHELL_CAPABILITY_ID),
            "legacy shell must not be offered to the model"
        );
        let bash_tool = tool_definitions
            .iter()
            .find(|definition| definition.capability_id.as_str() == CODING_BASH_CAPABILITY_ID)
            .expect("bash tool definition visible");
        assert_eq!(bash_tool.name.as_str(), "bash");
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"path": "/host/safe.txt"})),
            )
            .await
            .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
                surface_version: surface.version.clone(),
                capability_id: CapabilityId::new(CODING_READ_CAPABILITY_ID)
                    .expect("coding read capability id"), // safety: built-in capability id is a valid literal.
                input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("coding read invocation"); // safety: test-only assertion in #[cfg(test)] module.
        let Resolution::Done(completed) = outcome else {
            panic!("expected completed coding read invocation");
        };
        let output = capability_io
            .result_output(&completed_loop_result_ref(&completed))
            .expect("result output lookup") // safety: test-only assertion in #[cfg(test)] module.
            .expect("result output"); // safety: test-only assertion in #[cfg(test)] module.
        assert!(
            output["output"]
                .as_str()
                .is_some_and(|text| text.contains("1:safe host file")),
            "the confirmed host file must be readable through /host, got {output}"
        );

        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"path": "/workspace/note.txt"})),
            )
            .await
            .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
                surface_version: surface.version,
                capability_id: CapabilityId::new(CODING_READ_CAPABILITY_ID)
                    .expect("coding read capability id"), // safety: built-in capability id is a valid literal.
                input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("workspace coding read invocation"); // safety: test-only assertion in #[cfg(test)] module.
        let Resolution::Done(completed) = outcome else {
            panic!("expected completed coding read invocation");
        };
        let output = capability_io
            .result_output(&completed_loop_result_ref(&completed))
            .expect("result output lookup") // safety: test-only assertion in #[cfg(test)] module.
            .expect("result output"); // safety: test-only assertion in #[cfg(test)] module.
        assert!(
            output["output"]
                .as_str()
                .is_some_and(|text| text.contains("1:safe workspace file")),
            "the workspace file must be readable through /workspace, got {output}"
        );
    }

    #[tokio::test]
    async fn capability_port_skill_install_writes_user_skill_root() {
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                "standalone-skill-port-owner",
                storage_root.clone(),
            )
            .with_runtime_policy(local_host_minimal_approval_policy()),
        )
        .await
        .expect("standalone services build"); // safety: test-only assertion in #[cfg(test)] module.
        let runtime = services.host_runtime.clone(); // safety: test-only assertion in #[cfg(test)] module.
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
        let workspace_mounts = runtime_surfaces.workspace_mount_policy_for_test().clone();
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("standalone-skill-port-user").expect("user id"), // safety: literal test id is valid.
            policy,
            workspace_mounts,
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };
        let run_context = run_context("skill-install-write").await;
        enable_global_auto_approve_for_run(
            &services,
            &run_context,
            &UserId::new("standalone-skill-port-user").expect("user id"),
        )
        .await;
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port"); // safety: test-only assertion in #[cfg(test)] module.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface"); // safety: test-only assertion in #[cfg(test)] module.
        let content =
            "---\nname: qa-smoke-skill\ndescription: qa smoke skill\n---\nqa skill loaded\n";
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({ "content": content })),
            )
            .await
            .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.

        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
                surface_version: surface.version,
                capability_id: CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)
                    .expect("skill_install capability id"), // safety: built-in capability id is a valid literal.
                input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("skill_install invocation"); // safety: test-only assertion in #[cfg(test)] module.

        let Resolution::Done(completed) = outcome else {
            panic!("expected completed skill_install invocation, got {outcome:?}");
        };
        let output = capability_io
            .result_output(&completed_loop_result_ref(&completed))
            .expect("result output lookup") // safety: test-only assertion in #[cfg(test)] module.
            .expect("result output"); // safety: test-only assertion in #[cfg(test)] module.
        assert_eq!(output["installed"], serde_json::json!(true));
        // The agent's own in-run skill port must write into the DATABASE, the tree discovery and
        // Settings read. It used to write to the host disk while everything else read the database,
        // so an agent-installed skill was invisible after the turn that created it
        // (nearai/ironclaw#7168).
        assert!(
            crate::filesystem_assembly::database_file_bytes(
                &storage_root,
                "/tenants/tenant-skill-install-write/users/standalone-skill-port-user/skills/qa-smoke-skill/SKILL.md",
            )
            .await
            .is_some(),
            "the agent's skill_install must write into the database-backed skill tree"
        );
        assert!(
            !storage_root
                .join(
                    "tenants/tenant-skill-install-write/users/standalone-skill-port-user/skills/qa-smoke-skill/SKILL.md"
                )
                .exists(),
            "nothing may be left on the host disk: a skill written there is invisible to discovery"
        );
    }

    #[tokio::test]
    async fn capability_port_omits_host_disclosure_without_confirmed_host_mount() {
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
        let storage_root = dir.path().join("standalone");
        let workspace_root = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("workspace root"); // safety: test-only setup in #[cfg(test)] module.
        std::fs::write(workspace_root.join("note.txt"), "hidden workspace file\n")
            .expect("workspace file"); // safety: test-only setup in #[cfg(test)] module.
        let raw_workspace = workspace_root
            .canonicalize()
            .expect("canonical workspace root")
            .to_string_lossy()
            .into_owned();
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-no-host-owner",
                storage_root,
            )
            .with_local_runtime_workspace_root(workspace_root.clone()),
        )
        .await
        .expect("standalone services build"); // safety: test-only assertion in #[cfg(test)] module.
        let runtime = services.host_runtime.clone(); // safety: test-only assertion in #[cfg(test)] module.
        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate"); // safety: test-only assertion in #[cfg(test)] module.
        let workspace_mounts = runtime_surfaces.workspace_mount_policy_for_test().clone();
        let policy = Arc::new(
            crate::builtin_capability_policy::builtin_capability_policy().expect("policy parses"),
        );
        let capability_io = Arc::new(StagedCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let factory = RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id: UserId::new("standalone-no-host-user").expect("user id"), // safety: literal test id is valid.
            policy,
            workspace_mounts,
            memory_mounts: runtime_surfaces.memory_mounts_for_test().clone(),
            system_extensions_lifecycle_mounts: runtime_surfaces
                .system_extensions_lifecycle_mounts_for_test()
                .clone(),
            extension_surface_source: ExtensionCapabilitySurfaceSource::default(),
            input_resolver,
            result_writer,
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            skill_activation_source: None,
            trajectory_observer: None,
            outbound_preferences_service: None,
            outbound_preference_write_requires_approval: false,
            approval_settings: Arc::new(ironclaw_approvals::EmptyApprovalSettingsProvider),
            project_service: Arc::clone(&runtime_surfaces.project_service),
            approval_requests: runtime_surfaces.approval_requests_for_test().clone(),
            capability_leases: runtime_surfaces.capability_leases_for_test().clone(),
            gate_record_store: Arc::new(ironclaw_approvals::GateRecordStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            replay_payload_store: Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
                crate::wrap_scoped(Arc::clone(runtime_surfaces.extension_filesystem_for_test())),
            )),
            external_tool_catalog: std::sync::Arc::new(
                ironclaw_turns::InMemoryExternalToolCatalog::new(),
            ),
            unavailable_capability_ids: HashSet::new(),
        };
        let run_context = run_context("no-host-disclosure").await;
        enable_global_auto_approve_for_run(
            &services,
            &run_context,
            &UserId::new("standalone-no-host-user").expect("user id"),
        )
        .await;
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port"); // safety: test-only assertion in #[cfg(test)] module.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface"); // safety: test-only assertion in #[cfg(test)] module.
        let read_descriptor = surface
            .descriptors
            .iter()
            .find(|descriptor| descriptor.capability_id.as_str() == CODING_READ_CAPABILITY_ID)
            .expect("read descriptor visible");
        // The pinned read docs legitimately mention the `ssh://host/` URL
        // scheme, so a bare "/host" substring is not a disclosure marker.
        // The actual disclosure is the scoped-roots note, which is appended
        // only when the host mount is confirmed.
        assert!(
            !read_descriptor
                .safe_description
                .contains("Available scoped roots")
                && !read_descriptor
                    .safe_description
                    .contains("confirmed host home mount"),
            "normal standalone read description must not disclose host roots: {}",
            read_descriptor.safe_description
        );
        assert!(
            surface
                .descriptors
                .iter()
                .all(|descriptor| descriptor.capability_id.as_str() != SHELL_CAPABILITY_ID),
            "legacy shell must not be model-visible"
        );
        let tool_definitions = port.tool_definitions().expect("tool definitions");
        let read_tool = tool_definitions
            .iter()
            .find(|definition| definition.capability_id.as_str() == CODING_READ_CAPABILITY_ID)
            .expect("read tool definition visible");
        assert!(
            !read_tool.description.contains("Available scoped roots")
                && !read_tool.description.contains("confirmed host home mount"),
            "normal standalone provider tool description must not disclose host roots: {}",
            read_tool.description
        );
        assert!(
            tool_definitions
                .iter()
                .all(|definition| definition.capability_id.as_str() != SHELL_CAPABILITY_ID),
            "legacy shell must not be offered to the model"
        );

        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(
                    serde_json::json!({"path": format!("{raw_workspace}/note.txt")}),
                ),
            )
            .await
            .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.
        let outcome = port
            .invoke_capability(LoopRequest {
                activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
                surface_version: surface.version,
                capability_id: CapabilityId::new(CODING_READ_CAPABILITY_ID)
                    .expect("coding read capability id"), // safety: built-in capability id is a valid literal.
                input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("raw workspace coding read invocation"); // safety: test-only assertion in #[cfg(test)] module.
        match outcome {
            Resolution::Done(failure) => {
                assert_eq!(
                    failure.verdict.error_kind(),
                    Some(&FailureKind::FilesystemDenied)
                );
            }
            other => panic!("expected raw workspace read to be denied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn capability_port_restores_activated_github_extension_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let owner_id = "standalone-github-surface-owner";
        {
            let services = crate::factory::build_runtime_substrate(
                crate::deployment::local_filesystem_build_input(owner_id, storage_root.clone()),
            )
            .await
            .expect("standalone services build");
            let runtime_surfaces = services
                .local_runtime_for_test()
                .expect("local runtime substrate");
            let extension_management = runtime_surfaces.extension_management.clone();
            // #6520 membership: installs are private to their caller, so
            // install AS the surface user whose capability port is asserted
            // below; there is no separate Activate action — the port's
            // prechecked activation publishes the surface directly.
            let surface_user = UserId::new("standalone-github-user").expect("user id");
            let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                .expect("valid github ref");
            extension_management
                .install(package_ref.clone(), &surface_user)
                .await
                .expect("install github extension");
            extension_management
                .activate_with_prechecked_credentials_for_user_for_test(package_ref, &surface_user)
                .await
                .expect("activate github extension");
        }

        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(owner_id, storage_root),
        )
        .await
        .expect("standalone services rebuild");
        let run_context = run_context("github-surface").await;
        let restore_seed_scope = crate::runtime::capability_host::resource_scope_for_run(
            &run_context,
            &UserId::new("standalone-github-user").expect("user id"),
        );
        seed_configured_account_and_secret(&services, &restore_seed_scope, "github").await;
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new("standalone-github-user").expect("user id"),
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");
        assert_github_capabilities_visible(&wiring, &run_context).await;
    }

    #[tokio::test]
    async fn capability_port_refreshes_extensions_after_activation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "standalone-live-github-surface-owner",
                storage_root,
            ),
        )
        .await
        .expect("standalone services build");
        let run_context = run_context("github-live-surface").await;
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new("standalone-live-github-user").expect("user id"),
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let inactive_surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("inactive visible surface");
        let inactive_capability_ids = inactive_surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<Vec<_>>();
        assert!(
            !inactive_capability_ids.contains(&"github.search_issues"),
            "github capability should stay hidden before activation"
        );

        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let extension_management = runtime_surfaces.extension_management.clone();
        // #6520 membership: installs are private to their caller, so install
        // AS the surface user whose capability port is asserted; there is no
        // separate Activate action — prechecked activation publishes directly.
        let surface_user = UserId::new("standalone-live-github-user").expect("user id");
        let seed_scope =
            crate::runtime::capability_host::resource_scope_for_run(&run_context, &surface_user);
        seed_configured_account_and_secret(&services, &seed_scope, "github").await;
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("valid github ref");
        extension_management
            .install(package_ref.clone(), &surface_user)
            .await
            .expect("install github extension");
        extension_management
            .activate_with_prechecked_credentials_for_user_for_test(package_ref, &surface_user)
            .await
            .expect("activate github extension");

        let active_surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("active visible surface");
        let active_capability_ids = active_surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<Vec<_>>();
        assert!(active_capability_ids.contains(&"github.search_issues"));
        assert!(active_capability_ids.contains(&"github.get_issue"));
        assert!(active_capability_ids.contains(&"github.comment_issue"));

        let staged_after_activation = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(
                    "github__search_issues",
                    serde_json::json!({"query": "repo:nearai/ironclaw is:issue"}),
                ),
            ))
            .await
            .expect("provider registration resolves github after prompt-stage refresh");
        assert_eq!(
            staged_after_activation.capability_id.as_str(),
            "github.search_issues"
        );

        let tool_definitions = port.tool_definitions().expect("tool definitions");
        let tool_definition_ids = tool_definitions
            .iter()
            .map(|definition| definition.capability_id.as_str())
            .collect::<Vec<_>>();
        assert!(
            tool_definition_ids.contains(&"github.search_issues"),
            "refreshed provider tools should include github after activation"
        );
    }

    #[tokio::test]
    async fn hosted_sandbox_extension_search_and_registration_use_tenant_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let network = Arc::new(
            ironclaw_extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name(
                "calendar-search",
            ),
        );
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "hosted-sandbox-extension-search-owner",
                dir.path().join("standalone"),
            )
            .with_runtime_policy(
                crate::hosted_single_tenant_volume_sandboxed_runtime_policy()
                    .expect("hosted sandbox runtime policy resolves"),
            )
            .with_runtime_process_binding(crate::RebornRuntimeProcessBinding::user_sandbox(
                Arc::new(ironclaw_host_runtime::UserSandboxProcessPort::new(
                    Arc::new(UnusedSandboxTransport),
                )),
            ))
            .with_network_http_egress_for_test(network),
        )
        .await
        .expect("hosted sandbox services build");
        let run_context = run_context("extension-search-loop-port").await;
        enable_global_auto_approve_for_run(
            &services,
            &run_context,
            &UserId::new("standalone-extension-search-user").expect("user id"),
        )
        .await;
        let fallback_user_id = UserId::new("standalone-extension-search-user").expect("user id");
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        ensure_thread_for_run(thread_service.as_ref(), &run_context, &fallback_user_id).await;
        let wiring = capability_wiring(
            &services,
            thread_service,
            fallback_user_id,
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id.as_str() == EXTENSION_SEARCH_CAPABILITY_ID)
            .expect("extension_search tool definition");

        let extension_ids = ironclaw_extension_support::packages::bundled_packages()
            .iter()
            .map(|package| package.id)
            .collect::<Vec<_>>();
        for (index, extension_id) in extension_ids.into_iter().enumerate() {
            let mut tool_call = provider_tool_call_with_name(
                tool_definition.name.as_str(),
                serde_json::json!({"query": extension_id}),
            );
            tool_call.turn_id = Some(format!("extension-search-turn-{index}"));
            tool_call.id = format!("extension-search-call-{index}");
            let candidate = port
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(tool_call))
                .await
                .expect("extension_search provider tool call stages");
            assert_eq!(
                candidate.capability_id.as_str(),
                EXTENSION_SEARCH_CAPABILITY_ID
            );

            let outcome = port
                .invoke_capability(invocation_for_candidate(&candidate))
                .await
                .expect("extension_search invocation");

            let Resolution::Done(outcome) = outcome else {
                panic!(
                    "extension_search should be authorized to read the system extension catalog"
                );
            };
            let preview = outcome.refs.preview.as_ref().unwrap_or_else(|| {
                panic!("the model must receive the {extension_id} search result inline")
            });
            assert!(
                preview
                    .as_str()
                    .contains(&format!("\"id\":\"{extension_id}\"")),
                "the model-visible result must contain the {extension_id} catalog entry: {preview}"
            );
        }

        let register_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| {
                definition.capability_id.as_str() == EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID
            })
            .expect("extension_register_hosted_mcp tool definition");
        let mut register_call = provider_tool_call_with_name(
            register_definition.name.as_str(),
            serde_json::json!({
                "desired_id": "calendar",
                "desired_name": "Calendar MCP",
                "endpoint": "https://mcp.example.test/rpc",
                "auth_type": "no_auth"
            }),
        );
        register_call.turn_id = Some("hosted-register-turn".to_string());
        register_call.id = "hosted-register-call".to_string();
        let register_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(register_call))
            .await
            .expect("hosted registration tool call stages");
        let register_outcome = port
            .invoke_capability(invocation_for_candidate(&register_candidate))
            .await
            .expect("hosted registration invocation");
        assert!(
            matches!(register_outcome, Resolution::Done(_)),
            "hosted registration should persist through the tenant-workspace mount: {register_outcome:?}"
        );

        let mut read_back_call = provider_tool_call_with_name(
            tool_definition.name.as_str(),
            serde_json::json!({"query": "mcp-calendar"}),
        );
        read_back_call.turn_id = Some("hosted-register-read-back-turn".to_string());
        read_back_call.id = "hosted-register-read-back-call".to_string();
        let read_back_candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(read_back_call))
            .await
            .expect("registration read-back tool call stages");
        let read_back = port
            .invoke_capability(invocation_for_candidate(&read_back_candidate))
            .await
            .expect("registration read-back invocation");
        let Resolution::Done(read_back) = read_back else {
            panic!("registered hosted MCP should be discoverable, got {read_back:?}");
        };
        let preview = read_back
            .refs
            .preview
            .expect("registered hosted MCP is model-visible");
        assert!(
            preview.as_str().contains("\"id\":\"mcp-calendar\""),
            "registration read-back must contain the durable package: {preview}"
        );
    }

    #[tokio::test]
    async fn register_does_not_rebuild_surface_mid_response() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("standalone");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input_with_profile(
                crate::RebornCompositionProfile::StandaloneUnrestricted,
                "standalone-mid-response-owner",
                storage_root,
            )
            .with_runtime_policy(local_host_minimal_approval_policy()),
        )
        .await
        .expect("standalone services build");
        let run_context = run_context("mid-response").await;
        let mid_response_seed_scope = crate::runtime::capability_host::resource_scope_for_run(
            &run_context,
            &UserId::new("standalone-mid-response-user").expect("user id"),
        );
        seed_configured_account_and_secret(&services, &mid_response_seed_scope, "github").await;
        let wiring = capability_wiring(
            &services,
            Arc::new(InMemorySessionThreadService::default()),
            UserId::new("standalone-mid-response-user").expect("user id"),
            Arc::new(
                crate::builtin_capability_policy::builtin_capability_policy()
                    .expect("policy parses"),
            ),
            Arc::new(UnavailableModelGateway),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
            None,
            None,
            None,
            true,
        )
        .expect("standalone capability wiring");
        let port = wiring
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");

        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("prompt-stage surface refresh");

        let mut call1 = provider_tool_call_with_name(
            "read",
            serde_json::json!({"path": "/host/nonexistent.txt"}),
        );
        call1.id = "call-mid-response-1".to_string();
        let candidate1 = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call1))
            .await
            .expect("first register");

        let runtime_surfaces = services
            .local_runtime_for_test()
            .expect("local runtime substrate");
        let extension_management = runtime_surfaces.extension_management.clone();
        // #6520 membership: installs are private to their caller, so install
        // AS the surface user whose capability port is asserted; there is no
        // separate Activate action — prechecked activation publishes directly.
        let surface_user = UserId::new("standalone-mid-response-user").expect("user id");
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("valid github ref");
        extension_management
            .install(package_ref.clone(), &surface_user)
            .await
            .expect("install github extension");
        extension_management
            .activate_with_prechecked_credentials_for_user_for_test(package_ref, &surface_user)
            .await
            .expect("activate github extension");

        let mut call2 =
            provider_tool_call_with_name("read", serde_json::json!({"path": "/host/other.txt"}));
        call2.id = "call-mid-response-2".to_string();
        let candidate2 = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call2))
            .await
            .expect("second register after extension activation");

        assert_eq!(
            candidate1.surface_version, candidate2.surface_version,
            "both candidates must carry the same surface version so invoke_capability_batch can serve them from one snapshot"
        );

        let batch_result = port
            .invoke_capability_batch(ironclaw_loop_contracts::LoopRequestBatch {
                invocations: vec![
                    invocation_for_candidate(&candidate1),
                    invocation_for_candidate(&candidate2),
                ],
                stop_on_first_suspension: false,
            })
            .await;
        if let Err(ref error) = batch_result {
            assert_ne!(
                error.kind,
                ironclaw_loop_contracts::AgentLoopHostErrorKind::StaleSurface,
                "invoke_capability_batch must not fail with StaleSurface: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn capability_port_exposes_activated_gsuite_extensions_to_model() {
        let harness = gsuite_surface_harness(
            "standalone-gsuite-surface-owner",
            "gsuite-surface",
            "standalone-gsuite-surface-user",
            GsuiteExtensionState::Activated,
        )
        .await;

        assert_gsuite_capabilities_visibility(
            &harness.wiring,
            &harness.run_context,
            GsuiteCapabilityVisibility::Visible,
        )
        .await;
    }

    #[tokio::test]
    async fn activated_gmail_provider_tool_call_without_account_returns_oauth_gate() {
        let harness = gsuite_surface_harness(
            "standalone-gmail-auth-owner",
            "gmail-auth-gate",
            "standalone-gmail-auth-user",
            GsuiteExtensionState::Activated,
        )
        .await;
        let port = harness
            .wiring
            .capability_factory
            .create_capability_port(&harness.run_context)
            .await
            .expect("capability port");
        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        let tool_definition = port
            .tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .find(|definition| definition.capability_id.as_str() == "gmail.list_messages")
            .expect("gmail.list_messages tool definition");
        assert_eq!(tool_definition.name.as_str(), "gmail__list_messages");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                provider_tool_call_with_name(tool_definition.name.as_str(), serde_json::json!({})),
            ))
            .await
            .expect("gmail provider tool call stages");

        let outcome = port
            .invoke_capability(invocation_for_candidate(&candidate))
            .await
            .expect("gmail provider tool call invokes");

        let Resolution::Blocked(blocked) = outcome else {
            panic!("expected Gmail provider tool call to return an auth gate, got {outcome:?}");
        };
        assert_eq!(blocked.kind(), "auth");
        // Flip consequence (§5.2.9 collapse, confirmed) — the `credential_requirements`
        // (provider, requester_extension, provider_scopes) collapsed onto the
        // durable `GateRecord::Auth`, not the `Blocked::Auth` resolution channel,
        // so they can no longer be asserted from the returned Resolution here.
        // Re-express against the persisted auth gate record (keyed by the gate ref
        // / recovered request id, as the set-cycle test does for approval records).
    }

    #[tokio::test]
    async fn deactivated_gsuite_extension_capabilities_not_exposed_to_model() {
        let harness = gsuite_surface_harness(
            "standalone-gsuite-inactive-surface-owner",
            "gsuite-inactive-surface",
            "standalone-gsuite-inactive-surface-user",
            GsuiteExtensionState::Installed,
        )
        .await;

        assert_gsuite_capabilities_visibility(
            &harness.wiring,
            &harness.run_context,
            GsuiteCapabilityVisibility::HiddenUntilActivated,
        )
        .await;
        #[cfg(not(feature = "test-support"))]
        {
            let port = harness
                .wiring
                .capability_factory
                .create_capability_port(&harness.run_context)
                .await
                .expect("capability port");
            assert!(
                port.tool_definitions()
                    .expect("tool definitions")
                    .iter()
                    .all(|definition| definition.capability_id.as_str() != "builtin.result_read"),
                "the retired result_read capability must not be model-visible"
            );
        }
    }

    #[tokio::test]
    async fn truncated_result_uses_completed_artifact_without_continuation_offset() {
        let backend = Arc::new(InMemoryBackend::new());
        let artifact_store = Arc::new(
            ironclaw_threads::DurableToolArtifactStore::new(backend)
                .expect("artifact store initializes"),
        );
        let capability_io =
            StagedCapabilityIo::new(Arc::new(CapabilityDisplayPreviewStore::default()));
        let run_context = run_context("large-artifact-result").await;
        let resource_scope = resource_scope_for_run(
            &run_context,
            &UserId::new("large-artifact-owner").expect("owner id"),
        );
        let capability_id = CapabilityId::new("builtin.read").expect("capability id");
        let input_ref = CapabilityInputRef::new("input:large-artifact").expect("input ref");
        let output =
            serde_json::json!({ "output": "x".repeat(ARTIFACT_INLINE_PREVIEW_MAX_BYTES + 1) });
        let serialized = serde_json::to_vec(&output).expect("serialize output");
        let bounded_preview = first_look_result_preview(&serialized)
            .expect("large output has a bounded preview")
            .text;
        let output_bytes = u64::try_from(serialized.len()).expect("test output length");
        let receipt = ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: resource_scope.clone(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default().set_output_bytes(output_bytes)),
        };
        let completed_artifact = artifact_store
            .persist(
                ArtifactWriteMetadata {
                    write_key: None,
                    owner_scope: ArtifactOwnerScope::from_resource_scope(&resource_scope),
                    namespace: run_context.effective_artifact_namespace(),
                    producer_capability_id: capability_id.clone(),
                    content_type: "application/json".to_string(),
                    expected_bytes: Some(output_bytes),
                },
                &serialized,
                &receipt,
            )
            .await
            .expect("runtime artifact persists");

        let written = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: Some(&receipt),
                completed_artifact: Some(&completed_artifact),
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &capability_id,
                output: serde_json::Value::String(bounded_preview),
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("large result writes");

        let observation = written
            .model_observation
            .expect("large result has model observation");
        let artifact_uri = observation
            .artifacts
            .first()
            .expect("artifact metadata")
            .artifact_ref
            .clone();
        assert!(artifact_uri.starts_with("artifact://"));
        let observation_json = serde_json::to_value(&observation).expect("observation serializes");
        let ToolObservationDetail::ArtifactReference {
            artifact_ref,
            preview,
            ..
        } = observation.detail
        else {
            panic!("large result must expose an artifact reference");
        };
        assert_eq!(artifact_ref, artifact_uri);
        assert!(preview.is_some());
        assert!(
            !observation_json["detail"]
                .as_object()
                .expect("detail object")
                .contains_key("next_offset")
        );

        let structured_preview = serde_json::json!({
            "output": "x".repeat(8 * 1024),
            "artifact_ref": artifact_uri,
            "total_bytes": output_bytes,
        });
        let structured_written = capability_io
            .write_capability_result(CapabilityResultWrite {
                receipt: Some(&receipt),
                completed_artifact: Some(&completed_artifact),
                canonical_output_digest: None,
                run_context: &run_context,
                input_ref: &CapabilityInputRef::new("input:structured-artifact")
                    .expect("input ref"),
                invocation_id: InvocationId::new(),
                capability_id: &capability_id,
                output: structured_preview,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
                canonical_item_count: None,
            })
            .await
            .expect("structured artifact preview writes");
        assert!(matches!(
            structured_written
                .model_observation
                .expect("structured preview has model observation")
                .detail,
            ToolObservationDetail::ArtifactReference { .. }
        ));

        let artifact_ref = artifact_uri
            .parse::<ArtifactRef>()
            .expect("artifact URI parses");
        let stored = artifact_store
            .read(ArtifactReadRequest {
                owner_scope: ArtifactOwnerScope::from_resource_scope(&resource_scope),
                namespace: run_context.effective_artifact_namespace(),
                target: ArtifactReadTarget {
                    artifact_id: artifact_ref.id(),
                    selector: ArtifactSelector::Full,
                    max_output_bytes: written.byte_len,
                },
            })
            .await
            .expect("artifact read succeeds")
            .expect("artifact exists");
        assert!(!stored.content.is_empty());
    }
}

// arch-exempt: large_file, pre-existing large file minimally touched for the §5.3 Stage 2a-i replay-payload move (field/store wiring + tests), plan #6175
