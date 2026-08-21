// arch-exempt: large_file, caller-tier coding-tool suite shares one runtime/mount fixture set, plan #4539
//
// Host-runtime integration coverage for the pinned coding tools (`read`,
// `write`, `edit`, `glob`, `grep`). Engine-level behavior (hashline output
// formats, selector grammar, stale-anchor rejection, glob/grep budgets) is
// covered by the engine suites in `ironclaw_extension_support` and the
// pinned contract snapshot under `tests/reborn_coding_engines.rs`; this
// file pins the HOST boundaries: mount authorization denials, workspace-root
// resolution, relative-path round trips, read-before-edit snapshot seeding
// through the public read path, post-edit-check wiring, and the failure-kind
// mapping of engine errors onto the runtime surface.
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_filesystem::{
    DiskFilesystem, Fault, FaultInjecting, FilesystemOperation, RootFilesystem,
};
use ironclaw_host_api::process::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
};
use ironclaw_host_api::result_meta::FailureKind;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{
    action::NetworkPolicy,
    artifact::{
        AccountedArtifactPersister, ArtifactDigest, ArtifactId, ArtifactNamespaceId, ArtifactRef,
        ArtifactWriteError, ArtifactWriteMetadata, CompletedArtifact,
    },
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    dispatch::DispatchFailureDetail,
    ids::{CapabilityGrantId, CapabilityId, ExtensionId, PackageId, RunId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{HostPath, MountAlias, VirtualPath},
    resource::{ResourceEstimate, ResourceReceipt},
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{
    CODING_EDIT_CAPABILITY_ID, CODING_READ_CAPABILITY_ID, CODING_WRITE_CAPABILITY_ID,
    CapabilitySurfaceVersion, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HostRuntime,
    HostRuntimeServices, PostEditCheckConfig, RuntimeCapabilityOutcome, RuntimeProcessPort,
    UserSandboxProcessPort, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use serde_json::{Value, json};

#[tokio::test]
async fn coding_write_to_read_only_mount_reports_an_actionable_denial() {
    // A write through a read-only scoped mount must fail as a filesystem
    // denial AND tell the model which path hit the permission wall (the
    // exact pinned resolution text rides the untrusted diagnostic channel).
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/notes.txt", "content": "hello"}),
        context,
    )
    .await;

    // FilesystemDenied is carried 1:1 through the runtime boundary.
    assert_eq!(failure.kind, FailureKind::FilesystemDenied);
    let Some(DispatchFailureDetail::Diagnostic { text }) = failure.detail.as_ref() else {
        panic!(
            "expected a diagnostic carrying the coding denial, got {:?}",
            failure.detail
        );
    };
    assert!(
        text.contains("workspace/notes.txt"),
        "the reason must name the denied path: {text}"
    );
    assert!(
        text.contains("does not permit"),
        "the reason must say the mount refused the operation: {text}"
    );
    assert!(
        !temp.path().join("notes.txt").exists(),
        "the denied write must not touch the filesystem"
    );
}

/// Agent-scoped dispatch without a caller-stamped artifact namespace must not
/// fail closed: the host runtime derives an invocation-anchored namespace (the
/// same `ArtifactNamespaceId::from_root_run` derivation the WebUI product
/// adapter uses), so the kernel's agent-scoped artifact-persistence guard
/// passes and the completed result carries the durable artifact. Regression
/// for the cutover guard that failed every agent-scoped direct `HostRuntime`
/// invoke with `ResourceError::Storage` surfaced as "the tool ran out of
/// resources" (`standalone_extension_activate_accepts_manual_token_from_webui_gate_scope`
/// and the admin-configuration/trigger-create harnesses).
#[tokio::test]
async fn agent_scoped_dispatch_without_stamped_namespace_derives_one_and_persists() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let mut context = execution_context_with_mounts([CODING_WRITE_CAPABILITY_ID], mounts);
    assert!(
        context.agent_id.is_some(),
        "fixture must be agent-scoped to exercise the agent-scoped guard"
    );
    context.artifact_namespace = None;
    assert!(
        context.run_id.is_some(),
        "fixture context carries a run identity like the production loop"
    );

    let completed = invoke_completed_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/notes.txt", "content": "hello"}),
        context,
    )
    .await;

    let artifact = completed
        .completed_artifact
        .expect("agent-scoped dispatch must persist canonical output as a durable artifact");
    assert!(
        artifact.byte_len > 0,
        "the persisted artifact must account the written output bytes"
    );
    assert!(
        completed.canonical_output_digest.is_some(),
        "the completed result must carry the canonical output digest"
    );
}

#[tokio::test]
async fn coding_read_out_of_scope_rejection_carries_the_path_and_available_roots() {
    // Loop-boundary pin: an out-of-scope absolute path (copied verbatim from
    // a task description) must produce a FilesystemDenied failure whose
    // diagnostic names the path and the available scoped roots so the model
    // can correct course.
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": "/testbed/replacer.go"}),
        context,
    )
    .await;

    assert_eq!(failure.kind, FailureKind::FilesystemDenied);
    let Some(DispatchFailureDetail::Diagnostic { text }) = failure.detail.as_ref() else {
        panic!(
            "expected a diagnostic carrying the coding resolution error, got {:?}",
            failure.detail
        );
    };
    assert!(
        text.contains("/testbed/replacer.go"),
        "the reason must name the offending path: {text}"
    );
    assert!(
        text.contains("available scoped root"),
        "the reason must point at the scoped-root resolution: {text}"
    );
}

#[tokio::test]
async fn coding_read_failure_reports_missing_path() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_only());
    let runtime = runtime_with_filesystem(filesystem);
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let failure = invoke_failure_with_context(
        &runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": "/workspace/missing.py"}),
        context,
    )
    .await;

    assert_eq!(failure.kind, FailureKind::OperationFailed);
    let Some(DispatchFailureDetail::Diagnostic { text }) = failure.detail.as_ref() else {
        panic!(
            "expected a diagnostic carrying the coding not-found text, got {:?}",
            failure.detail
        );
    };
    assert_eq!(text, "Path '/workspace/missing.py' not found");
}

#[tokio::test]
async fn coding_write_maps_filesystem_provider_write_failure_to_operation_failed() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "old\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(
        FaultInjecting::new(filesystem).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("/main.rs")
                .backend("injected write failure"),
        ),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let error = invoke_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/main.rs", "content": "new\n"}),
        context,
    )
    .await
    .unwrap_err();

    assert_eq!(error, FailureKind::OperationFailed);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("main.rs")).unwrap(),
        "old\n"
    );
}

#[tokio::test]
async fn builtin_edit_tools_append_new_post_edit_check_findings_only() {
    // The operator-configured post-edit check runs after a successful edit and
    // surfaces its diagnostics to the model. A second edit whose check output
    // is identical must not repeat previously-reported lines (new-only diff).
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("main.rs"), "alpha beta\n").unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing(
        "error[E0308]: mismatched types\nwarning: unused variable `x`\n",
        1,
    ));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new(
            "cargo check --message-format=short 2>&1",
            Duration::from_secs(7),
        ),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);
    let header = seed_read_tag(&runtime, "/workspace/main.rs", context.clone()).await;
    assert!(
        check_port.requests().is_empty(),
        "read must not trigger the post-edit check"
    );

    let first_completed = invoke_completed_with_context(
        &runtime,
        CODING_EDIT_CAPABILITY_ID,
        json!({"input": format!("{header}\nPUT 1:\n+gamma beta\n")}),
        context.clone(),
    )
    .await;
    assert_eq!(
        first_completed.usage.process_count, 1,
        "an edit whose post-edit check ran must account for the spawned \
         process exactly like builtin.shell"
    );
    let first = first_completed.output;

    assert!(
        first["output"]
            .as_str()
            .is_some_and(|output| output.contains("gamma beta")),
        "edit itself must succeed and render the new content: {first}"
    );
    assert_eq!(first["post_edit_check"]["exit_code"], json!(1));
    let new_output = first["post_edit_check"]["new_output"]
        .as_str()
        .expect("first edit surfaces the check findings as new_output");
    assert!(new_output.contains("error[E0308]: mismatched types"));
    assert!(new_output.contains("unused variable"));

    let requests = check_port.requests();
    assert_eq!(requests.len(), 1, "one check per successful edit");
    assert_eq!(
        requests[0].command,
        "cargo check --message-format=short 2>&1"
    );
    assert_eq!(requests[0].timeout_secs, Some(7));
    assert_eq!(
        requests[0].workdir.as_deref(),
        Some("/workspace"),
        "check must run at the writable mount root so the process port \
         resolves it exactly like a shell workdir"
    );

    // A chained edit on the same file anchors on the tag of a CURRENT read:
    // the first edit changed the file hash, so refresh the header through the
    // public read path (the canonical read -> edit chain the engine contract
    // and the passing trace suites exercise) instead of reusing the stale
    // read header or the first edit's echoed output header.
    let refreshed_header = seed_read_tag(&runtime, "/workspace/main.rs", context.clone()).await;
    assert_ne!(
        refreshed_header, header,
        "the first edit must refresh the file hash: {refreshed_header}"
    );
    let second = invoke_with_context(
        &runtime,
        CODING_EDIT_CAPABILITY_ID,
        json!({"input": format!("{refreshed_header}\nPUT 1:\n+gamma delta\n")}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(
        second["post_edit_check"],
        json!({"exit_code": 1}),
        "identical check output must carry no repeated lines"
    );
    assert_eq!(check_port.requests().len(), 2);
}

#[tokio::test]
async fn builtin_edit_tools_skip_post_edit_check_when_unconfigured() {
    // Feature off (no config): the mutating tools must not touch the process
    // port at all and the model-facing output carries no post_edit_check field.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let runtime = runtime_with_filesystem_and_process_port(filesystem, Arc::clone(&check_port));
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert!(
        written["output"]
            .as_str()
            .is_some_and(|output| output.contains("Successfully wrote")),
        "write must succeed: {written}"
    );
    assert!(
        written.get("post_edit_check").is_none(),
        "unconfigured runtime must not emit a post_edit_check field"
    );
    assert!(
        check_port.requests().is_empty(),
        "unconfigured runtime must not invoke the process port"
    );
}

#[tokio::test]
async fn builtin_edit_tools_report_post_edit_check_timeout_without_failing_the_edit() {
    // The check is advisory: a timed-out check must not fail the already
    // successful edit, and the model learns the check timed out.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::timing_out(Duration::from_secs(7)));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(7)),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert!(
        written["output"]
            .as_str()
            .is_some_and(|output| output.contains("Successfully wrote")),
        "edit must not fail: {written}"
    );
    assert_eq!(written["post_edit_check"], json!({"timed_out": true}));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("new.rs")).unwrap(),
        "fn hello() {}\n"
    );
}

#[tokio::test]
async fn builtin_edit_tools_omit_new_output_when_check_passes_clean() {
    // A passing check with no new findings stays token-lean: exit_code only.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("", 0));
    let runtime = runtime_with_filesystem_process_port_and_post_edit_check(
        filesystem,
        Arc::clone(&check_port),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let written = invoke_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await
    .unwrap();

    assert_eq!(written["post_edit_check"], json!({"exit_code": 0}));
}

#[tokio::test]
async fn builtin_edit_tools_disable_post_edit_check_when_process_backend_is_none() {
    // Regression (PR #5979 review): write/edit declare only filesystem
    // effects, so their plan never requires a process — but a configured
    // post-edit check used to spawn through the default process port anyway,
    // bypassing ProcessBackendKind::None entirely. Under a no-process policy
    // the advisory check must be withheld: the edit succeeds, no process port
    // is touched, and nothing is accounted.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let check_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let runtime = runtime_with_post_edit_check_and_policy(
        filesystem,
        Arc::clone(&check_port),
        None,
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
        process_denied_runtime_policy(),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await;

    assert!(
        completed.output["output"]
            .as_str()
            .is_some_and(|output| output.contains("Successfully wrote")),
        "edit must succeed: {}",
        completed.output
    );
    assert!(
        completed.output.get("post_edit_check").is_none(),
        "ProcessBackendKind::None must disable the post-edit check"
    );
    assert!(
        check_port.requests().is_empty(),
        "a no-process policy must never spawn the check on the local host port"
    );
    assert_eq!(
        completed.usage.process_count, 0,
        "no process ran, so none may be accounted"
    );
}

#[tokio::test]
async fn builtin_edit_tools_run_post_edit_check_in_user_sandbox_not_on_local_host() {
    // Regression (PR #5978 review): the edit plans declare no process effect, so
    // the default process port handed to them is the deployment-blind local host
    // port. Running the configured check through it would escape the sandbox onto
    // the shared provider host under a user-sandbox policy. The resolver instead
    // bundles the check with the port matching the plan's process backend, so
    // under a user-sandbox policy the check runs ISOLATED in the user's own
    // sandbox — never on the local host port.
    let temp = tempfile::tempdir().unwrap();

    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let local_port = Arc::new(ScriptedProcessPort::completing("diagnostics", 1));
    let sandbox_transport = Arc::new(RecordingSandboxTransport::default());
    let runtime = runtime_with_post_edit_check_and_policy(
        filesystem,
        Arc::clone(&local_port),
        Some(Arc::new(UserSandboxProcessPort::new(
            Arc::clone(&sandbox_transport) as Arc<dyn SandboxCommandTransport>,
        ))),
        PostEditCheckConfig::new("cargo check", Duration::from_secs(30)),
        user_sandbox_runtime_policy(),
    );
    let context = execution_context_with_mounts(coding_capability_ids(), mounts);

    let completed = invoke_completed_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "/workspace/new.rs", "content": "fn hello() {}\n"}),
        context,
    )
    .await;

    assert!(
        completed.output["output"]
            .as_str()
            .is_some_and(|output| output.contains("Successfully wrote")),
        "edit must succeed: {}",
        completed.output
    );
    assert_eq!(
        completed.output["post_edit_check"]["new_output"]
            .as_str()
            .expect("the sandbox-run check surfaces its diagnostics as new_output"),
        "sandbox diagnostics",
        "a user-sandbox policy runs the check ISOLATED in the user sandbox \
         and surfaces its output to the model"
    );
    assert!(
        local_port.requests().is_empty(),
        "the check must not escape the sandbox policy onto the local host port"
    );
    assert_eq!(
        sandbox_transport.request_count(),
        1,
        "the check runs through the user sandbox port, never the local host"
    );
    assert_eq!(
        completed.usage.process_count, 1,
        "the sandbox-run check is accounted as one spawned process"
    );
}

/// Editing an existing file requires a prior `read` of that file in the same
/// run: the read records the hashline snapshot tag the edit anchors on. This
/// helper seeds that state through the public read path and returns the
/// `[path#TAG]` header line the edit input must carry.
async fn seed_read_tag<R: HostRuntime + ?Sized>(
    runtime: &R,
    path: &str,
    context: ExecutionContext,
) -> String {
    let read = invoke_with_context(
        runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": path}),
        context,
    )
    .await
    .expect("read seeds the hashline snapshot");
    read["output"]
        .as_str()
        .expect("coding read returns text")
        .lines()
        .next()
        .expect("coding read starts with the hashline header")
        .to_string()
}

async fn invoke_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, FailureKind> {
    let outcome = runtime
        .invoke_capability((
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

async fn invoke_completed_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> ironclaw_host_runtime::RuntimeCapabilityCompleted {
    let outcome = runtime
        .invoke_capability((
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => *completed,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

async fn invoke_failure_with_context<R: HostRuntime + ?Sized>(
    runtime: &R,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> ironclaw_host_runtime::RuntimeCapabilityFailure {
    let outcome = runtime
        .invoke_capability((
            context,
            CapabilityId::new(capability).unwrap(),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

/// Deterministic in-memory `AccountedArtifactPersister` for agent-scoped
/// dispatch: unique monotonic `ArtifactId`s, checked byte length, digest over
/// the persisted bytes, and the metadata content type. Agent-scoped dispatch
/// requires a persister (the kernel guard fails with `Resource` otherwise);
/// mirrors the loop ingress contract.
#[derive(Default)]
struct TestArtifactPersister {
    next_id: AtomicU64,
}

#[async_trait]
impl AccountedArtifactPersister for TestArtifactPersister {
    async fn persist(
        &self,
        metadata: ArtifactWriteMetadata,
        bytes: &[u8],
        _receipt: &ResourceReceipt,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let artifact_id = ArtifactId::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        let byte_len = u64::try_from(bytes.len()).map_err(|_| ArtifactWriteError::Storage)?;
        Ok(CompletedArtifact {
            artifact_ref: ArtifactRef::new(artifact_id),
            byte_len,
            total_lines: None,
            content_type: metadata.content_type,
            digest: ArtifactDigest::from_bytes(bytes),
        })
    }
}

/// Minimal `ArtifactPersistencePort` for the coding spill path. Coding output
/// above the inline ceiling allocates/appends/finalizes through this port
/// (distinct from the kernel's accounted persister above), so a fixture
/// without it cannot exercise a spilled preview at all.
#[derive(Default)]
struct TestArtifactStore {
    bytes: tokio::sync::Mutex<Vec<u8>>,
    content_type: tokio::sync::Mutex<Option<String>>,
}

#[async_trait]
impl ironclaw_host_api::artifact::ArtifactAccessPort for TestArtifactStore {
    async fn read(
        &self,
        _request: ironclaw_host_api::artifact::ArtifactReadRequest,
    ) -> Result<
        Option<ironclaw_host_api::artifact::ArtifactReadChunk>,
        ironclaw_host_api::artifact::ArtifactAccessError,
    > {
        Ok(None)
    }
}

#[async_trait]
impl ironclaw_host_api::artifact::ArtifactPersistencePort for TestArtifactStore {
    async fn allocate(
        &self,
        metadata: ArtifactWriteMetadata,
    ) -> Result<ironclaw_host_api::artifact::ArtifactWriteHandle, ArtifactWriteError> {
        let handle = ironclaw_host_api::artifact::ArtifactWriteHandle::new(
            ArtifactId::new(0),
            metadata.owner_scope.clone(),
            metadata.namespace,
        );
        *self.content_type.lock().await = Some(metadata.content_type);
        self.bytes.lock().await.clear();
        Ok(handle)
    }

    async fn append(
        &self,
        _handle: &ironclaw_host_api::artifact::ArtifactWriteHandle,
        chunk: &[u8],
    ) -> Result<(), ArtifactWriteError> {
        self.bytes.lock().await.extend_from_slice(chunk);
        Ok(())
    }

    async fn finalize(
        &self,
        handle: ironclaw_host_api::artifact::ArtifactWriteHandle,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let bytes = self.bytes.lock().await;
        let content_type = self
            .content_type
            .lock()
            .await
            .clone()
            .ok_or(ArtifactWriteError::InvalidHandle)?;
        Ok(CompletedArtifact {
            artifact_ref: ArtifactRef::new(handle.artifact_id()),
            byte_len: u64::try_from(bytes.len()).map_err(|_| ArtifactWriteError::Storage)?,
            total_lines: None,
            content_type,
            digest: ArtifactDigest::from_bytes(&bytes),
        })
    }
}

fn runtime_with_filesystem_and_artifacts<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    let artifacts = Arc::new(TestArtifactStore::default());
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_artifact_ports(artifacts.clone(), artifacts)
    .with_accounted_artifact_persistence(Arc::new(TestArtifactPersister::default()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem<F>(filesystem: F) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_accounted_artifact_persistence(Arc::new(TestArtifactPersister::default()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem_and_process_port<F, P>(
    filesystem: F,
    process_port: Arc<P>,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(process_port)
    .with_accounted_artifact_persistence(Arc::new(TestArtifactPersister::default()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

fn runtime_with_filesystem_process_port_and_post_edit_check<F, P>(
    filesystem: F,
    process_port: Arc<P>,
    post_edit_check: PostEditCheckConfig,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
{
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(process_port)
    .with_post_edit_check(post_edit_check)
    .with_accounted_artifact_persistence(Arc::new(TestArtifactPersister::default()))
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

/// Like `runtime_with_filesystem_process_port_and_post_edit_check`, but with
/// an explicit runtime policy (and optionally a user sandbox process port)
/// so tests can pin how the process policy gates the post-edit check.
fn runtime_with_post_edit_check_and_policy<F, P>(
    filesystem: F,
    process_port: Arc<P>,
    user_sandbox_process_port: Option<Arc<UserSandboxProcessPort>>,
    post_edit_check: PostEditCheckConfig,
    policy: EffectiveRuntimePolicy,
) -> impl HostRuntime
where
    F: RootFilesystem + 'static,
    P: RuntimeProcessPort + 'static,
{
    let mut services = HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(process_port)
    .with_post_edit_check(post_edit_check)
    .with_runtime_policy(policy)
    .with_accounted_artifact_persistence(Arc::new(TestArtifactPersister::default()))
    .with_trust_policy(Arc::new(trust_policy()));
    if let Some(user_sandbox_process_port) = user_sandbox_process_port {
        services = services.with_user_sandbox_process_port(user_sandbox_process_port);
    }
    services.host_runtime_for_local_testing()
}

/// SecureDefault-shaped local policy: scoped-virtual filesystem, no process
/// backend. Approval stays at AskDestructive so the only axis under test is
/// the process backend.
fn process_denied_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::None,
        network_mode: NetworkMode::Brokered,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// HostedDev-shaped tenant policy with a user-sandbox process backend and the
/// same tenant-workspace filesystem selection used by production hosted
/// deployments.
fn user_sandbox_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::HostedDev,
        resolved_profile: RuntimeProfile::HostedDev,
        filesystem_backend: FilesystemBackendKind::TenantWorkspace,
        process_backend: ProcessBackendKind::UserSandbox,
        network_mode: NetworkMode::Allowlist,
        secret_mode: SecretMode::TenantBroker,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::Standard,
    }
}

/// Sandbox transport double that counts requests; the user-sandbox test
/// asserts the post-edit check runs through it (isolated in the tenant
/// sandbox) rather than escaping onto the local host port.
#[derive(Default)]
struct RecordingSandboxTransport {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
}

impl RecordingSandboxTransport {
    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request);
        Ok(CommandExecutionOutput {
            output: "sandbox diagnostics".to_string(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::from_millis(3),
        })
    }
}

/// Process-port double that records every request and replays one scripted
/// outcome, mirroring the recording port used by the builtin.shell tests.
struct ScriptedProcessPort {
    requests: std::sync::Mutex<Vec<CommandExecutionRequest>>,
    response: Result<(String, i64), RuntimeProcessError>,
}

impl ScriptedProcessPort {
    fn completing(output: &str, exit_code: i64) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            response: Ok((output.to_string(), exit_code)),
        }
    }

    fn timing_out(timeout: Duration) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            response: Err(RuntimeProcessError::Timeout(timeout)),
        }
    }

    fn requests(&self) -> Vec<CommandExecutionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl RuntimeProcessPort for ScriptedProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.requests.lock().unwrap().push(request);
        match &self.response {
            Ok((output, exit_code)) => Ok(CommandExecutionOutput {
                output: output.clone(),
                saved_output: None,
                exit_code: *exit_code,
                sandboxed: false,
                duration: Duration::from_millis(3),
            }),
            Err(error) => Err(error.clone()),
        }
    }
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

fn coding_capability_ids() -> [&'static str; 5] {
    [
        CODING_READ_CAPABILITY_ID,
        CODING_WRITE_CAPABILITY_ID,
        CODING_EDIT_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
    ]
}

fn mounted_filesystem(path: &Path, permissions: MountPermissions) -> (DiskFilesystem, MountView) {
    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects/coding-pack").unwrap(),
            HostPath::from_path_buf(path.to_path_buf()),
        )
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/coding-pack").unwrap(),
        permissions,
    )])
    .unwrap();
    (filesystem, mounts)
}

fn execution_context_with_mounts<const N: usize>(
    grants: [&str; N],
    mounts: MountView,
) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: grants
            .into_iter()
            .map(|grant| dispatch_grant_with_mounts(grant, mounts.clone()))
            .collect(),
    };
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    // Agent-scoped dispatch requires a durable artifact namespace (plus a
    // persister, which every runtime builder here wires). Derive it from the
    // context's run identity so the harness mirrors the loop ingress contract
    // instead of tripping the kernel's agent-scoped guard.
    context.artifact_namespace = Some(ArtifactNamespaceId::from_root_run(
        context.run_id.unwrap_or_default(),
    ));
    context
}

fn dispatch_grant_with_mounts(capability: &str, mounts: MountView) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts,
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        // The coding edit descriptor declares delete authority (REM/MV file ops),
        // so the coding grants must honestly carry it or every edit that the
        // authorization fold plans with delete authority is policy-denied.
        EffectKind::DeleteFilesystem,
    ]
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

#[tokio::test]
async fn a_relative_path_written_by_write_is_readable_by_read() {
    let temp = tempfile::tempdir().unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem(filesystem);
    let context = || execution_context_with_mounts(coding_capability_ids(), mounts.clone());

    invoke_with_context(
        &runtime,
        CODING_WRITE_CAPABILITY_ID,
        json!({"path": "scripts/egfr.py", "content": "print('staged')\n"}),
        context(),
    )
    .await
    .expect("a relative path must be writable");

    let read = invoke_with_context(
        &runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": "scripts/egfr.py"}),
        context(),
    )
    .await
    .expect("the same relative path must be readable");

    assert!(
        read["output"]
            .as_str()
            .expect("coding read returns text")
            .contains("staged"),
        "write and read must resolve one relative path to one place; got {read:?}"
    );
}

/// Regression (PinchBench payload parity): an engine-bounded `read` window
/// above the host's 24 KiB inline threshold must stay inline and whole.
///
/// Spilling is what activates the kernel's canonical bound, so routing large
/// reads through an artifact capped payload per call far below what the
/// pre-pinned `read_file` delivered (46.5 KiB median, 75.4 KiB max, no
/// artifact). Because context accumulates, that shrinks payload and *raises*
/// total tokens for the same file: measured 5.9x smaller payloads for 2.53x the
/// input tokens.
#[tokio::test]
async fn an_engine_bounded_read_window_stays_inline_above_the_host_threshold() {
    let temp = tempfile::tempdir().unwrap();
    let body: String = (1..=4_000usize)
        .map(|n| format!("line {n:05} {}\n", "transcript filler text".repeat(3)))
        .collect();
    std::fs::write(temp.path().join("transcript.md"), &body).unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem_and_artifacts(filesystem);

    // ~400 rendered lines: over the 24 KiB host threshold that used to force a
    // spill, under the document inline ceiling.
    let read = invoke_with_context(
        &runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": "transcript.md:100-700"}),
        execution_context_with_mounts(coding_capability_ids(), mounts),
    )
    .await
    .expect("an engine-bounded window must be readable");

    let output = read["output"].as_str().expect("coding read returns text");
    assert!(
        output.len() > 24 * 1024,
        "fixture must exceed the 24 KiB host inline threshold to be meaningful; got {} bytes",
        output.len()
    );
    assert!(
        read.get("artifact_ref").is_none(),
        "an engine-bounded window must not spill: spilling is what caps the payload; got {read:?}"
    );
    assert!(
        output.contains("line 00100 ") && output.contains("line 00500 "),
        "the whole requested span must be delivered inline"
    );
    assert!(
        !output.contains("artifact output elided"),
        "a window delivered whole must carry no elision marker"
    );
}

/// Regression (PinchBench transcript tasks): a spilled `read` of an explicit
/// contiguous line range must not come back with its middle deleted.
///
/// The old adapter preview kept a head and a tail and dropped everything
/// between with a bare `... [artifact output elided] ...` marker that named no
/// continuation. On a ~5,000-line transcript the model received ~24% of the
/// lines it asked for, could not tell which span was missing, and burned a
/// second call re-reading the gap — measured as 236 of 956 read calls eliding
/// content across 52 of 147 benchmark tasks. The window must instead stay
/// contiguous and name the artifact selector to resume from.
#[tokio::test]
async fn a_spilled_read_window_stays_contiguous_and_names_its_resume_selector() {
    let temp = tempfile::tempdir().unwrap();
    // Distinctive per-line payload so a deleted middle is detectable by content
    // rather than by byte count alone.
    let total_lines = 4_000usize;
    let body: String = (1..=total_lines)
        .map(|n| format!("line {n:05} {}\n", "transcript filler text".repeat(3)))
        .collect();
    std::fs::write(temp.path().join("transcript.md"), &body).unwrap();
    let (filesystem, mounts) = mounted_filesystem(temp.path(), MountPermissions::read_write());
    let runtime = runtime_with_filesystem_and_artifacts(filesystem);

    let read = invoke_with_context(
        &runtime,
        CODING_READ_CAPABILITY_ID,
        json!({"path": "transcript.md:100-2400"}),
        execution_context_with_mounts(coding_capability_ids(), mounts),
    )
    .await
    .expect("a wide contiguous range must be readable");

    let output = read["output"].as_str().expect("coding read returns text");
    assert!(
        read["artifact_ref"].is_string(),
        "an oversized window must spill to a durable artifact; got {read:?}"
    );
    assert!(
        !output.contains("artifact output elided"),
        "a document window must never have its middle deleted; got:\n{output}"
    );

    // Every rendered line number present must be strictly consecutive: a hole
    // is exactly what forced the model to re-read the gap.
    let rendered: Vec<u64> = output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(number, _)| number.trim().parse::<u64>().ok())
        .collect();
    assert!(
        rendered.len() > 1,
        "the preview must carry numbered lines; got:\n{output}"
    );
    for pair in rendered.windows(2) {
        assert_eq!(
            pair[1],
            pair[0] + 1,
            "rendered lines must stay contiguous, found a gap {} -> {} in:\n{output}",
            pair[0],
            pair[1]
        );
    }

    // The model must be told how to continue, in the engine's own selector
    // idiom, rather than being left to guess the next window.
    let artifact_ref = read["artifact_ref"].as_str().expect("artifact ref string");
    assert!(
        output.contains(&format!("Use {artifact_ref}:")) && output.contains("to continue"),
        "a truncated window must name the artifact selector to resume from; got:\n{output}"
    );
}
