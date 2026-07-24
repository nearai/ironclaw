//! `ExtensionHost` — the only active-set writer (overview.md §6).
//!
//! Every extension moves through the same pipeline and the same states; the
//! only extension-specific participation is manifest data and the two
//! idempotent adapter hooks. Installation state and the active snapshot are
//! written here and nowhere else; a single async mutex serializes lifecycle
//! operations (single serving process assumption).
//!
//! The host record carries only the working subset it can prove —
//! `InstallationState::{Installed, Active, Failed}` plus a redacted
//! `last_error`. Removal is the facade path (`remove_record` drops the row and
//! the facade runs auth/credential cleanup); the host does not own a
//! multi-step removal pipeline.

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
///
/// The lifecycle passes the staged record's declared `[[channel.egress]]`
/// targets so vendor wiring works **during** activation — before the
/// extension is published to any snapshot (a snapshot lookup here would
/// fail-closed exactly when `activate()` needs egress).
pub trait EgressFactory: Send + Sync {
    fn egress_for_channel(
        &self,
        extension_id: &str,
        installation_id: &str,
        declared: &[ironclaw_host_api::ChannelEgressDescriptor],
    ) -> Arc<dyn RestrictedEgress>;
}

/// Dependencies `ExtensionHost` is constructed with. Every port is generic;
/// composition supplies concrete implementations.
pub struct ExtensionHostDeps {
    pub store: Arc<dyn InstallationRecordStore>,
    pub loader: Arc<dyn ExtensionLoader>,
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
    /// Publish notifications for [`SnapshotWatch::subscribe`] — carries the
    /// published generation so subscribers can coalesce and re-read
    /// `current()`. `send_replace` never fails with zero receivers.
    snapshot_published: tokio::sync::watch::Sender<u64>,
    /// Receiver template cloned into every [`SnapshotWatch`]; also keeps the
    /// channel alive independent of external subscribers.
    snapshot_published_rx: tokio::sync::watch::Receiver<u64>,
}

struct LifecycleState {
    snapshot: Arc<ActiveSnapshot>,
    generation: u64,
}

/// A cloneable, synchronous view of the current active snapshot.
#[derive(Clone)]
pub struct SnapshotWatch {
    cell: Arc<RwLock<Arc<ActiveSnapshot>>>,
    published: tokio::sync::watch::Receiver<u64>,
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

    /// Subscribe to snapshot publishes: the receiver wakes on every new
    /// generation (values coalesce under contention — always re-read
    /// [`Self::current`] after a wake). The channel closes when the host is
    /// dropped.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.published.clone()
    }
}

/// Typed lifecycle failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    #[error("extension `{extension_id}` is not installed")]
    NotInstalled { extension_id: String },
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
        let (snapshot_published, snapshot_published_rx) = tokio::sync::watch::channel(0);
        Self {
            deps,
            lifecycle_lock: Mutex::new(LifecycleState {
                snapshot: ActiveSnapshot::empty(),
                generation: 0,
            }),
            snapshot_cell: Arc::new(RwLock::new(ActiveSnapshot::empty())),
            snapshot_published,
            snapshot_published_rx,
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
            published: self.snapshot_published_rx.clone(),
        }
    }

    fn mirror_snapshot(&self, snapshot: &Arc<ActiveSnapshot>) {
        {
            let mut cell = match self.snapshot_cell.write() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *cell = Arc::clone(snapshot);
        }
        // Notify AFTER the mirror write so a woken subscriber's `current()`
        // read always observes at least this generation.
        self.snapshot_published
            .send_replace(snapshot.generation().0);
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
    /// new generation. Initial failure publishes nothing and records the
    /// terminal `Failed` state with a redacted `last_error`; failed refresh
    /// retains the prior active record and generation.
    pub async fn activate(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let record = self.require_installed(extension_id).await?;
        self.publish_candidate_locked(&mut guard, record).await
    }

    /// Build and publish one candidate installation record without first
    /// withdrawing the currently active generation.
    ///
    /// This is the runtime-publication entrypoint for composition. A refresh
    /// candidate is loaded, bound, conflict-checked, and activated while the
    /// prior immutable snapshot and host record remain authoritative. Only a
    /// fully valid candidate is persisted as Active and swapped into the next
    /// generation. A failed first publication records Failed as the ordinary
    /// activation path does; a failed refresh retains the prior active record
    /// and generation intact.
    pub async fn publish_candidate(
        &self,
        record: InstallationRecord,
    ) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        self.publish_candidate_locked(&mut guard, record).await
    }

    async fn publish_candidate_locked(
        &self,
        guard: &mut LifecycleState,
        record: InstallationRecord,
    ) -> Result<(), LifecycleError> {
        let record = InstallationRecord {
            state: InstallationState::Installed,
            last_error: None,
            ..record
        };
        let previous_is_active = guard.snapshot.extension(&record.extension_id).is_some();

        let active = match self.build_active(&record).await {
            Ok(active) => active,
            Err(error) => {
                self.record_candidate_failure(&record, previous_is_active, &error)
                    .await?;
                return Err(error);
            }
        };
        if let Some(conflict) = guard.snapshot.would_conflict(&active) {
            let error = LifecycleError::Conflict(conflict);
            self.record_candidate_failure(&record, previous_is_active, &error)
                .await?;
            return Err(error);
        }

        if let Some(channel) = &active.channel {
            let egress = self.deps.egress.egress_for_channel(
                &record.extension_id,
                &record.installation_id,
                record
                    .resolved
                    .channel
                    .as_ref()
                    .map(|channel| channel.egress.as_slice())
                    .unwrap_or(&[]),
            );
            let ctx = ironclaw_product::ChannelContext {
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
                let error = LifecycleError::ActivationHook {
                    reason: redact(&error.to_string()),
                };
                self.record_candidate_failure(&record, previous_is_active, &error)
                    .await?;
                return Err(error);
            }
        }

        // Build the immutable replacement before mutating either publication
        // surface. Snapshot construction failure therefore leaves the prior
        // host record and generation untouched.
        let (generation, snapshot) =
            self.build_snapshot(guard, &record.extension_id, Some(Arc::new(active)))?;
        self.persist_state(&record, InstallationState::Active, None)
            .await?;
        self.publish_snapshot(guard, generation, snapshot);
        Ok(())
    }

    /// Deactivate an active extension: unpublish (drain happens as the old
    /// generation `Arc` drops) → persist Installed.
    pub async fn deactivate(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let mut guard = self.lifecycle_lock.lock().await;
        let record = self.require_installed(extension_id).await?;
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

    /// Drop an installation record. This is the live removal path: the
    /// lifecycle facade unpublishes via [`Self::deactivate`], runs auth /
    /// credential cleanup (`cleanup_for_lifecycle`), and drops the mirrored
    /// host record here.
    pub async fn remove_record(&self, extension_id: &str) -> Result<(), LifecycleError> {
        let _guard = self.lifecycle_lock.lock().await;
        self.deps.store.delete(extension_id).await?;
        Ok(())
    }

    /// The redacted `last_error` for every installation record that carries
    /// one, keyed by extension id. A record has a `last_error` exactly when its
    /// last activation attempt failed (state `Failed`); the product projection
    /// uses the presence of a reason to surface `InstallationState::Failed` and
    /// threads the reason itself onto the extensions wire's `activation_error`.
    pub async fn installation_errors(
        &self,
    ) -> Result<std::collections::HashMap<String, String>, LifecycleError> {
        Ok(self
            .deps
            .store
            .list()
            .await?
            .into_iter()
            .filter_map(|record| record.last_error.map(|error| (record.extension_id, error)))
            .collect())
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

    async fn record_candidate_failure(
        &self,
        record: &InstallationRecord,
        previous_is_active: bool,
        error: &LifecycleError,
    ) -> Result<(), LifecycleError> {
        if previous_is_active {
            return Ok(());
        }
        self.persist_state(
            record,
            InstallationState::Failed,
            Some(redact(&error.to_string())),
        )
        .await?;
        Ok(())
    }

    fn build_snapshot(
        &self,
        guard: &LifecycleState,
        extension_id: &str,
        active: Option<Arc<ActiveExtension>>,
    ) -> Result<(u64, Arc<ActiveSnapshot>), LifecycleError> {
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
        let generation = guard.generation + 1;
        let snapshot = ActiveSnapshot::build(Generation(generation), extensions)?;
        Ok((generation, snapshot))
    }

    fn publish_snapshot(
        &self,
        guard: &mut LifecycleState,
        generation: u64,
        snapshot: Arc<ActiveSnapshot>,
    ) {
        guard.generation = generation;
        guard.snapshot = snapshot;
        self.mirror_snapshot(&guard.snapshot);
    }

    /// Rebuild and publish the next generation with `extension_id` set to
    /// `active` (or removed when `None`). One immutable `Arc` swap.
    async fn publish_with(
        &self,
        guard: &mut LifecycleState,
        extension_id: &str,
        active: Option<Arc<ActiveExtension>>,
    ) -> Result<(), LifecycleError> {
        let (generation, snapshot) = self.build_snapshot(guard, extension_id, active)?;
        self.publish_snapshot(guard, generation, snapshot);
        Ok(())
    }
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
