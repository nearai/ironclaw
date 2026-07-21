//! The active-snapshot [`ToolResolver`]: dispatch resolves activated
//! extension capabilities from the published generation (overview.md §5.2).
//!
//! Resolution is a lookup into the immutable snapshot the lifecycle host
//! published; in-flight dispatches keep the binding they resolved even
//! across a concurrent upgrade/removal swap. The resolved [`ToolAdapter`] is
//! behavior-only, so this module also owns the dispatch-side wrapper that
//! carries the host bookkeeping across the ABI.
//!
//! Resource-settlement invariant: every `ToolAdapter` published in an
//! `ActiveExtension` settles a forwarded reservation exactly once
//! (lane-backed adapters settle inside their runtime lane; native factory
//! adapters are wrapped in the composition loader's settling decorator).
//! The wrapper therefore forwards the prepared reservation verbatim and
//! synthesizes the result bookkeeping from re-measured output bytes — the
//! receipt has no consumer above the dispatcher, and upstream usage reads
//! only `output_bytes`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_dispatcher::{
    BoundCapabilityAdapter, CapabilityDispatchRequest, ResolvedCapability, RuntimeAdapterResult,
    ToolResolver,
};
use ironclaw_host_api::{
    CapabilityId, DispatchError, ExtensionId, ReservationStatus, ResourceReceipt, ResourceUsage,
    RuntimeDispatchErrorKind, RuntimeKind, ToolCall, ToolCallResources, ToolError, ToolPorts,
};

use crate::active::ResolvedToolBinding;
use crate::lifecycle::SnapshotWatch;

/// Resolves prebound tool bindings from the currently published
/// [`crate::ActiveSnapshot`].
pub struct SnapshotToolResolver {
    watch: SnapshotWatch,
}

impl SnapshotToolResolver {
    pub fn new(watch: SnapshotWatch) -> Self {
        Self { watch }
    }
}

impl ToolResolver for SnapshotToolResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        let snapshot = self.watch.current();
        let binding = snapshot.resolve_tool(capability_id)?;
        let provider = ExtensionId::new(binding.declaration.id.as_str()).ok()?;
        let runtime = binding.declaration.runtime.kind();
        Some(ResolvedCapability {
            provider,
            runtime,
            adapter: Arc::new(SnapshotBoundCapability { binding, runtime }),
        })
    }
}

/// Dispatch-side wrapper over one resolved [`ToolAdapter`] binding.
struct SnapshotBoundCapability {
    binding: ResolvedToolBinding,
    runtime: RuntimeKind,
}

#[async_trait]
impl BoundCapabilityAdapter for SnapshotBoundCapability {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let capability_id = request.capability_id.clone();
        let scope = request.scope.clone();
        let estimate = request.estimate.clone();
        let reservation_id = request
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);
        let call = ToolCall {
            capability_id: request.capability_id,
            scope: request.scope,
            input: request.input,
            deadline: None,
            resources: ToolCallResources {
                estimate: request.estimate,
                mounts: request.mounts,
                reservation: request.resource_reservation,
            },
        };
        // Ports are derived from the resolved declaration, nothing wider; the
        // restricted-egress port lands with its first native consumer (the
        // extracted channel crates) — lane-backed adapters reach the network
        // through their staged host-egress pipeline, never through ports.
        let ports = ToolPorts { egress: None };
        let result = self
            .binding
            .adapter
            .invoke(call, &ports)
            .await
            .map_err(|error| dispatch_error_for_tool_error(&capability_id, self.runtime, error))?;

        // The adapter's byte count is advisory; re-measure for enforcement.
        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(result.output_bytes);
        let usage = ResourceUsage {
            output_bytes,
            ..ResourceUsage::default()
        };
        Ok(RuntimeAdapterResult {
            output: result.output,
            display_preview: result.display_preview,
            output_bytes,
            usage: usage.clone(),
            receipt: ResourceReceipt {
                id: reservation_id.unwrap_or_default(),
                scope,
                status: ReservationStatus::Reconciled,
                estimate,
                actual: Some(usage),
            },
        })
    }
}

/// Map a [`ToolError`] onto the dispatch port's redacted categories, shaped
/// by the binding's runtime kind so the error surface matches the lane the
/// capability runs on.
fn dispatch_error_for_tool_error(
    capability_id: &CapabilityId,
    runtime: RuntimeKind,
    error: ToolError,
) -> DispatchError {
    match error {
        ToolError::AuthRequired {
            required_secrets,
            credential_requirements,
        } => DispatchError::AuthRequired {
            capability: capability_id.clone(),
            required_secrets,
            credential_requirements,
        },
        ToolError::InvalidInput { .. } => {
            dispatch_error_for_kind(runtime, RuntimeDispatchErrorKind::InputEncode, None, None)
        }
        ToolError::Failed {
            kind,
            safe_summary,
            model_visible_cause,
        } => dispatch_error_for_kind(runtime, kind, safe_summary, model_visible_cause),
    }
}

fn dispatch_error_for_kind(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
    safe_summary: Option<String>,
    model_visible_cause: Option<String>,
) -> DispatchError {
    match runtime {
        // The lane variants carry the cause on `model_visible_cause` (#5965):
        // raw-or-better cause text, scrubbed downstream at the model-visible
        // Diagnostic seam. When an adapter supplied only a fixed host-authored
        // summary, that text is trivially cause-safe, so it rides the same
        // channel rather than being dropped.
        RuntimeKind::Wasm => DispatchError::Wasm {
            kind,
            model_visible_cause: model_visible_cause.or(safe_summary),
        },
        RuntimeKind::Mcp => DispatchError::Mcp {
            kind,
            model_visible_cause: model_visible_cause.or(safe_summary),
        },
        RuntimeKind::Script => DispatchError::Script {
            kind,
            model_visible_cause: model_visible_cause.or(safe_summary),
        },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty {
            kind,
            safe_summary,
            detail: None,
        },
    }
}
