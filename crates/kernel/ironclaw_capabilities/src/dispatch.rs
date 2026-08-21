//! Composition-only runtime dispatch contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` routes already-authorized capability invocations to
//! prebound adapters resolved by capability id through the injected
//! [`ToolResolver`]. It does not select packages or runtime kinds, parse
//! extension manifests, implement sandbox policy, reserve budget itself, or
//! execute product workflows. Binding construction (which adapter serves a
//! capability, with which package, plan, and ports) happens at
//! activation/registration time in the resolver implementations: the host
//! built-in registry resolver in `ironclaw_host_runtime` and the
//! active-snapshot resolver over `ironclaw_extension_host`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_log::{EventSink, RuntimeEvent};
use ironclaw_host_api::{
    artifact::{
        ARTIFACT_INLINE_PREVIEW_MAX_BYTES, AccountedArtifactPersister, ArtifactOwnerScope,
        ArtifactWriteMetadata, CompletedArtifact,
    },
    authorized::Authorized,
    dispatch::{
        CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
        CapabilityDisplayOutputPreview, DispatchError, DispatchFailureKind,
        RuntimeDispatchErrorKind,
    },
    ids::{CapabilityId, ExtensionId, InvocationId},
    invocation::{Actor, InvocationOrigin},
    lane::RuntimeLane,
    resource::{ResourceReceipt, ResourceReservation, ResourceScope, ResourceUsage},
    result_meta::OutputDigest,
    runtime::RuntimeKind,
};
use ironclaw_loop_contracts::ContentDigest;
use ironclaw_resources::ResourceGovernor;
use serde_json::Value;

enum ServiceHandle<'a, T>
where
    T: ?Sized,
{
    Borrowed(&'a T),
    Shared(Arc<T>),
}

impl<T> ServiceHandle<'_, T>
where
    T: ?Sized,
{
    fn as_ref(&self) -> &T {
        match self {
            Self::Borrowed(value) => value,
            Self::Shared(value) => value.as_ref(),
        }
    }
}

/// Resolve a prebound capability binding by capability id (TOOL-1).
///
/// Implementations are snapshot-shaped: resolution is a lookup into bindings
/// constructed at activation/registration time, never a per-invocation
/// package/runtime-kind selection. An unknown id returns `None` and dispatch
/// fails before any adapter work (TOOL-2).
pub trait ToolResolver: Send + Sync {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability>;
}

/// One prebound, ready-to-invoke capability binding.
#[derive(Clone)]
pub struct ResolvedCapability {
    /// The owning extension (host built-ins resolve as the synthetic
    /// `builtin` provider).
    pub provider: ExtensionId,
    /// The implementation lane, carried for dispatch events and results;
    /// selection already happened when the binding was constructed.
    pub runtime: RuntimeKind,
    pub adapter: Arc<dyn BoundCapabilityAdapter>,
}

/// A prebound capability implementation behind [`ToolResolver`].
///
/// The adapter receives the already-authorized [`CapabilityDispatchRequest`]
/// unchanged — everything static (package, descriptor, execution plan,
/// filesystem, ports) was captured when the binding was constructed. If
/// `resource_reservation` is present, the binding owns the
/// reconcile-or-release leg for it (same legs as the runtime lanes always
/// had); the dispatcher only releases a reservation when resolution fails
/// before any binding takes it.
///
/// Implementations must not perform caller-facing authorization or approval
/// resolution and must surface only redacted [`DispatchError`] categories.
#[async_trait]
pub trait BoundCapabilityAdapter: Send + Sync {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError>;
}

/// Runtime-normalized adapter result before the dispatcher adds stable
/// identity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdapterResult {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
    pub usage: ResourceUsage,
    pub receipt: ResourceReceipt,
    pub output_bytes: u64,
    pub completed_artifact: Option<CompletedArtifact>,
    pub canonical_output_digest: Option<OutputDigest>,
}

/// First-`Some`-wins composition of resolvers (host built-ins chained with
/// the active extension snapshot).
pub struct ChainToolResolver {
    resolvers: Vec<Arc<dyn ToolResolver>>,
}

impl ChainToolResolver {
    pub fn new(resolvers: Vec<Arc<dyn ToolResolver>>) -> Self {
        Self { resolvers }
    }
}

impl ToolResolver for ChainToolResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        self.resolvers
            .iter()
            .find_map(|resolver| resolver.resolve(capability_id))
    }
}

/// Narrow runtime dispatcher over prebound capability bindings.
pub struct RuntimeDispatcher<'a, G>
where
    G: ResourceGovernor,
{
    resolver: ServiceHandle<'a, dyn ToolResolver + 'a>,
    governor: ServiceHandle<'a, G>,
    event_sink: Option<ServiceHandle<'a, dyn EventSink + 'a>>,
    artifact_persistence: Option<Arc<dyn AccountedArtifactPersister>>,
}

impl<'a, G> RuntimeDispatcher<'a, G>
where
    G: ResourceGovernor,
{
    pub fn new(resolver: &'a dyn ToolResolver, governor: &'a G) -> Self {
        Self {
            resolver: ServiceHandle::Borrowed(resolver),
            governor: ServiceHandle::Borrowed(governor),
            event_sink: None,
            artifact_persistence: None,
        }
    }

    pub fn from_arcs(
        resolver: Arc<dyn ToolResolver>,
        governor: Arc<G>,
    ) -> RuntimeDispatcher<'static, G>
    where
        G: 'static,
    {
        RuntimeDispatcher {
            resolver: ServiceHandle::Shared(resolver),
            governor: ServiceHandle::Shared(governor),
            event_sink: None,
            artifact_persistence: None,
        }
    }

    pub fn with_event_sink(mut self, sink: &'a dyn EventSink) -> Self {
        self.event_sink = Some(ServiceHandle::Borrowed(sink));
        self
    }

    pub fn with_event_sink_arc(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = Some(ServiceHandle::Shared(sink));
        self
    }

    pub fn with_artifact_persistence_arc(
        mut self,
        persistence: Arc<dyn AccountedArtifactPersister>,
    ) -> Self {
        self.artifact_persistence = Some(persistence);
        self
    }

    #[tracing::instrument(level = "debug", skip(self, authorized), fields(capability_id, scope))]
    pub async fn dispatch_json(
        &self,
        authorized: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let (invocation, lane, mounts, resource_reservation) =
            match authorized.into_parts(chrono::Utc::now()) {
                Ok(parts) => parts,
                Err(authorized) => {
                    let capability = authorized.invocation().capability.clone();
                    let reservation = authorized.abort();
                    drop(DispatchReservationGuard::new(
                        self.governor.as_ref(),
                        reservation,
                    ));
                    return Err(DispatchError::AuthorizationExpired { capability });
                }
            };
        let scope = invocation.scope.clone();
        let capability_id = invocation.capability.clone();
        tracing::Span::current().record("capability_id", tracing::field::display(&capability_id));
        tracing::Span::current().record("scope", tracing::field::debug(&scope));
        let authenticated_actor_user_id = match &invocation.actor {
            Actor::Sealed(user_id) => Some(user_id.clone()),
            Actor::System => None,
        };
        let origin = invocation.origin.clone();
        let run_id = match invocation.origin {
            InvocationOrigin::LoopRun(run_id) | InvocationOrigin::ScheduledLoopRun(run_id)
                if invocation.process_id.is_none() =>
            {
                Some(run_id)
            }
            _ => None,
        };
        let parent_invocation_id = run_id.map(|run_id| InvocationId::from_uuid(run_id.as_uuid()));
        let artifact_namespace = invocation.artifact_namespace;
        let mut request = CapabilityDispatchRequest {
            artifact_namespace,
            capability_id: invocation.capability,
            scope: invocation.scope,
            origin,
            authenticated_actor_user_id,
            run_id,
            estimate: invocation.estimate,
            mounts,
            resource_reservation,
            input: invocation.input,
        };
        let mut reservation_guard = DispatchReservationGuard::new(
            self.governor.as_ref(),
            request.resource_reservation.take(),
        );
        let mut event = RuntimeEvent::dispatch_requested(scope.clone(), capability_id.clone());
        event.parent_invocation_id = parent_invocation_id;
        self.emit_event(event).await?;

        let Some(resolved) = self.resolver.as_ref().resolve(&request.capability_id) else {
            let error = DispatchError::UnknownCapability {
                capability: capability_id.clone(),
            };
            self.emit_dispatch_failure(
                scope,
                capability_id,
                None,
                None,
                parent_invocation_id,
                &error,
            )
            .await?;
            return Err(error);
        };
        let provider = resolved.provider.clone();
        let runtime = resolved.runtime;
        if RuntimeLane::from_runtime_kind(runtime) != Some(lane) {
            let error = DispatchError::MissingRuntimeBackend { runtime };
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(provider),
                Some(runtime),
                parent_invocation_id,
                &error,
            )
            .await?;
            return Err(error);
        }

        if let Err(error) = reservation_guard.validate() {
            let error = dispatch_resource_error(runtime, error);
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(provider),
                Some(runtime),
                parent_invocation_id,
                &error,
            )
            .await?;
            return Err(error);
        }

        let mut event = RuntimeEvent::runtime_selected(
            scope.clone(),
            capability_id.clone(),
            provider.clone(),
            runtime,
        );
        event.parent_invocation_id = parent_invocation_id;
        self.emit_event(event).await?;

        if scope.agent_id.is_some()
            && (artifact_namespace.is_none() || self.artifact_persistence.is_none())
        {
            let error = dispatch_resource_error(
                runtime,
                ironclaw_resources::ResourceError::Storage {
                    reason: "agent-scoped runtime dispatch requires artifact persistence"
                        .to_string(),
                },
            );
            self.emit_dispatch_failure(
                scope,
                capability_id,
                Some(provider),
                Some(runtime),
                parent_invocation_id,
                &error,
            )
            .await?;
            return Err(error);
        }

        let mut execution = match resolved
            .adapter
            .dispatch_json(CapabilityDispatchRequest {
                resource_reservation: reservation_guard.take(),
                ..request
            })
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                self.emit_dispatch_failure(
                    scope,
                    capability_id,
                    Some(provider),
                    Some(runtime),
                    parent_invocation_id,
                    &error,
                )
                .await?;
                return Err(error);
            }
        };

        let mut canonical_output_digest = execution.canonical_output_digest;
        let canonical_item_count = execution.output.as_array().map(|items| items.len() as u64);
        let finalize_result: Result<(), DispatchError> = async {
            let canonical_bytes = serde_json::to_vec(&execution.output).map_err(|_| {
                dispatch_resource_error(
                    runtime,
                    ironclaw_resources::ResourceError::Storage {
                        reason: "completed runtime output could not be serialized".to_string(),
                    },
                )
            })?;
            let canonical_len = u64::try_from(canonical_bytes.len()).map_err(|_| {
                dispatch_resource_error(
                    runtime,
                    ironclaw_resources::ResourceError::Storage {
                        reason: "completed runtime output length is unsupported".to_string(),
                    },
                )
            })?;
            let accounted_len = execution
                .completed_artifact
                .as_ref()
                .map_or(canonical_len, |artifact| artifact.byte_len);
            if execution
                .receipt
                .actual
                .as_ref()
                .map(|usage| usage.output_bytes)
                != Some(accounted_len)
            {
                return Err(dispatch_resource_error(
                    runtime,
                    ironclaw_resources::ResourceError::Storage {
                        reason: "completed runtime accounting does not match durable output"
                            .to_string(),
                    },
                ));
            }
            if canonical_output_digest.is_none() {
                canonical_output_digest = Some(
                    ContentDigest::from_json_value(&execution.output)
                        .map(|digest| OutputDigest::new(digest.0))
                        .map_err(|_| {
                            dispatch_resource_error(
                                runtime,
                                ironclaw_resources::ResourceError::Storage {
                                    reason: "completed runtime output digest could not be computed"
                                        .to_string(),
                                },
                            )
                        })?,
                );
            }
            execution.output_bytes = accounted_len;
            execution.usage.output_bytes = accounted_len;
            if execution.completed_artifact.is_none()
                && let (Some(namespace), Some(persistence)) =
                    (artifact_namespace, self.artifact_persistence.as_ref())
            {
                execution.completed_artifact = Some(
                    persistence
                        .persist(
                            ArtifactWriteMetadata {
                                write_key: None,
                                owner_scope: ArtifactOwnerScope::from_resource_scope(
                                    &execution.receipt.scope,
                                ),
                                namespace,
                                producer_capability_id: capability_id.clone(),
                                content_type: "application/json".to_string(),
                                expected_bytes: Some(canonical_len),
                            },
                            &canonical_bytes,
                            &execution.receipt,
                        )
                        .await
                        .map_err(|_| {
                            dispatch_resource_error(
                                runtime,
                                ironclaw_resources::ResourceError::Storage {
                                    reason: "completed runtime artifact persistence failed"
                                        .to_string(),
                                },
                            )
                        })?,
                );
            }
            // Bounding the transport preview is only safe when the canonical
            // output was persisted as a durable artifact the model can read
            // back (the preview is then a reference into that artifact).
            // Without a persisted artifact, truncating `execution.output`
            // would silently destroy the result — so the bound applies only
            // when `completed_artifact` exists.
            if execution.completed_artifact.is_some()
                && canonical_bytes.len() > ARTIFACT_INLINE_PREVIEW_MAX_BYTES
            {
                let canonical_text = std::str::from_utf8(&canonical_bytes).map_err(|_| {
                    dispatch_resource_error(
                        runtime,
                        ironclaw_resources::ResourceError::Storage {
                            reason: "completed runtime preview could not be represented"
                                .to_string(),
                        },
                    )
                })?;
                let mut end = ARTIFACT_INLINE_PREVIEW_MAX_BYTES;
                while !canonical_text.is_char_boundary(end) {
                    end -= 1;
                }
                execution.output = Value::String(canonical_text[..end].to_string());
            }
            Ok(())
        }
        .await;
        if let Err(error) = finalize_result {
            self.emit_dispatch_failure(
                scope.clone(),
                capability_id.clone(),
                Some(provider.clone()),
                Some(runtime),
                parent_invocation_id,
                &error,
            )
            .await?;
            return Err(error);
        }

        let mut event = RuntimeEvent::dispatch_succeeded(
            scope,
            capability_id.clone(),
            provider.clone(),
            runtime,
            execution.output_bytes,
        );
        event.parent_invocation_id = parent_invocation_id;
        self.emit_event(event).await?;

        Ok(CapabilityDispatchResult {
            capability_id,
            provider,
            runtime,
            output: execution.output,
            display_preview: execution.display_preview,
            usage: execution.usage,
            receipt: execution.receipt,
            completed_artifact: execution.completed_artifact,
            canonical_output_digest,
            canonical_item_count,
        })
    }

    async fn emit_dispatch_failure(
        &self,
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        parent_invocation_id: Option<InvocationId>,
        error: &DispatchError,
    ) -> Result<(), DispatchError> {
        let mut event = RuntimeEvent::dispatch_failed(
            scope,
            capability_id,
            provider,
            runtime,
            error.event_kind(),
        );
        event.parent_invocation_id = parent_invocation_id;
        self.emit_event(event).await
    }

    async fn emit_event(&self, event: RuntimeEvent) -> Result<(), DispatchError> {
        tracing::debug!(
            event_kind = ?event.kind,
            capability_id = %event.capability_id,
            provider = event.provider.as_ref().map(|provider| provider.as_str()).unwrap_or(""),
            runtime = ?event.runtime,
            output_bytes = event.output_bytes,
            error_kind = event.error_kind.as_deref().unwrap_or(""),
            "runtime dispatcher observed event"
        );
        if let Some(sink) = self.event_sink.as_ref() {
            let _ = sink.as_ref().emit(event).await;
        }
        Ok(())
    }
}

struct DispatchReservationGuard<'a, G>
where
    G: ResourceGovernor,
{
    governor: &'a G,
    reservation: Option<ResourceReservation>,
}

impl<'a, G> DispatchReservationGuard<'a, G>
where
    G: ResourceGovernor,
{
    fn new(governor: &'a G, reservation: Option<ResourceReservation>) -> Self {
        Self {
            governor,
            reservation,
        }
    }

    fn take(&mut self) -> Option<ResourceReservation> {
        self.reservation.take()
    }

    fn validate(&self) -> Result<(), ironclaw_resources::ResourceError> {
        if let Some(reservation) = &self.reservation {
            self.governor.validate_reservation(reservation)?;
        }
        Ok(())
    }
}

impl<G> Drop for DispatchReservationGuard<'_, G>
where
    G: ResourceGovernor,
{
    fn drop(&mut self) {
        if let Some(reservation) = &self.reservation
            && let Err(error) = self.governor.release(reservation.id)
        {
            tracing::warn!(
                reservation_id = %reservation.id,
                error = %error,
                "failed to release prepared resource reservation after dispatcher validation failure"
            );
        }
    }
}

fn dispatch_resource_error(
    runtime: RuntimeKind,
    error: ironclaw_resources::ResourceError,
) -> DispatchError {
    tracing::debug!(%error, ?runtime, "reservation validation failed before dispatch");
    let cause = error.to_string();
    // System has no runtime backend to attribute a resource-reservation
    // failure to, so it classifies as MissingRuntimeBackend here rather than
    // as a generic provider rejection.
    if runtime == RuntimeKind::System {
        return DispatchError::MissingRuntimeBackend { runtime };
    }
    DispatchError::provider_rejected(
        Some(runtime),
        DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Resource),
        Some(cause),
        None,
    )
}

#[async_trait]
impl<G> CapabilityDispatcher for RuntimeDispatcher<'_, G>
where
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        authorized: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        RuntimeDispatcher::dispatch_json(self, authorized).await
    }
}
