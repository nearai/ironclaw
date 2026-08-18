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
use ironclaw_capabilities::{
    BoundCapabilityAdapter, CapabilityDispatchRequest, ResolvedCapability, RuntimeAdapterResult,
    ToolResolver,
};
use ironclaw_extension_contracts::tool_adapter::{
    ToolCall, ToolCallResources, ToolError, ToolPorts,
};
use ironclaw_host_api::{
    dispatch::{DispatchError, DispatchFailureKind, RuntimeDispatchErrorKind},
    ids::{CapabilityId, ExtensionId},
    resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
    runtime::RuntimeKind,
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
            model_visible_cause,
        } => DispatchError::AuthRequired {
            capability: capability_id.clone(),
            required_secrets,
            credential_requirements,
            model_visible_cause: model_visible_cause.map(Box::new),
        },
        ToolError::Rejected {
            kind,
            diagnostic,
            detail,
            ..
        } => DispatchError::Rejected {
            // Runtime provenance belongs to the trusted resolved binding, not
            // to the extension-supplied error payload.
            runtime: Some(runtime),
            kind,
            diagnostic,
            detail,
        },
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
    // Keep the caller's host-authored summary distinct from the provider cause:
    // the former is the public label, while the latter remains on the typed
    // diagnostic channel for downstream scrubbing.
    let error = DispatchError::provider_rejected(
        Some(runtime),
        DispatchFailureKind::Runtime(kind),
        model_visible_cause,
    );
    match safe_summary {
        Some(text) => error.with_host_summary(text),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause_of(error: &DispatchError) -> Option<&str> {
        match error {
            DispatchError::Rejected {
                diagnostic: Some(diagnostic),
                ..
            } => diagnostic.message.as_ref().map(|message| message.as_str()),
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
    /// (no raw cause), the lane arms preserve it on the host-summary channel
    /// instead of collapsing to the kind's generic sentence.
    #[test]
    fn lane_summary_stays_on_the_host_summary_channel_when_no_raw_cause() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let error = ToolError::Failed {
            kind: RuntimeDispatchErrorKind::Backend,
            safe_summary: Some("vendor unavailable".to_string()),
            model_visible_cause: None,
        };
        let dispatch = dispatch_error_for_tool_error(&cap, RuntimeKind::Wasm, error);
        let DispatchError::Rejected {
            diagnostic, detail, ..
        } = dispatch
        else {
            panic!("expected Rejected dispatch error");
        };
        assert!(diagnostic.is_none());
        assert!(matches!(
            detail,
            Some(ironclaw_host_api::dispatch::DispatchFailureDetail::HostSummary { text })
                if text == "vendor unavailable"
        ));
    }

    #[test]
    fn failed_tool_error_keeps_host_summary_separate_from_vendor_cause() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let dispatch = dispatch_error_for_tool_error(
            &cap,
            RuntimeKind::FirstParty,
            ToolError::Failed {
                kind: RuntimeDispatchErrorKind::Backend,
                safe_summary: Some("the tool's backend failed".to_string()),
                model_visible_cause: Some("vendor backend returned 503".to_string()),
            },
        );

        let DispatchError::Rejected {
            runtime,
            diagnostic,
            detail,
            ..
        } = dispatch
        else {
            panic!("expected Rejected dispatch error");
        };
        assert_eq!(runtime, Some(RuntimeKind::FirstParty));
        assert_eq!(
            diagnostic
                .as_ref()
                .and_then(|diagnostic| diagnostic.message.as_ref())
                .map(|message| message.as_str()),
            Some("vendor backend returned 503")
        );
        assert!(matches!(
            detail,
            Some(ironclaw_host_api::dispatch::DispatchFailureDetail::HostSummary { text })
                if text == "the tool's backend failed"
        ));
    }

    #[test]
    fn rejected_tool_error_runtime_comes_from_the_outer_binding() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let dispatch = dispatch_error_for_tool_error(
            &cap,
            RuntimeKind::FirstParty,
            ToolError::Rejected {
                runtime: Some(RuntimeKind::Wasm),
                kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend),
                diagnostic: None,
                detail: None,
            },
        );

        let DispatchError::Rejected { runtime, .. } = dispatch else {
            panic!("expected Rejected dispatch error");
        };
        assert_eq!(runtime, Some(RuntimeKind::FirstParty));
    }
}
