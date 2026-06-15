use std::sync::Arc;

use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_runtime::SHELL_CAPABILITY_ID;
use ironclaw_loop_support::{
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_turns::{
    RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
    run_profile::{
        CapabilityInvocation, CapabilityOutcome, InMemoryLoopHostMilestoneSink,
        InMemoryRunProfileResolver, LoopRunContext, ProviderToolCall, VisibleCapabilityRequest,
    },
};

use super::{
    LocalDevCapabilityIo, LocalDevExtensionSurfaceSource, LocalDevLoopCapabilityPortFactory,
};

async fn run_context(label: &str) -> LoopRunContext {
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile resolves"); // safety: test-only assertion in #[cfg(test)] module.
    LoopRunContext::new(
        TurnScope::new(
            TenantId::new(format!("tenant-{label}")).expect("tenant id"), // safety: test-only assertion in #[cfg(test)] module.
            Some(AgentId::new(format!("agent-{label}")).expect("agent id")), // safety: test-only assertion in #[cfg(test)] module.
            Some(ProjectId::new(format!("project-{label}")).expect("project id")), // safety: test-only assertion in #[cfg(test)] module.
            ThreadId::new(format!("thread-{label}")).expect("thread id"), // safety: test-only assertion in #[cfg(test)] module.
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
        name: "builtin_shell".to_string(),
        arguments,
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

#[tokio::test]
async fn local_dev_yolo_shell_translates_workspace_workdir_without_scoped_mounts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let workspace_root = dir.path().join("workspace");
    let shell_workdir = workspace_root.join("qa-coding-smoke");
    std::fs::create_dir_all(&shell_workdir).expect("workspace shell dir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");
    let services = crate::build_reborn_services(
        crate::local_runtime_build_input_with_options(
            crate::RebornCompositionProfile::LocalDevYolo,
            "local-dev-shell-owner",
            storage_root,
            crate::RebornLocalRuntimeProfileOptions {
                confirm_host_access: true,
            },
        )
        .expect("local yolo input")
        .with_local_dev_workspace_root(workspace_root)
        .with_local_dev_confirmed_host_home_root(host_home),
    )
    .await
    .expect("local-dev services build");
    let runtime = services.host_runtime.clone().expect("host runtime");
    let workspace_mounts = services
        .local_runtime
        .as_ref()
        .expect("local runtime substrate")
        .workspace_mounts
        .clone();
    let memory_mounts = services
        .local_runtime
        .as_ref()
        .expect("local runtime substrate")
        .memory_mounts
        .clone();
    let policy = Arc::new(
        crate::local_dev_capability_policy::local_dev_capability_policy().expect("policy parses"),
    );
    let capability_io = Arc::new(LocalDevCapabilityIo::default());
    let input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    let factory = LocalDevLoopCapabilityPortFactory {
        runtime,
        fallback_user_id: UserId::new("local-dev-shell-user").expect("user id"),
        policy,
        workspace_mounts,
        memory_mounts,
        extension_surface_source: LocalDevExtensionSurfaceSource::default(),
        input_resolver,
        result_writer,
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        skill_activation_source: None,
    };
    let run_context = run_context("shell-workdir").await;
    let port = factory
        .create_capability_port(&run_context)
        .await
        .expect("capability port");
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible surface");
    let input_ref = capability_io
        .register_provider_tool_call_input(
            &run_context,
            &provider_tool_call(serde_json::json!({
                "command": "mkdir -p /workspace/qa-coding-smoke && test -d /host && printf '%s:%s' local-dev-shell-ok \"$PWD\"",
                "workdir": "/workspace/qa-coding-smoke"
            })),
        )
        .await
        .expect("input ref");

    let outcome = port
        .invoke_capability(CapabilityInvocation {
            surface_version: surface.version,
            capability_id: CapabilityId::new(SHELL_CAPABILITY_ID).expect("shell capability id"),
            input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("shell invocation");

    let CapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed shell invocation");
    };
    let output = capability_io
        .result_output(completed.result_ref.as_str())
        .expect("result output lookup")
        .expect("result output");
    assert_eq!(output["exit_code"], serde_json::json!(0));
    assert_eq!(output["success"], serde_json::json!(true));
    assert_eq!(
        output["output"],
        serde_json::json!(format!(
            "local-dev-shell-ok:{}",
            shell_workdir
                .canonicalize()
                .expect("canonical shell workdir")
                .display()
        ))
    );
}
