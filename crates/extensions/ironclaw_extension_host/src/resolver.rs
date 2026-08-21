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
    dispatch::{DispatchError, DispatchFailureDetail, DispatchFailureKind, ProviderDiagnostic},
    ids::{CapabilityId, ExtensionId},
    messaging::StandardMessagingErrorCode,
    resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
    runtime::RuntimeKind,
    safe_summary::SafeSummary,
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
        ToolError::AuthRequired { requirement } => DispatchError::AuthRequired {
            capability: capability_id.clone(),
            requirement,
        },
        ToolError::Rejected {
            kind,
            diagnostic,
            detail,
            ..
        } => {
            let messaging_code = standard_messaging_code(diagnostic.as_deref());
            DispatchError::Rejected {
                // Runtime provenance belongs to the trusted resolved binding,
                // not to the extension-supplied error payload.
                runtime: Some(runtime),
                kind: DispatchFailureKind::Runtime(kind),
                diagnostic,
                detail: dispatch_detail_for_tool_error(messaging_code, detail),
            }
        }
    }
}

/// Resolve the only trusted text an extension may select — a closed standard
/// messaging code — after the adapter boundary. Raw host-summary variants are
/// intentionally discarded here because an extension cannot attest to host
/// authorship. Other detail variants retain their existing untrusted/typed
/// semantics and continue through the downstream scrubbers.
fn dispatch_detail_for_tool_error(
    messaging_code: Option<StandardMessagingErrorCode>,
    detail: Option<DispatchFailureDetail>,
) -> Option<DispatchFailureDetail> {
    messaging_code
        .map(trusted_standard_messaging_summary)
        .or_else(|| extension_detail_without_host_summary(detail))
}

/// Recognize only an exact member of the closed standard-messaging code set.
/// Provider text never undergoes substring or keyword interpretation here.
fn standard_messaging_code(
    diagnostic: Option<&ProviderDiagnostic>,
) -> Option<StandardMessagingErrorCode> {
    let code = diagnostic?.code.as_ref()?.as_str();
    StandardMessagingErrorCode::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == code)
}

fn trusted_standard_messaging_summary(code: StandardMessagingErrorCode) -> DispatchFailureDetail {
    let summary = match SafeSummary::new(format!("messaging operation failed: {}", code.as_str())) {
        Ok(summary) => summary,
        Err(_) => SafeSummary::placeholder(),
    };
    DispatchFailureDetail::HostSummary {
        summary,
        detail: None,
    }
}

fn extension_detail_without_host_summary(
    detail: Option<DispatchFailureDetail>,
) -> Option<DispatchFailureDetail> {
    match detail {
        Some(DispatchFailureDetail::HostSummary { .. })
        | Some(DispatchFailureDetail::HostRemediation { .. }) => None,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        dispatch::RuntimeDispatchErrorKind, messaging::StandardMessagingErrorCode,
    };

    fn cause_of(error: &DispatchError) -> Option<&str> {
        match error {
            DispatchError::Rejected {
                diagnostic: Some(diagnostic),
                ..
            } => diagnostic.message.as_ref().map(|message| message.as_str()),
            _ => None,
        }
    }

    #[test]
    fn standard_messaging_code_becomes_a_host_summary_at_the_resolver_boundary() {
        let cap = CapabilityId::new("telegram.send_message").unwrap();
        let dispatch = dispatch_error_for_tool_error(
            &cap,
            RuntimeKind::FirstParty,
            ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::OperationFailed,
                diagnostic: Some(Box::new(ironclaw_host_api::dispatch::ProviderDiagnostic {
                    code: Some(ironclaw_host_api::dispatch::ProviderErrorCode::new(
                        StandardMessagingErrorCode::RateLimited.as_str(),
                    )),
                    message: None,
                    retry_after: None,
                })),
                detail: None,
            },
        );
        let DispatchError::Rejected { detail, .. } = dispatch else {
            panic!("expected Rejected dispatch error");
        };
        assert!(matches!(
            detail,
            Some(ironclaw_host_api::dispatch::DispatchFailureDetail::HostSummary {
                summary,
                detail: None,
            }) if summary.as_str().contains(StandardMessagingErrorCode::RateLimited.as_str())
        ));
    }

    #[test]
    fn extension_host_remediation_is_not_trusted_at_the_resolver_boundary() {
        let detail = DispatchFailureDetail::HostRemediation {
            text: ironclaw_host_api::host_remediation::HostRemediation::new("connect the account")
                .unwrap(),
        };

        assert!(extension_detail_without_host_summary(Some(detail)).is_none());
    }

    /// The generic extension lanes must carry a provider diagnostic across the
    /// tool ABI onto the dispatch error — including the FirstParty/System arm,
    /// which routes it to the diagnostic channel rather than dropping it.
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
            let error = ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::Backend,
                diagnostic: Some(Box::new(ironclaw_host_api::dispatch::ProviderDiagnostic {
                    code: None,
                    message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                        "channel_not_found",
                    )),
                    retry_after: None,
                })),
                detail: None,
            };
            let dispatch = dispatch_error_for_tool_error(&cap, runtime, error);
            assert_eq!(
                cause_of(&dispatch),
                Some("channel_not_found"),
                "lane {runtime:?} dropped the model-visible cause"
            );
        }
    }

    /// An adapter cannot attest to host authorship, so a summary supplied in
    /// the tool error is discarded when no closed standard code is present.
    #[test]
    fn lane_summary_is_dropped_when_no_closed_code_is_present() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let error = ToolError::Rejected {
            kind: RuntimeDispatchErrorKind::Backend,
            diagnostic: None,
            detail: Some(
                ironclaw_host_api::dispatch::DispatchFailureDetail::HostSummary {
                    summary: ironclaw_host_api::safe_summary::SafeSummary::new(
                        "vendor unavailable",
                    )
                    .unwrap(),
                    detail: None,
                },
            ),
        };
        let dispatch = dispatch_error_for_tool_error(&cap, RuntimeKind::Wasm, error);
        let DispatchError::Rejected {
            diagnostic, detail, ..
        } = dispatch
        else {
            panic!("expected Rejected dispatch error");
        };
        assert!(diagnostic.is_none());
        assert!(detail.is_none());
    }

    #[test]
    fn rejected_tool_error_drops_extension_summary_but_keeps_vendor_cause() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let dispatch = dispatch_error_for_tool_error(
            &cap,
            RuntimeKind::FirstParty,
            ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::Backend,
                diagnostic: Some(Box::new(ironclaw_host_api::dispatch::ProviderDiagnostic {
                    code: None,
                    message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                        "vendor backend returned 503",
                    )),
                    retry_after: None,
                })),
                detail: Some(
                    ironclaw_host_api::dispatch::DispatchFailureDetail::HostSummary {
                        summary: ironclaw_host_api::safe_summary::SafeSummary::new(
                            "the tool's backend failed",
                        )
                        .unwrap(),
                        detail: None,
                    },
                ),
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
        assert!(detail.is_none());
    }

    #[test]
    fn rejected_tool_error_runtime_comes_from_the_outer_binding() {
        let cap = CapabilityId::new("acme.cap").unwrap();
        let dispatch = dispatch_error_for_tool_error(
            &cap,
            RuntimeKind::FirstParty,
            ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::Backend,
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
