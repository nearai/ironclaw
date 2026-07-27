//! Extension tool binding over the host runtime lanes.
//!
//! The generic extension host (`ironclaw_extension_host`) loads WASM / hosted
//! MCP / first-party-registry extensions through synthesized
//! [`ToolAdapter`]s — the extension ships no host Rust (LIFE-4). The lanes
//! themselves are host-runtime-private; this binder is the narrow sanctioned
//! surface that prebinds one package to its lane and returns the adapter,
//! without exposing lane types, the registry, the filesystem, or the
//! governor.
//!
//! Resource accounting contract: a lane-backed adapter forwards the prepared
//! reservation from `ToolCall::resources` into the lane, which settles it
//! (reconcile-or-release — the same legs the lanes always had). The
//! usage/receipt bookkeeping is dropped at the [`ToolAdapter`] ABI by design;
//! the dispatch-side wrapper re-measures output bytes for its result.

use std::collections::HashMap;
use std::sync::Arc;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, DispatchError, RuntimeKind, RuntimeLane, ToolAdapter,
    ToolCall, ToolError, ToolPorts, ToolResult, runtime_policy::EffectiveRuntimePolicy,
};
use ironclaw_resources::ResourceGovernor;

use super::RootFilesystem;
use super::runtime_adapters::{RuntimeLaneExecutor, RuntimeLaneRequest};

/// Binds extension packages to their runtime lanes, yielding one
/// [`ToolAdapter`] per extension (the adapter routes internally by
/// capability id).
#[derive(Clone)]
pub struct ExtensionLaneToolBinder {
    inner: Arc<dyn LanePackageBinder>,
}

impl ExtensionLaneToolBinder {
    pub(super) fn new(inner: Arc<dyn LanePackageBinder>) -> Self {
        Self { inner }
    }

    /// Prebind one package to its lane. Fails with a typed error when the
    /// package's runtime kind has no configured lane in this composition.
    pub fn bind_package(
        &self,
        package: Arc<ExtensionPackage>,
    ) -> Result<Arc<dyn ToolAdapter>, ExtensionToolBindError> {
        self.inner.bind(package)
    }
}

/// Typed binding failures surfaced to the extension host's loader.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionToolBindError {
    #[error("no runtime backend is configured for {runtime:?} extensions")]
    MissingRuntimeBackend { runtime: RuntimeKind },
}

pub(super) trait LanePackageBinder: Send + Sync {
    fn bind(
        &self,
        package: Arc<ExtensionPackage>,
    ) -> Result<Arc<dyn ToolAdapter>, ExtensionToolBindError>;
}

/// The generic-per-composition binder: captures the configured lanes plus the
/// statics every lane invocation needs (filesystem, governor, runtime
/// policy).
pub(super) struct ServiceLanePackageBinder<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    pub(super) executor: Arc<RuntimeLaneExecutor<F, G>>,
    pub(super) filesystem: Arc<F>,
    pub(super) governor: Arc<G>,
    pub(super) runtime_policy: EffectiveRuntimePolicy,
}

impl<F, G> LanePackageBinder for ServiceLanePackageBinder<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    fn bind(
        &self,
        package: Arc<ExtensionPackage>,
    ) -> Result<Arc<dyn ToolAdapter>, ExtensionToolBindError> {
        let runtime = package.manifest.runtime_kind();
        let lane = RuntimeLane::from_runtime_kind(runtime)
            .filter(|lane| self.executor.supports_lane(*lane))
            .ok_or(ExtensionToolBindError::MissingRuntimeBackend { runtime })?;
        let descriptors: HashMap<CapabilityId, Arc<CapabilityDescriptor>> = package
            .capabilities
            .iter()
            .map(|descriptor| (descriptor.id.clone(), Arc::new(descriptor.clone())))
            .collect();
        Ok(Arc::new(LaneBackedToolAdapter {
            package,
            descriptors,
            lane,
            executor: Arc::clone(&self.executor),
            filesystem: Arc::clone(&self.filesystem),
            governor: Arc::clone(&self.governor),
            runtime_policy: self.runtime_policy.clone(),
        }))
    }
}

/// One extension's lane-backed adapter: routes by capability id to the
/// prebound descriptor and invokes the captured lane.
struct LaneBackedToolAdapter<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    package: Arc<ExtensionPackage>,
    descriptors: HashMap<CapabilityId, Arc<CapabilityDescriptor>>,
    lane: RuntimeLane,
    executor: Arc<RuntimeLaneExecutor<F, G>>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    runtime_policy: EffectiveRuntimePolicy,
}

#[async_trait::async_trait]
impl<F, G> ToolAdapter for LaneBackedToolAdapter<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    async fn invoke(
        &self,
        call: ToolCall,
        _ports: &ToolPorts<'_>,
    ) -> Result<ToolResult, ToolError> {
        let Some(descriptor) = self.descriptors.get(&call.capability_id) else {
            return Err(ToolError::Failed {
                kind: ironclaw_host_api::RuntimeDispatchErrorKind::UndeclaredCapability,
                safe_summary: None,
                model_visible_cause: None,
            });
        };
        let execution = self
            .executor
            .dispatch_json(
                self.lane,
                RuntimeLaneRequest {
                    package: &self.package,
                    descriptor,
                    filesystem: self.filesystem.as_ref(),
                    governor: self.governor.as_ref(),
                    runtime_policy: &self.runtime_policy,
                    capability_id: &call.capability_id,
                    scope: call.scope,
                    // ToolCall carries actor authority within `scope`; this adapter
                    // interface has no separate human-actor handle.
                    authenticated_actor_user_id: None,
                    // ToolCall does not carry loop turn-run identity either; only
                    // the first-party coding lane consumes `run_id` today.
                    run_id: None,
                    // Legacy extension-host calls do not carry a sealed
                    // invocation origin. They cannot claim scheduled-loop
                    // lineage; origin-sensitive first-party policy therefore
                    // sees `None` rather than a fabricated classification.
                    origin: None,
                    estimate: call.resources.estimate,
                    mounts: call.resources.mounts,
                    resource_reservation: call.resources.reservation,
                    input: call.input,
                },
            )
            .await
            .map_err(tool_error_from_dispatch)?;
        Ok(ToolResult {
            output: execution.output,
            display_preview: execution.display_preview,
            output_bytes: execution.output_bytes,
        })
    }
}

/// Map a lane failure onto the tool ABI. Lane errors are already redacted to
/// stable kinds; `AuthRequired` keeps its gate payload so the generic re-auth
/// flow is preserved end to end.
fn tool_error_from_dispatch(error: DispatchError) -> ToolError {
    match error {
        DispatchError::AuthRequired {
            required_secrets,
            credential_requirements,
            ..
        } => ToolError::AuthRequired {
            required_secrets,
            credential_requirements,
        },
        DispatchError::Wasm {
            kind,
            model_visible_cause,
        }
        | DispatchError::Mcp {
            kind,
            model_visible_cause,
        }
        | DispatchError::Script {
            kind,
            model_visible_cause,
        } => ToolError::Failed {
            kind,
            safe_summary: None,
            model_visible_cause,
        },
        DispatchError::FirstParty {
            kind, safe_summary, ..
        } => ToolError::Failed {
            kind,
            safe_summary,
            model_visible_cause: None,
        },
        other => ToolError::Failed {
            kind: ironclaw_host_api::RuntimeDispatchErrorKind::Client,
            safe_summary: Some(other.event_kind().replace('_', " ")),
            model_visible_cause: None,
        },
    }
}
