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
    CapabilityId, DispatchError, DispatchFailureDetail, ExtensionId, ReservationStatus,
    ResourceReceipt, ResourceUsage, RuntimeDispatchErrorKind, RuntimeKind, ToolCall,
    ToolCallResources, ToolError, ToolPorts,
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
        // FirstParty/System carry the raw cause on the Diagnostic detail
        // channel (untrusted-provenance text, scrubbed downstream) rather than
        // dropping it — the lane arms' `model_visible_cause` equivalent for the
        // detail-shaped variant. A fixed host-authored summary, if that is all
        // the adapter gave, still travels on `safe_summary`.
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty {
            kind,
            safe_summary,
            detail: model_visible_cause.map(|text| DispatchFailureDetail::Diagnostic { text }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause_of(error: &DispatchError) -> Option<&str> {
        match error {
            DispatchError::Wasm {
                model_visible_cause,
                ..
            }
            | DispatchError::Mcp {
                model_visible_cause,
                ..
            }
            | DispatchError::Script {
                model_visible_cause,
                ..
            } => model_visible_cause.as_deref(),
            DispatchError::FirstParty {
                detail: Some(DispatchFailureDetail::Diagnostic { text }),
                ..
            } => Some(text.as_str()),
            _ => None,
        }
    }

    /// The generic extension lanes must carry a failing adapter's
    /// `model_visible_cause` across the tool ABI onto the dispatch error —
    /// including the FirstParty/System arm, which routes it to the Diagnostic
    /// detail channel rather than dropping it (#5965 on the extension path).
    #[test]
    fn tool_error_cause_survives_every_lane() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        for runtime in [
            RuntimeKind::Wasm,
            RuntimeKind::Mcp,
            RuntimeKind::Script,
            RuntimeKind::FirstParty,
            RuntimeKind::System,
        ] {
            let error = ToolError::Failed {
                kind: RuntimeDispatchErrorKind::Backend,
                safe_summary: None,
                model_visible_cause: Some("channel_not_found".to_string()),
            };
            let dispatch = dispatch_error_for_tool_error(&cap, runtime, error);
            assert_eq!(
                cause_of(&dispatch),
                Some("channel_not_found"),
                "lane {runtime:?} dropped the model-visible cause"
            );
        }
    }

    /// When the adapter supplied only a fixed host-authored `safe_summary`
    /// (no raw cause), the lane arms still surface it on the cause channel so
    /// the failure keeps an actionable label instead of collapsing to the
    /// kind's generic sentence.
    #[test]
    fn lane_summary_rides_the_cause_channel_when_no_raw_cause() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let error = ToolError::Failed {
            kind: RuntimeDispatchErrorKind::Backend,
            safe_summary: Some("vendor unavailable".to_string()),
            model_visible_cause: None,
        };
        let dispatch = dispatch_error_for_tool_error(&cap, RuntimeKind::Wasm, error);
        assert_eq!(cause_of(&dispatch), Some("vendor unavailable"));
    }
}
