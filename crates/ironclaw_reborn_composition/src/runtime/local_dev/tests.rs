#[cfg(test)]
mod tests {
    #![allow(clippy::module_inception)]

    use super::super::*;

    use ironclaw_host_api::{AgentId, MountPermissions, ProjectId, TenantId, ThreadId};
    use ironclaw_threads::{
        EnsureThreadRequest, InMemorySessionThreadService, MessageKind, ThreadHistoryRequest,
    };
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CapabilityInvocation, CapabilityOutcome, InMemoryLoopHostMilestoneSink,
            InMemoryRunProfileResolver, VisibleCapabilityRequest,
        },
    };

    async fn run_context(label: &str) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new(format!("tenant-{label}")).expect("tenant id"),
                Some(AgentId::new(format!("agent-{label}")).expect("agent id")),
                Some(ProjectId::new(format!("project-{label}")).expect("project id")),
                ThreadId::new(format!("thread-{label}")).expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    fn provider_tool_call(arguments: serde_json::Value) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn-1".to_string()),
            id: "call-1".to_string(),
            name: "builtin_echo".to_string(),
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    #[tokio::test]
    async fn capability_io_writes_durable_preview_message_and_live_upsert_id() {
        let run_context = run_context("durable-preview").await;
        let thread_scope = ThreadScope {
            tenant_id: run_context.scope.tenant_id.clone(),
            agent_id: run_context.scope.agent_id.clone().expect("agent id"),
            project_id: run_context.scope.project_id.clone(),
            owner_user_id: None,
            mission_id: None,
        };
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
        let capability_io = LocalDevCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service.clone(),
            thread_scope.clone(),
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();

        let result_ref = capability_io
            .write_capability_result(
                &run_context,
                &input_ref,
                invocation_id,
                &CapabilityId::new("builtin.echo").expect("capability id"),
                serde_json::json!({"content": "hello"}),
            )
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

    #[tokio::test]
    async fn capability_io_fails_result_when_durable_preview_append_fails() {
        let run_context = run_context("durable-preview-failure").await;
        let thread_scope = ThreadScope {
            tenant_id: run_context.scope.tenant_id.clone(),
            agent_id: run_context.scope.agent_id.clone().expect("agent id"),
            project_id: run_context.scope.project_id.clone(),
            owner_user_id: None,
            mission_id: None,
        };
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
        let capability_io = LocalDevCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            thread_service,
            thread_scope,
        );
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"message": "hello"})),
            )
            .await
            .expect("input stages");
        let invocation_id = InvocationId::new();

        let error = capability_io
            .write_capability_result(
                &run_context,
                &input_ref,
                invocation_id,
                &CapabilityId::new("builtin.echo").expect("capability id"),
                serde_json::json!({"content": "hello"}),
            )
            .await
            .expect_err("missing thread rejects durable preview append");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
        let preview_record = display_previews
            .record_for_invocation(invocation_id)
            .expect("live preview record was staged before durable append");
        assert!(preview_record.timeline_message_id.is_none());
    }

    #[tokio::test]
    async fn capability_io_resolves_input_refs_repeatedly() {
        let capability_io = LocalDevCapabilityIo::default();
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
        let capability_io = LocalDevCapabilityIo::default();
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
        store
            .insert_with_oldest_eviction(
                "result:first".to_string(),
                serde_json::Value::String("a".repeat(3 * 1024 * 1024)),
            )
            .expect("first result stages");
        store
            .insert_with_oldest_eviction(
                "result:second".to_string(),
                serde_json::Value::String("b".repeat(2 * 1024 * 1024)),
            )
            .expect("second result stages");

        assert!(store.get("result:first").is_none());
        assert!(store.get("result:second").is_some());
        assert!(store.total_bytes <= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES);
    }

    #[test]
    fn local_dev_builtin_surface_grants_capability_classes() {
        let capability_ids = local_dev_builtin_capability_ids();

        assert!(capability_ids.contains(&WRITE_FILE_CAPABILITY_ID));
        assert!(capability_ids.contains(&APPLY_PATCH_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_LIST_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
        assert!(capability_ids.contains(&SKILL_REMOVE_CAPABILITY_ID));
        assert!(capability_ids.contains(&SHELL_CAPABILITY_ID));
        assert!(capability_ids.contains(&HTTP_CAPABILITY_ID));
        assert_eq!(
            local_dev_allowed_effects(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem
            ]
        );
        assert_eq!(
            local_dev_provider_allowed_effects(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network
            ]
        );

        let workspace_mounts =
            crate::local_dev_mounts::workspace_mount_view(MountPermissions::read_write(), &[])
                .expect("workspace mounts build");
        let skill_mounts =
            crate::local_dev_mounts::skill_management_mount_view().expect("skill mounts build");
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
        let grants = local_dev_builtin_grants(
            &ExtensionId::new("loop-driver").expect("valid extension id"),
            &workspace_mounts,
            &skill_mounts,
        )
        .expect("local-dev grants build");
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
            local_dev_shell_network_policy()
        );

        let http_grant = grant_for(HTTP_CAPABILITY_ID);
        assert_eq!(
            http_grant.constraints.allowed_effects,
            vec![EffectKind::DispatchCapability, EffectKind::Network]
        );
        assert!(http_grant.constraints.mounts.mounts.is_empty());
        assert_eq!(
            http_grant.constraints.network,
            local_dev_shell_network_policy()
        );

        let read_file_grant = grant_for(READ_FILE_CAPABILITY_ID);
        assert_eq!(
            read_file_grant.constraints.allowed_effects,
            local_dev_allowed_effects()
        );
        assert_eq!(read_file_grant.constraints.mounts, workspace_mounts);
        assert_eq!(
            read_file_grant.constraints.network,
            NetworkPolicy::default()
        );

        let skill_install_grant = grant_for(SKILL_INSTALL_CAPABILITY_ID);
        assert_eq!(skill_install_grant.constraints.mounts, skill_mounts);
        assert_eq!(
            skill_install_grant.constraints.network,
            NetworkPolicy::default()
        );
    }

    #[tokio::test]
    async fn local_yolo_capability_port_reads_confirmed_host_mount() {
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only setup in #[cfg(test)] module.
        let storage_root = dir.path().join("local-dev");
        let host_home = dir.path().join("home");
        std::fs::create_dir_all(&host_home).expect("host home"); // safety: test-only setup in #[cfg(test)] module.
        std::fs::write(host_home.join("safe.txt"), "safe host file\n").expect("host file"); // safety: test-only setup in #[cfg(test)] module.

        let services = crate::build_reborn_services(
            crate::RebornBuildInput::local_dev_with_profile(
                crate::RebornCompositionProfile::LocalDevYolo,
                "local-dev-yolo-host-owner",
                storage_root,
            )
            .with_runtime_policy(
                crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"), // safety: test-only helper in #[cfg(test)] module.
            )
            .with_local_dev_confirmed_host_home_root(host_home),
        )
        .await
        .expect("local-dev-yolo services build"); // safety: test-only assertion in #[cfg(test)] module.
        let runtime = services.host_runtime.clone().expect("host runtime"); // safety: test-only assertion in #[cfg(test)] module.
        let workspace_mounts = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate") // safety: test-only assertion in #[cfg(test)] module.
            .workspace_mounts
            .clone();
        let capability_io = Arc::new(LocalDevCapabilityIo::default());
        let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
        let factory = LocalDevLoopCapabilityPortFactory::new(
            runtime,
            UserId::new("local-yolo-host-user").expect("user id"), // safety: literal test id is valid.
            workspace_mounts,
            input_resolver,
            result_writer,
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        );
        let run_context = run_context("host-mount-read").await;
        let port = factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port"); // safety: test-only assertion in #[cfg(test)] module.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface"); // safety: test-only assertion in #[cfg(test)] module.
        let input_ref = capability_io
            .register_provider_tool_call_input(
                &run_context,
                &provider_tool_call(serde_json::json!({"path": "/host/safe.txt"})),
            )
            .await
            .expect("input ref"); // safety: test-only assertion in #[cfg(test)] module.

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: CapabilityId::new(READ_FILE_CAPABILITY_ID)
                    .expect("read_file capability id"), // safety: built-in capability id is a valid literal.
                input_ref,
            })
            .await
            .expect("read_file invocation"); // safety: test-only assertion in #[cfg(test)] module.
        let CapabilityOutcome::Completed(completed) = outcome else {
            panic!("expected completed read_file invocation");
        };
        let output = capability_io
            .result_output(completed.result_ref.as_str())
            .expect("result output lookup") // safety: test-only assertion in #[cfg(test)] module.
            .expect("result output"); // safety: test-only assertion in #[cfg(test)] module.
        assert_eq!(
            output["content"],
            serde_json::json!("     1│ safe host file")
        );
    }

    #[test]
    fn model_visible_tool_output_truncates_at_utf8_boundary() {
        let output = model_visible_tool_output(&serde_json::json!({
            "message": "é".repeat(300),
        }));

        assert!(output.len() <= MODEL_VISIBLE_TOOL_OUTPUT_MAX_BYTES);
        assert!(output.is_char_boundary(output.len()));
        ToolResultSafeSummary::new(output).expect("summary remains safe");
    }
}
