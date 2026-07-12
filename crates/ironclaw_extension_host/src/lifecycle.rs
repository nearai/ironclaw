//! `ExtensionHost` — the only active-set writer (overview.md §6).
//!
//! Every extension moves through the same pipeline and the same states; the
//! only extension-specific participation is manifest data and the two
//! idempotent adapter hooks. Installation state and the active snapshot are
//! written here and nowhere else; a single async mutex serializes lifecycle
//! operations (single serving process assumption). The removal order is
//! fixed (§6.2) and identical for every extension.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RestrictedEgress};
use tokio::sync::Mutex;

use crate::active::{ActiveExtension, ActiveSnapshot, Generation, SnapshotConflict};
use crate::entrypoint::{BindError, check_binding};
use crate::loaders::{ExtensionLoader, LoadContext};
use crate::state::InstallationState;
use crate::store::{InstallationRecord, InstallationRecordStore, StoreError};

/// Host-side hooks for the removal steps `ExtensionHost` sequences but does
/// not itself own (auth revoke/grant deletion, integration-state deletion,
/// draining). Injected by composition; the host owns only the order.
#[async_trait]
pub trait RemovalHooks: Send + Sync {
    /// Best-effort remote revoke plus local grant deletion for the
    /// extension's vendors, shared-vendor aware: a vendor still used by
    /// another active extension keeps its grants. Failure lands the removal
    /// in `RemovalPending`.
    async fn revoke_and_delete_grants(&self, ctx: &RemovalContext<'_>) -> Result<(), HookError>;

    /// Delete config/secrets, identity bindings, and route registrations for
    /// this extension. Conversation and LLM history are never touched.
    /// Failure lands the removal in `RemovalPending`.
    async fn delete_integration_state(&self, ctx: &RemovalContext<'_>) -> Result<(), HookError>;
}

/// Context for the host-owned removal hooks.
pub struct RemovalContext<'a> {
    pub extension_id: &'a str,
    pub installation_id: &'a str,
    /// Extension ids that remain active after this removal — hooks use this
    /// for shared-vendor awareness.
    pub other_active_extension_ids: &'a [String],
}

/// Drains in-flight work for an extension before its snapshot generation is
/// dropped. Injected by composition.
#[async_trait]
pub trait DrainController: Send + Sync {
    async fn drain(&self, extension_id: &str, deadline: Duration) -> Result<(), HookError>;
}

/// Typed removal/drain hook failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HookError {
    #[error("removal hook failed: {reason}")]
    Failed { reason: String },
}

/// Host-owned egress used by `channel.activate()`/`cleanup()`. Injected so
/// the crate does not link the concrete egress implementation.
pub trait EgressFactory: Send + Sync {
    fn egress_for(&self, extension_id: &str) -> Arc<dyn RestrictedEgress>;
}

/// Dependencies `ExtensionHost` is constructed with. Every port is generic;
/// composition supplies concrete implementations.
pub struct ExtensionHostDeps {
    pub store: Arc<dyn InstallationRecordStore>,
    pub loader: Arc<dyn ExtensionLoader>,
    pub removal_hooks: Arc<dyn RemovalHooks>,
    pub drain: Arc<dyn DrainController>,
    pub egress: Arc<dyn EgressFactory>,
    /// Host-owned capability ids (the built-in registry). An extension
    /// declaring any of these fails activation with a conflict (TOOL-10).
    pub reserved_capability_ids: BTreeSet<CapabilityId>,
    /// Fixed host route paths (full canonical paths). An extension whose
    /// canonical ingress path (`/webhooks/extensions/{id}/{suffix}`)
    /// collides with one fails activation with a conflict (ING-1).
    pub reserved_ingress_routes: BTreeSet<String>,
    /// Bounded deadline for adapter hooks and drains.
    pub hook_deadline: Duration,
}

/// The generic extension lifecycle host.
pub struct ExtensionHost {
    deps: ExtensionHostDeps,
    /// Serializes every lifecycle operation (single serving process).
    lifecycle_lock: Mutex<LifecycleState>,
    /// Lock-free mirror of the current snapshot for synchronous readers
    /// (the dispatch-time tool resolver). Written only under
    /// `lifecycle_lock`, so readers observe exactly the published
    /// generations in order.
    snapshot_cell: Arc<RwLock<Arc<ActiveSnapshot>>>,
}

struct LifecycleState {
    snapshot: Arc<ActiveSnapshot>,
    generation: u64,
}

/// A cloneable, synchronous view of the current active snapshot.
#[derive(Clone)]
pub struct SnapshotWatch {
    cell: Arc<RwLock<Arc<ActiveSnapshot>>>,
}

impl SnapshotWatch {
    /// The currently published generation. In-flight readers keep the `Arc`
    /// they resolved.
    pub fn current(&self) -> Arc<ActiveSnapshot> {
        match self.cell.read() {
            Ok(guard) => Arc::clone(&guard),
            // A poisoned mirror still holds the last published snapshot;
            // resolution staying available beats propagating the panic.
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }
}

/// Typed lifecycle failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    #[error("extension `{extension_id}` is not installed")]
    NotInstalled { extension_id: String },
    #[error("extension `{extension_id}` cannot transition {from} → {to}")]
    IllegalTransition {
        extension_id: String,
        from: &'static str,
        to: &'static str,
    },
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error(transparent)]
    Conflict(#[from] SnapshotConflict),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("activation hook failed: {reason}")]
    ActivationHook { reason: String },
}

impl ExtensionHost {
    pub async fn new(deps: ExtensionHostDeps) -> Self {
        Self {
            deps,
            lifecycle_lock: Mutex::new(LifecycleState {
                snapshot: ActiveSnapshot::empty(),
                generation: 0,
            }),
            snapshot_cell: Arc::new(RwLock::new(ActiveSnapshot::empty())),
        }
    }

    /// The current active snapshot (generation-pinned; in-flight readers keep
    /// their own `Arc`).
    pub async fn snapshot(&self) -> Arc<ActiveSnapshot> {
        Arc::clone(&self.lifecycle_lock.lock().await.snapshot)
    }

    /// A synchronous watch over the published snapshot, for dispatch-time
    /// resolvers.
    pub fn snapshot_watch(&self) -> SnapshotWatch {
        SnapshotWatch {
            cell: Arc::clone(&self.snapshot_cell),
        }
    }

    fn mirror_snapshot(&self, snapshot: &Arc<ActiveSnapshot>) {
        let mut cell = match self.snapshot_cell.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *cell = Arc::clone(snapshot);
    }

    /// Install a resolved extension in `Installed` state (idempotent upsert).
    pub async fn install(&self, record: InstallationRecord) -> Result<(), LifecycleError> {
        let _guard = self.lifecycle_lock.lock().await;
        let record = InstallationRecord {
            state: InstallationState::Installed,
            last_error: None,
            ..record
        };
        self.deps.store.upsert(record).await?;
        Ok(())
    }

    /// Activate an installed extension: load → bind → binding check → global
    /// conflict check → `channel.activate()` → persist Active → publish one
    /// new generation. Failure publishes nothing and records a typed error.
    pub async fn activate(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let record = self.require_installed(extension_id).await?;

        // Persist the transient Activating state before any work.
        self.persist_state(&record, InstallationState::Activating, None)
            .await?;

        match self.build_active(&record).await {
            Ok(active) => {
                // Global conflict check against the current active set.
                if let Some(conflict) = guard.snapshot.would_conflict(&active) {
                    self.persist_state(
                        &record,
                        InstallationState::Installed,
                        Some(redact(&conflict.to_string())),
                    )
                    .await?;
                    return Err(LifecycleError::Conflict(conflict));
                }

                // Vendor wiring: channel.activate(). Failure aborts with
                // nothing published.
                if let Some(channel) = &active.channel {
                    let egress = self.deps.egress.egress_for(extension_id);
                    let ctx = ironclaw_product_adapters::ChannelContext {
                        extension_id: &record.extension_id,
                        installation_id: &record.installation_id,
                        config: &record.config,
                    };
                    if let Err(error) = with_deadline(
                        self.deps.hook_deadline,
                        channel.activate(&ctx, egress.as_ref()),
                    )
                    .await
                    {
                        self.persist_state(
                            &record,
                            InstallationState::Installed,
                            Some(redact(&error.to_string())),
                        )
                        .await?;
                        return Err(LifecycleError::ActivationHook {
                            reason: redact(&error.to_string()),
                        });
                    }
                }

                // Persist Active, then publish exactly one new generation.
                self.persist_state(&record, InstallationState::Active, None)
                    .await?;
                self.publish_with(&mut guard, extension_id, Some(Arc::new(active)))
                    .await?;
                Ok(())
            }
            Err(error) => {
                self.persist_state(
                    &record,
                    InstallationState::Installed,
                    Some(redact(&error.to_string())),
                )
                .await?;
                Err(error)
            }
        }
    }

    /// Deactivate an active extension: unpublish (drain happens as the old
    /// generation `Arc` drops) → persist Installed.
    pub async fn deactivate(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let record = self.require_installed(extension_id).await?;
        self.persist_state(&record, InstallationState::Deactivating, None)
            .await?;
        self.publish_with(&mut guard, extension_id, None).await?;
        let _ = self
            .deps
            .drain
            .drain(extension_id, self.deps.hook_deadline)
            .await;
        self.persist_state(&record, InstallationState::Installed, None)
            .await?;
        Ok(())
    }

    /// Remove an extension following the fixed order (§6.2):
    /// unpublish → drain → channel.cleanup() → auth revoke + grant delete →
    /// config/secret/identity delete → Removed. A cleanup failure lands in
    /// `RemovalPending` (retryable) and never reports success early.
    pub async fn remove(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let record = self.require_installed(extension_id).await?;

        // 1. Persist Removing; unpublish (new work rejected).
        self.persist_state(&record, InstallationState::Removing, None)
            .await?;
        let active = guard.snapshot.extension(extension_id);
        self.publish_with(&mut guard, extension_id, None).await?;

        // 2. Drain in-flight work (bounded).
        let _ = self
            .deps
            .drain
            .drain(extension_id, self.deps.hook_deadline)
            .await;

        let other_active = guard.snapshot.extension_ids();
        let ctx = RemovalContext {
            extension_id: &record.extension_id,
            installation_id: &record.installation_id,
            other_active_extension_ids: &other_active,
        };

        // 3. channel.cleanup() — idempotent, best-effort.
        if let Some(active) = &active
            && let Some(channel) = &active.channel
        {
            let egress = self.deps.egress.egress_for(extension_id);
            if let Err(error) = with_deadline(
                self.deps.hook_deadline,
                channel.cleanup(
                    &ironclaw_product_adapters::ChannelContext {
                        extension_id: &record.extension_id,
                        installation_id: &record.installation_id,
                        config: &record.config,
                    },
                    egress.as_ref(),
                ),
            )
            .await
            {
                return self
                    .to_removal_pending(&record, &redact(&error.to_string()))
                    .await;
            }
        }

        // 4. Auth revoke + grant deletion (shared-vendor aware).
        if let Err(error) = self.deps.removal_hooks.revoke_and_delete_grants(&ctx).await {
            return self
                .to_removal_pending(&record, &redact(&error.to_string()))
                .await;
        }

        // 5. Config/secret/identity/route deletion.
        if let Err(error) = self.deps.removal_hooks.delete_integration_state(&ctx).await {
            return self
                .to_removal_pending(&record, &redact(&error.to_string()))
                .await;
        }

        // 6. Persist Removed and delete the record. History is never touched.
        self.deps.store.delete(extension_id).await?;
        Ok(())
    }

    /// Retry a `RemovalPending` removal from step 3.
    pub async fn retry_removal(&self, extension_id: &str) -> Result<(), LifecycleError> {
        // The extension is already unpublished; re-run the cleanup tail by
        // re-entering `remove` (idempotent hooks).
        self.remove(extension_id).await
    }

    /// Drop an installation record without running the removal pipeline.
    ///
    /// Transitional (facade era): removal side effects are still owned by
    /// the lifecycle facade, which unpublishes via [`Self::deactivate`] and
    /// then drops the mirrored record here. Deleted when the facade
    /// collapses and [`Self::remove`] becomes the production removal path.
    pub async fn remove_record(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let _guard = self.lifecycle_lock.lock().await;
        self.deps.store.delete(extension_id).await?;
        Ok(())
    }

    /// Restore all enabled generations at startup and publish once. An
    /// invalid extension is skipped with a typed error and does not block the
    /// valid rest.
    pub async fn restore_at_startup(&self) -> Result<RestoreReport, LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let records = self.deps.store.list().await?;
        let mut restored = Vec::new();
        let mut skipped = Vec::new();

        for record in records {
            // Resolve the transient state deterministically.
            let target = record.state.resume_target();
            if target != InstallationState::Active {
                // Non-active (or activation-interrupted → Installed) records
                // are left as they resolve; only Active extensions publish.
                if record.state != target {
                    self.persist_state(&record, target, record.last_error.clone())
                        .await?;
                }
                continue;
            }
            match self.build_active(&record).await {
                Ok(active) => restored.push((record.extension_id.clone(), Arc::new(active))),
                Err(error) => {
                    self.persist_state(
                        &record,
                        InstallationState::Installed,
                        Some(redact(&error.to_string())),
                    )
                    .await?;
                    skipped.push((record.extension_id.clone(), redact(&error.to_string())));
                }
            }
        }

        guard.generation += 1;
        let snapshot = ActiveSnapshot::build(
            Generation(guard.generation),
            restored
                .iter()
                .map(|(_, active)| Arc::clone(active))
                .collect(),
        )?;
        guard.snapshot = snapshot;
        self.mirror_snapshot(&guard.snapshot);

        Ok(RestoreReport {
            restored: restored.into_iter().map(|(id, _)| id).collect(),
            skipped,
        })
    }

    async fn require_installed(
        &self,
        extension_id: &str,
    ) -> Result<InstallationRecord, LifecycleError> {
        self.deps
            .store
            .get(extension_id)
            .await?
            .ok_or_else(|| LifecycleError::NotInstalled {
                extension_id: extension_id.to_string(),
            })
    }

    async fn build_active(
        &self,
        record: &InstallationRecord,
    ) -> Result<ActiveExtension, LifecycleError> {
        let loaded = self
            .deps
            .loader
            .load(&LoadContext {
                extension_id: record.extension_id.clone(),
                installation_id: record.installation_id.clone(),
                resolved: Arc::clone(&record.resolved),
            })
            .await?;
        // A discovery-owning loader publishes its effective contract; static
        // loads bind against the persisted declaration.
        let resolved = loaded
            .effective_resolved
            .unwrap_or_else(|| Arc::clone(&record.resolved));
        let bindings = loaded.entrypoint.bind(crate::entrypoint::BindContext {
            installation_id: record.installation_id.clone(),
            resolved: Arc::clone(&resolved),
            config: record.config.clone(),
        })?;
        check_binding(&resolved, &bindings)?;
        for tool in &resolved.tools {
            if self.deps.reserved_capability_ids.contains(&tool.id) {
                return Err(LifecycleError::Conflict(
                    SnapshotConflict::ReservedCapability {
                        capability_id: tool.id.as_str().to_string(),
                        extension_id: record.extension_id.clone(),
                    },
                ));
            }
        }
        if let Some(channel) = &resolved.channel
            && let Some(ingress) = &channel.ingress
        {
            let route = crate::ingress::canonical_ingress_path(
                &record.extension_id,
                ingress.route_suffix.as_str(),
            );
            if self.deps.reserved_ingress_routes.contains(&route) {
                return Err(LifecycleError::Conflict(SnapshotConflict::ReservedRoute {
                    route,
                    extension_id: record.extension_id.clone(),
                }));
            }
        }
        Ok(ActiveExtension {
            extension_id: record.extension_id.clone(),
            installation_id: record.installation_id.clone(),
            resolved,
            tools: bindings.tools,
            channel: bindings.channel,
        })
    }

    async fn persist_state(
        &self,
        record: &InstallationRecord,
        state: InstallationState,
        last_error: Option<String>,
    ) -> Result<(), StoreError> {
        self.deps
            .store
            .upsert(InstallationRecord {
                extension_id: record.extension_id.clone(),
                installation_id: record.installation_id.clone(),
                state,
                resolved: Arc::clone(&record.resolved),
                config: record.config.clone(),
                last_error,
            })
            .await
    }

    async fn to_removal_pending(
        &self,
        record: &InstallationRecord,
        reason: &str,
    ) -> Result<(), LifecycleError> {
        self.persist_state(
            record,
            InstallationState::RemovalPending,
            Some(reason.to_string()),
        )
        .await?;
        Err(LifecycleError::ActivationHook {
            reason: reason.to_string(),
        })
    }

    /// Rebuild and publish the next generation with `extension_id` set to
    /// `active` (or removed when `None`). One immutable `Arc` swap.
    async fn publish_with(
        &self,
        guard: &mut LifecycleState,
        extension_id: &str,
        active: Option<Arc<ActiveExtension>>,
    ) -> Result<(), LifecycleError> {
        let mut extensions: Vec<Arc<ActiveExtension>> = guard
            .snapshot
            .extension_ids()
            .into_iter()
            .filter(|id| id != extension_id)
            .filter_map(|id| guard.snapshot.extension(&id))
            .collect();
        if let Some(active) = active {
            extensions.push(active);
        }
        guard.generation += 1;
        guard.snapshot = ActiveSnapshot::build(Generation(guard.generation), extensions)?;
        self.mirror_snapshot(&guard.snapshot);
        Ok(())
    }
}

/// Result of a startup restore.
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// Redact a hook/error string to a bounded, delimiter-free summary so no raw
/// payload or path is persisted on a record.
fn redact(reason: &str) -> String {
    let cleaned: String = reason
        .chars()
        .filter(|c| !matches!(c, '{' | '}' | '[' | ']' | '<' | '>' | '`' | '/' | '\\'))
        .take(200)
        .collect();
    cleaned.trim().to_string()
}

async fn with_deadline<F, T, E>(deadline: Duration, future: F) -> Result<T, DeadlineOr<E>>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    match tokio::time::timeout(deadline, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(DeadlineOr::Inner(error)),
        Err(_) => Err(DeadlineOr::Deadline),
    }
}

enum DeadlineOr<E> {
    Deadline,
    Inner(E),
}

impl<E: std::fmt::Display> std::fmt::Display for DeadlineOr<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deadline => f.write_str("hook exceeded its bounded deadline"),
            Self::Inner(error) => write!(f, "{error}"),
        }
    }
}
