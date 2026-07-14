//! core_builtin domain tools profile (`core_builtin_tools`).
//!
//! Unlike the other `profiles/*` domains, this harness does NOT flow through
//! `new_with_options`/`RebornServices` — it builds the `HostRuntime` directly
//! via `local_dev_host_runtime_with_http_egress` /
//! `local_dev_host_runtime_with_live_http_egress` and assembles
//! `HostRuntimeCapabilityHarness` by hand (`core_builtin_tools_from_runtime`),
//! so it does not go through `ToolsProfile`/`.build()`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingNetworkHttpTransport,
    RecordingRuntimeHttpEgress, host_runtime_storage_roots, http_test_policy,
    local_dev_host_runtime_with_http_egress, local_dev_host_runtime_with_live_http_egress,
    local_dev_host_runtime_with_real_egress_pipeline, memory_mounts, workspace_mounts,
};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, RuntimeKind, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, HTTP_CAPABILITY_ID,
    HTTP_SAVE_CAPABILITY_ID, HostRuntime, JSON_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    PROFILE_SET_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, RuntimeProcessPort, SHELL_CAPABILITY_ID,
    TIME_CAPABILITY_ID,
};

/// How [`core_builtin_tools`] constructs HTTP egress. The three modes are
/// mutually exclusive by construction: one field holds exactly one mode, and
/// each setter overwrites it (so the last setter called wins).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EgressMode {
    /// Default: whole-pipeline-bypassing `RecordingRuntimeHttpEgress` (no
    /// policy enforcement or leak scan runs; requests/responses are scripted
    /// and captured on the harness).
    Recording,
    /// `local_dev_host_runtime_with_live_http_egress`: real HTTP egress over
    /// the real network. No recording `RuntimeHttpEgress`/process port is
    /// captured on the harness.
    Live,
    /// S1 seam: `local_dev_host_runtime_with_real_egress_pipeline` — the REAL
    /// production egress pipeline (network-policy enforcement + leak scan)
    /// with only the wire-level transport recorded.
    RealPipeline,
}

/// Configuration axes for [`core_builtin_tools`]. `Default` matches the
/// zero-arg `core_builtin_tools(CoreBuiltinOptions::default())` call.
pub(crate) struct CoreBuiltinOptions {
    /// Network policy the built harness dispatches capabilities under.
    /// Defaults to `http_test_policy()`; override via `.with_network_policy(..)`.
    pub(crate) network_policy: NetworkPolicy,
    /// `true` (default) injects the inert `RecordingProcessPort` so
    /// `builtin.shell` invocations in tests never spawn a real OS process.
    /// `.with_live_shell()` sets this `false`, which skips injection and lets
    /// `HostRuntimeServices` default to the real `LocalHostProcessPort`.
    /// Consulted for `EgressMode::Recording` and `EgressMode::RealPipeline`;
    /// the `Live` path never wires a process port either way.
    pub(crate) recording_process: bool,
    /// HTTP egress construction mode; see [`EgressMode`]. Defaults to
    /// `Recording`; set via `.with_live_http_egress()` /
    /// `.with_real_egress_pipeline()`.
    pub(crate) egress: EgressMode,
}

impl Default for CoreBuiltinOptions {
    fn default() -> Self {
        Self {
            network_policy: http_test_policy(),
            recording_process: true,
            egress: EgressMode::Recording,
        }
    }
}

impl CoreBuiltinOptions {
    pub(crate) fn with_network_policy(mut self, network_policy: NetworkPolicy) -> Self {
        self.network_policy = network_policy;
        self
    }

    /// Opts out of the recording process port so the real `LocalHostProcessPort`
    /// executes shell commands on the host.
    pub(crate) fn with_live_shell(mut self) -> Self {
        self.recording_process = false;
        self
    }

    pub(crate) fn with_live_http_egress(mut self) -> Self {
        self.egress = EgressMode::Live;
        self
    }

    /// S1 seam: run the real production egress pipeline (network-policy
    /// enforcement + leak scan) with only the wire-level transport recorded.
    pub(crate) fn with_real_egress_pipeline(mut self) -> Self {
        self.egress = EgressMode::RealPipeline;
        self
    }
}

/// Core built-in tools (`time`/`json`/`http`/`memory_*`/`profile_set`/
/// `read_file`/`apply_patch`/`shell`). See [`CoreBuiltinOptions`] for the axes.
pub(crate) async fn core_builtin_tools(
    options: CoreBuiltinOptions,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let CoreBuiltinOptions {
        network_policy,
        recording_process,
        egress,
    } = options;
    // Inject the inert recording port by default so `builtin.shell`
    // invocations in tests never spawn a real OS process. `.with_live_shell()`
    // sets `recording_process = false`, which skips injection and lets
    // `HostRuntimeServices` default to the real `LocalHostProcessPort`.
    let recording_process_port = if recording_process {
        Some(Arc::new(
            super::super::super::process::RecordingProcessPort::new(),
        ))
    } else {
        None
    };
    let process_port_dyn: Option<Arc<dyn RuntimeProcessPort>> = recording_process_port
        .as_ref()
        .map(|p| Arc::clone(p) as Arc<dyn RuntimeProcessPort>);
    match egress {
        EgressMode::RealPipeline => {
            let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
            let transport = RecordingNetworkHttpTransport::with_body(br#"{"ok":true}"#.to_vec());
            let runtime = local_dev_host_runtime_with_real_egress_pipeline(
                storage_root.clone(),
                transport.clone(),
                process_port_dyn,
            )?;
            let mut harness = core_builtin_tools_from_runtime(
                root,
                workspace_root,
                runtime,
                network_policy,
                UserId::new("reborn-e2e-core-builtins-real-egress-user")?,
            )?;
            harness.real_egress_transport = Some(Arc::new(transport));
            harness.process_port = recording_process_port;
            Ok(harness)
        }
        EgressMode::Live => {
            let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
            let runtime = local_dev_host_runtime_with_live_http_egress(storage_root.clone())?;
            core_builtin_tools_from_runtime(
                root,
                workspace_root,
                runtime,
                network_policy,
                UserId::new("reborn-e2e-core-builtins-live-http-user")?,
            )
        }
        EgressMode::Recording => {
            let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
            let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
                br#"{"accepted":true}"#.to_vec(),
            ));
            let runtime = local_dev_host_runtime_with_http_egress(
                storage_root.clone(),
                Arc::clone(&runtime_http_egress),
                process_port_dyn,
            )?;
            let mut harness = core_builtin_tools_from_runtime(
                root,
                workspace_root,
                runtime,
                network_policy,
                UserId::new("reborn-e2e-core-builtins-user")?,
            )?;
            harness.http_egress = Some(runtime_http_egress);
            harness.process_port = recording_process_port;
            Ok(harness)
        }
    }
}

/// Zero-arg convenience; most callers want this and never touch
/// `CoreBuiltinOptions`.
pub(crate) async fn core_builtin_tools_default() -> HarnessResult<HostRuntimeCapabilityHarness> {
    core_builtin_tools(CoreBuiltinOptions::default()).await
}

/// Harness-port-seam Change 4: the SAME `core_builtin_tools_default` backend,
/// with an additional confirmed `/host` mount grant layered onto the
/// workspace mount view — mirrors `local_dev_mounts::ambient_workspace_mount_view`
/// appending a `/host` alias when `host_home_aliases` is non-empty. This is
/// the ONLY integration-tier construction with a confirmed host-home mount,
/// so it is the sole way to observe `wrap_local_dev_surface_disclosure`'s
/// scoped-roots note (the layer is a no-op — disabled — without a `/host`
/// alias present, see `LocalDevSurfaceDisclosure::enabled`).
pub(crate) async fn core_builtin_tools_with_confirmed_host_mount()
-> HarnessResult<HostRuntimeCapabilityHarness> {
    let mut harness = core_builtin_tools(CoreBuiltinOptions::default()).await?;
    let mut mounts = harness.mounts.mounts.clone();
    mounts.push(MountGrant::new(
        MountAlias::new("/host")?,
        VirtualPath::new("/projects/host")?,
        MountPermissions::read_write_list_delete(),
    ));
    harness.mounts = MountView::new(mounts)?;
    Ok(harness)
}

/// Real capability ids `core_builtin_tools_from_runtime` registers on the
/// built harness — a single source shared with the wiring-parity
/// capability-id subset check (`tests/integration/wiring_parity.rs`) instead
/// of a second hand-transcribed copy of this list.
pub(crate) fn core_builtin_tools_capability_ids() -> HarnessResult<Vec<CapabilityId>> {
    Ok(vec![
        CapabilityId::new(TIME_CAPABILITY_ID)?,
        CapabilityId::new(JSON_CAPABILITY_ID)?,
        CapabilityId::new(HTTP_CAPABILITY_ID)?,
        CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
        CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
        CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
        CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
        // `builtin.shell` on the surface so scripted shell calls route
        // through the process port (recording by default, live via
        // `.with_live_shell()`).
        CapabilityId::new(SHELL_CAPABILITY_ID)?,
    ])
}

fn core_builtin_tools_from_runtime(
    root: Arc<tempfile::TempDir>,
    workspace_root: PathBuf,
    runtime: Arc<dyn HostRuntime>,
    network_policy: NetworkPolicy,
    user_id: UserId,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
    let memory_mounts = memory_mounts(MountPermissions::read_write_list_delete())?;
    let memory_capability_ids = [
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
        CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
        // profile_set writes to the memory mount (context/profile.json under
        // the user-scoped scope), so it needs the memory mount override just
        // like the four memory_* capabilities above.
        CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
    ];
    let (io, result_writer_io) = super::super::default_capability_io_pair();
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Mutex::new(io),
        result_writer_io: Mutex::new(result_writer_io),
        durable_capability_io_thread_service: Mutex::new(None),
        durable_capability_io_requested: false,
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: memory_capability_ids
            .iter()
            .cloned()
            .map(|capability_id| (capability_id, memory_mounts.clone()))
            .collect(),
        capability_ids: core_builtin_tools_capability_ids()?,
        runtime_kind: RuntimeKind::FirstParty,
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::SpawnProcess,
            // `builtin.shell` declares ExecuteCode; the grant's allowed_effects
            // must include it or the authorizer denies the capability before
            // it reaches the process port.
            EffectKind::ExecuteCode,
        ],
        network_policy,
        secrets: Vec::new(),
        provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
        additional_provider_trust: Vec::new(),
        user_id,
        invocations: Arc::new(Mutex::new(Vec::new())),
        results: Arc::new(Mutex::new(Vec::new())),
        http_egress: None,
        network_egress: None,
        real_egress_transport: None,
        process_port: None,
        profile_filesystem: None,
        project_service: None,
        skill_activation_source: None,
        attachment_test_support: None,
        inbound_attachment_reader: None,
        outbound_target_tools: None,
        scope_capability_by_run_owner: false,
        product_auth: None,
        tool_permission_overrides: None,
        persistent_approval_policies: None,
        trigger_repository: None,
        reborn_services: None,
    })
}
