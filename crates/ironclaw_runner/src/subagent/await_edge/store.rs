//! CAS primitives over one [`AwaitEdge`] file — §2, §4.0, §5.2, §5.5. Built
//! on a single shared `ScopedFilesystem` handle via `wrap_scoped` (never
//! `with_fixed_view`, §4.5a) — every method takes the caller's live
//! `TurnScope`/axes as an explicit argument; the resolver recomputes the
//! `MountView` for that scope on that call, exactly like
//! `ironclaw_conversations::filesystem_store` and
//! `ironclaw_reborn_composition::llm_admin::llm_key_store` already do.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::ResourceScope;
use ironclaw_turns::{TurnRunId, TurnScope};

use super::{
    AwaitEdge, AwaitEdgeState, AwaitEdgeStoreError, EdgeTerminalKind, ReservationReleaseState,
    edge_dir_for_parent, edge_path,
};
use crate::subagent::await_edge::roster::{self, RosterKey};

/// Test-only fault-injection hooks for [`FilesystemAwaitEdgeStore::close_with_release`]'s
/// two named crash windows (§5.5 round-7 scenarios (a) and (b)). Bundled into
/// one struct instead of two more positional `Option<&dyn Fn()>` parameters —
/// keeps that method under `clippy::too_many_arguments` without an exemption.
/// `Default::default()` (both `None`) in production.
#[derive(Default, Clone, Copy)]
pub struct CloseCrashHooks<'a> {
    pub before_prune: Option<&'a (dyn Fn() + Send + Sync)>,
    pub before_delete: Option<&'a (dyn Fn() + Send + Sync)>,
}

/// Thin CAS wrapper around one shared `Arc<ScopedFilesystem<F>>`. Generic
/// over the backend, matching every other filesystem-backed reborn store
/// (`goal_store.rs`'s `FilesystemSubagentGoalStore<F>`,
/// `local_trigger_access::filesystem::RebornFilesystemLocalTriggerAccessStore<F>`)
/// — never `Arc<dyn RootFilesystem>`.
pub struct FilesystemAwaitEdgeStore<F: RootFilesystem + ?Sized> {
    fs: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemAwaitEdgeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(fs: Arc<ScopedFilesystem<F>>) -> Self {
        Self { fs }
    }

    fn resource_scope(&self, scope: &TurnScope) -> ResourceScope {
        scope.to_resource_scope()
    }

    async fn get_edge(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<Option<(AwaitEdge, RecordVersion)>, AwaitEdgeStoreError> {
        let path = edge_path(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
            parent_run_id,
            child_run_id,
        )?;
        let resource_scope = self.resource_scope(scope);
        match self.fs.get(&resource_scope, &path).await {
            Ok(Some(versioned)) => {
                let edge: AwaitEdge =
                    serde_json::from_slice(&versioned.entry.body).map_err(|error| {
                        AwaitEdgeStoreError::Backend {
                            reason: format!("await-edge deserialize failed: {error}"),
                        }
                    })?;
                Ok(Some((edge, versioned.version)))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(backend_error(error)),
        }
    }

    async fn put_edge_cas(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        edge: &AwaitEdge,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AwaitEdgeStoreError> {
        let path = edge_path(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
            parent_run_id,
            child_run_id,
        )?;
        let resource_scope = self.resource_scope(scope);
        let body = serde_json::to_vec(edge).map_err(|error| AwaitEdgeStoreError::Backend {
            reason: format!("await-edge serialize failed: {error}"),
        })?;
        self.fs
            .put(
                &resource_scope,
                &path,
                Entry {
                    body,
                    content_type: ContentType::json(),
                    kind: None,
                    indexed: Default::default(),
                },
                cas,
            )
            .await
            .map_err(|error| match error {
                FilesystemError::VersionMismatch { .. } => AwaitEdgeStoreError::VersionMismatch {
                    parent_run_id,
                    child_run_id,
                },
                other => backend_error(other),
            })
    }

    /// §4.5 write-before-first-edge: touch the roster marker, then open the
    /// edge at `Open`/`Unclaimed`. Idempotent — an existing edge for this
    /// exact `(parent_run_id, child_run_id)` pair is left untouched (the
    /// pair never recurs, §4.0's ABA-immunity argument, so a `Conflict` here
    /// can only mean "already opened by an earlier attempt of this same
    /// call", never a real collision).
    pub async fn open(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        edge: AwaitEdge,
    ) -> Result<(), AwaitEdgeStoreError> {
        let roster_key = RosterKey::from_resource_scope(&self.resource_scope(scope));
        roster::touch_roster_marker(&self.fs, &roster_key).await?;
        match self
            .put_edge_cas(
                scope,
                parent_run_id,
                child_run_id,
                &edge,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => {}
            Err(AwaitEdgeStoreError::VersionMismatch { .. }) => {}
            Err(other) => return Err(other),
        }
        // §4.0 round-5 self-heal: an unconditional, version-bumping re-put
        // after the edge write succeeds, not just before it. Closes the
        // boot-recovery race where boot reads the marker's version while the
        // edge dir is momentarily empty, then this edge lands, then boot's
        // stale-version delete would otherwise succeed and hide this scope
        // from recovery forever (design doc §4.0 "Round-5 fix").
        roster::touch_roster_marker(&self.fs, &roster_key).await?;
        Ok(())
    }

    /// Rollback-only abandon (§2 mode-scoped case (b)): CAS current state to
    /// `Abandoned` (best-effort — `NotFound` is benign, someone already
    /// closed it), then run the normal close sequence.
    pub async fn abandon(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        let Some((mut edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await?
        else {
            return Ok(());
        };
        if edge.state != AwaitEdgeState::Open {
            // Already settled/drained/abandoned by someone else — leave it
            // to whichever path is driving that state, not ours to override.
            return Ok(());
        }
        edge.state = AwaitEdgeState::Abandoned;
        match self
            .put_edge_cas(
                scope,
                parent_run_id,
                child_run_id,
                &edge,
                CasExpectation::Version(version),
            )
            .await
        {
            Ok(_) => {}
            Err(AwaitEdgeStoreError::VersionMismatch { .. }) => return Ok(()),
            Err(other) => return Err(other),
        }
        self.close(scope, parent_run_id, child_run_id).await
    }

    /// CAS `Open -> Settled` in one write (terminal_kind + terminal_byte_len
    /// set together, §5.4). Already-`Settled`/`Drained`/`Abandoned` is
    /// benign (concurrent redelivery, or recovery re-observing) — returns
    /// the current edge either way.
    pub async fn settle(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        terminal_kind: EdgeTerminalKind,
        terminal_byte_len: Option<u64>,
        terminal_reason: Option<String>,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        let Some((mut edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await?
        else {
            return Ok(None);
        };
        if edge.state != AwaitEdgeState::Open {
            return Ok(Some(edge));
        }
        edge.state = AwaitEdgeState::Settled;
        edge.terminal_kind = Some(terminal_kind);
        edge.terminal_byte_len = terminal_byte_len;
        edge.terminal_reason = terminal_reason;
        edge.settled_at = Some(Utc::now());
        match self
            .put_edge_cas(
                scope,
                parent_run_id,
                child_run_id,
                &edge,
                CasExpectation::Version(version),
            )
            .await
        {
            Ok(_) => Ok(Some(edge)),
            // Someone else settled it concurrently — re-read and return
            // whatever is current rather than erroring.
            Err(AwaitEdgeStoreError::VersionMismatch { .. }) => Ok(self
                .get_edge(scope, parent_run_id, child_run_id)
                .await?
                .map(|(edge, _)| edge)),
            Err(other) => Err(other),
        }
    }

    /// D3's group listing: every sibling edge under the same `parent_run_id`
    /// sharing `gate_ref` (the pre-existing shared-batch-gate mechanism this
    /// design doc doesn't name). Cheap list+filter at the ≤4-spawns/turn,
    /// ≤16-descendants group sizes this ever sees.
    pub async fn list_group(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        gate_ref: &ironclaw_turns::GateRef,
    ) -> Result<Vec<(TurnRunId, AwaitEdge)>, AwaitEdgeStoreError> {
        let dir = edge_dir_for_parent(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
            parent_run_id,
        )?;
        let resource_scope = self.resource_scope(scope);
        let entries = match self.fs.list_dir(&resource_scope, &dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(backend_error(error)),
        };
        let mut group = Vec::new();
        for entry in entries {
            let Some(uuid_str) = entry.name.strip_suffix(".json") else {
                continue;
            };
            let Ok(child_run_id) = TurnRunId::parse(uuid_str) else {
                continue;
            };
            if let Some((edge, _)) = self.get_edge(scope, parent_run_id, child_run_id).await?
                && edge.gate_ref == *gate_ref
            {
                group.push((child_run_id, edge));
            }
        }
        Ok(group)
    }

    /// §5.5 tri-state: `Unclaimed -> Claimed`, single CAS winner.
    pub async fn claim_release(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<Option<RecordVersion>, AwaitEdgeStoreError> {
        let Some((mut edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await?
        else {
            return Ok(None);
        };
        if edge.reservation_release != ReservationReleaseState::Unclaimed {
            return Ok(None);
        }
        edge.reservation_release = ReservationReleaseState::Claimed;
        match self
            .put_edge_cas(
                scope,
                parent_run_id,
                child_run_id,
                &edge,
                CasExpectation::Version(version),
            )
            .await
        {
            Ok(new_version) => Ok(Some(new_version)),
            Err(AwaitEdgeStoreError::VersionMismatch { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// §5.5: `Claimed -> Released`, after the capacity-release call
    /// succeeded. Returns the CAS's own returned version — the caller must
    /// carry this forward as the `delete_if_version` token (§2 round-4 fix),
    /// never an earlier transition's version.
    pub async fn mark_released(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        claimed_version: RecordVersion,
    ) -> Result<RecordVersion, AwaitEdgeStoreError> {
        let Some((mut edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await?
        else {
            return Err(AwaitEdgeStoreError::NotFound {
                parent_run_id,
                child_run_id,
            });
        };
        if version != claimed_version {
            return Err(AwaitEdgeStoreError::VersionMismatch {
                parent_run_id,
                child_run_id,
            });
        }
        edge.reservation_release = ReservationReleaseState::Released;
        self.put_edge_cas(
            scope,
            parent_run_id,
            child_run_id,
            &edge,
            CasExpectation::Version(version),
        )
        .await
    }

    /// §5.5 failure-path retry-unlock: `Claimed -> Unclaimed` so a transient
    /// release failure doesn't permanently strand the reservation.
    pub async fn unclaim_release(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        let Some((mut edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await?
        else {
            return Ok(());
        };
        if edge.reservation_release != ReservationReleaseState::Claimed {
            return Ok(());
        }
        edge.reservation_release = ReservationReleaseState::Unclaimed;
        match self
            .put_edge_cas(
                scope,
                parent_run_id,
                child_run_id,
                &edge,
                CasExpectation::Version(version),
            )
            .await
        {
            Ok(_) | Err(AwaitEdgeStoreError::VersionMismatch { .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Read-only peek, used by recovery to decide which sub-step of the
    /// close sequence still needs driving.
    pub async fn peek(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        Ok(self
            .get_edge(scope, parent_run_id, child_run_id)
            .await?
            .map(|(edge, _)| edge))
    }

    /// `delete_if_version` with the caller-supplied token — used by the
    /// close sequence once `reservation_release == Released`. `NotFound`/
    /// `VersionMismatch` here are both benign (§2: someone else finished the
    /// close, or it's mid-handling) — reported as already-closed by the
    /// caller, not an error.
    pub async fn delete_if_version(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        version: RecordVersion,
        crash_hook: Option<&(dyn Fn() + Send + Sync)>,
    ) -> Result<(), AwaitEdgeStoreError> {
        let path = edge_path(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
            parent_run_id,
            child_run_id,
        )?;
        let resource_scope = self.resource_scope(scope);
        // Test-only fault injection point (§5.5 round-7's mid-prune crash
        // seed): a hook that panics/returns before this call ever executes
        // simulates "crash between prune and delete" without a bespoke
        // seam. `None` in production.
        if let Some(hook) = crash_hook {
            hook();
        }
        match self
            .fs
            .delete_if_version(&resource_scope, &path, version)
            .await
        {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(other) => Err(backend_error(other)),
        }
    }

    /// The close-path's opportunistic roster prune (§4.5 round-7): after
    /// deleting an edge, re-list the parent's edge dir; if empty, prune the
    /// scope's roster marker via the same CAS'd sequence boot uses.
    pub async fn prune_roster_if_parent_empty(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        let dir = edge_dir_for_parent(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
            parent_run_id,
        )?;
        let resource_scope = self.resource_scope(scope);
        let is_empty = match self.fs.list_dir_bounded(&resource_scope, &dir, 1).await {
            Ok(entries) => entries.is_empty(),
            Err(FilesystemError::NotFound { .. }) => true,
            Err(error) => return Err(backend_error(error)),
        };
        if is_empty {
            let roster_key = RosterKey::from_resource_scope(&resource_scope);
            roster::prune_roster_marker(&self.fs, &roster_key).await?;
        }
        Ok(())
    }

    /// §4.3: bounded, scope-isolated listing of every unclosed edge (`Open`,
    /// `Settled`, or terminal-but-undeleted `Drained`/`Abandoned`, §2's crash
    /// window) under this scope's axis-qualified prefix.
    pub async fn list_unclosed_for_scope(
        &self,
        scope: &TurnScope,
    ) -> Result<Vec<(TurnRunId, TurnRunId, AwaitEdge)>, AwaitEdgeStoreError> {
        let root = super::edge_scope_root(
            scope.agent_id.as_ref().map(|id| id.as_str()),
            scope.project_id.as_ref().map(|id| id.as_str()),
        )?;
        let resource_scope = self.resource_scope(scope);
        let parent_dirs = match self.fs.list_dir(&resource_scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(backend_error(error)),
        };
        let mut unclosed = Vec::new();
        for parent_dir in parent_dirs {
            let Ok(parent_run_id) = TurnRunId::parse(&parent_dir.name) else {
                continue;
            };
            let dir = edge_dir_for_parent(
                scope.agent_id.as_ref().map(|id| id.as_str()),
                scope.project_id.as_ref().map(|id| id.as_str()),
                parent_run_id,
            )?;
            let children = match self.fs.list_dir(&resource_scope, &dir).await {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(backend_error(error)),
            };
            for child_entry in children {
                let Some(uuid_str) = child_entry.name.strip_suffix(".json") else {
                    continue;
                };
                let Ok(child_run_id) = TurnRunId::parse(uuid_str) else {
                    continue;
                };
                if let Some(edge) = self.peek(scope, parent_run_id, child_run_id).await? {
                    unclosed.push((parent_run_id, child_run_id, edge));
                }
            }
        }
        Ok(unclosed)
    }

    /// §2/§5.5's full close sequence: drive `reservation_release` to
    /// `Released` (retrying from whatever sub-state is found), prune the
    /// `released_children` entry, then `delete_if_version` using the
    /// `Released` CAS's own returned version. `release_fn` performs the
    /// actual `TurnStateStore::release_tree_descendants` call — injected so
    /// `store.rs` stays independent of `ironclaw_turns::TurnStateStore`'s
    /// concrete wiring (the resolver supplies it). `crash_hooks` bundles the
    /// two test-only fault-injection points for the crash windows named in
    /// §5.5 round-7: (a) between the `Released` CAS and the prune, (b)
    /// between the prune and `delete_if_version`. `CloseCrashHooks::default()`
    /// (both `None`) in production — bundled into one struct rather than two
    /// more positional `Option<&dyn Fn()>` parameters so this method stays
    /// under `clippy::too_many_arguments` without an exemption.
    pub async fn close_with_release<Fut, PruneFut>(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        release_fn: impl FnOnce() -> Fut,
        prune_fn: impl FnOnce() -> PruneFut,
        crash_hooks: CloseCrashHooks<'_>,
    ) -> Result<(), AwaitEdgeStoreError>
    where
        Fut: std::future::Future<Output = Result<(), AwaitEdgeStoreError>>,
        PruneFut: std::future::Future<Output = Result<(), AwaitEdgeStoreError>>,
    {
        let crash_hook_before_prune = crash_hooks.before_prune;
        let crash_hook_before_delete = crash_hooks.before_delete;
        // Single fetch, reused for every branch below -- the previous
        // `peek` (which discards the version) plus a second `get_edge` per
        // branch read+deserialized the same edge redundantly.
        let Some((edge, version)) = self.get_edge(scope, parent_run_id, child_run_id).await? else {
            return Ok(());
        };
        let released_version = match edge.reservation_release {
            // Already released by an earlier pass — recovery must not
            // re-invoke the release call (that would double-decrement); the
            // version already in hand is the delete token.
            ReservationReleaseState::Released => version,
            ReservationReleaseState::Unclaimed | ReservationReleaseState::Claimed => {
                let claimed_version =
                    if edge.reservation_release == ReservationReleaseState::Claimed {
                        version
                    } else {
                        match self
                            .claim_release(scope, parent_run_id, child_run_id)
                            .await?
                        {
                            Some(version) => version,
                            None => return Ok(()), // lost the claim race; the winner drives this
                        }
                    };
                match release_fn().await {
                    Ok(()) => {
                        match self
                            .mark_released(scope, parent_run_id, child_run_id, claimed_version)
                            .await
                        {
                            Ok(version) => version,
                            // §5.4's unified VersionMismatch disposition: a
                            // concurrent resolver instance (recovery sweep
                            // vs. reactive settle, or two recovery passes)
                            // can win the `Claimed -> Released` CAS first —
                            // that racer already drove `release_fn` (dedup'd
                            // by the idempotency key) and moved the edge to
                            // `Released`. Re-read the edge and adopt its
                            // current version as the delete token rather
                            // than failing the whole close; only a genuinely
                            // unexpected state (edge gone, or still short of
                            // `Released`) propagates the original error.
                            Err(super::AwaitEdgeStoreError::VersionMismatch { .. }) => {
                                let Some((current, version)) =
                                    self.get_edge(scope, parent_run_id, child_run_id).await?
                                else {
                                    return Ok(());
                                };
                                if current.reservation_release != ReservationReleaseState::Released
                                {
                                    return Err(super::AwaitEdgeStoreError::VersionMismatch {
                                        parent_run_id,
                                        child_run_id,
                                    });
                                }
                                version
                            }
                            Err(other) => return Err(other),
                        }
                    }
                    Err(error) => {
                        self.unclaim_release(scope, parent_run_id, child_run_id)
                            .await?;
                        return Err(error);
                    }
                }
            }
        };
        // Test-only fault injection point, scenario (a): a crash between the
        // `Released` CAS above and the prune below leaves the edge at
        // `Released` on disk with the dedup entry still present — recovery
        // (re-running this same sequence) must complete the prune and delete
        // without re-invoking `release_fn` (it only retries release from
        // `Claimed`, never from `Released`). `None` in production.
        if let Some(hook) = crash_hook_before_prune {
            hook();
        }
        // §5.5 round-7: prune the tree's `released_children` dedup entry for
        // this child strictly before the delete, never after — a crash here
        // just re-derives the same (idempotent) prune on the next recovery
        // pass, whereas pruning after delete would leave no edge on disk to
        // re-drive the prune from if this step were skipped.
        prune_fn().await?;
        self.delete_if_version(
            scope,
            parent_run_id,
            child_run_id,
            released_version,
            crash_hook_before_delete,
        )
        .await?;
        self.prune_roster_if_parent_empty(scope, parent_run_id)
            .await?;
        Ok(())
    }

    /// Full close for an edge already past `Released` on disk (recovery's
    /// terminal-but-undeleted sweep, §2) — no release call to make.
    pub async fn close(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        let Some((_, version)) = self.get_edge(scope, parent_run_id, child_run_id).await? else {
            return Ok(());
        };
        self.delete_if_version(scope, parent_run_id, child_run_id, version, None)
            .await?;
        self.prune_roster_if_parent_empty(scope, parent_run_id)
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<F> ironclaw_loop_host::AwaitEdgeWriter for FilesystemAwaitEdgeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    // `check_scope_recovered` uses the trait's default (always admits) —
    // `boot_recovery::ScopeRecoveryDriver` wraps this store and overrides it
    // with the real lazy-admission check; production wires the driver, not
    // this store directly, into `SubagentSpawnDeps.await_edge_writer`.

    async fn record_awaited_child(
        &self,
        record: ironclaw_loop_host::AwaitedChildSetRecord,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        let parent_run_id = record.parent_run_context.run_id;
        let child_run_id = record.child_run_id;
        let child_scope = record.child_scope.clone();
        let edge = AwaitEdge {
            child_scope: record.child_scope,
            child_thread_id: record.child_thread_id,
            parent_thread_id: record.parent_run_context.thread_id.clone(),
            parent_run_context: record.parent_run_context,
            tree_root_run_id: record.tree_root_run_id,
            gate_ref: record.gate_ref,
            source_binding_ref: record.source_binding_ref,
            reply_target_binding_ref: record.reply_target_binding_ref,
            subagent_kind: record.subagent_kind,
            spawn_capability_id: record.spawn_capability_id,
            result_ref: record.result_ref,
            mode: record.mode,
            state: AwaitEdgeState::Open,
            terminal_kind: None,
            terminal_byte_len: None,
            terminal_reason: None,
            reservation_release: ReservationReleaseState::Unclaimed,
            created_at: Utc::now(),
            settled_at: None,
        };
        self.open(&child_scope, parent_run_id, child_run_id, edge)
            .await
            .map_err(super::map_await_edge_error)
    }

    async fn abandon_awaited_child(
        &self,
        child_scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        self.abandon(child_scope, parent_run_id, child_run_id)
            .await
            .map_err(super::map_await_edge_error)
    }
}

/// Loop-exit's blocked-gate evidence check (`crate::loop_exit_applier`):
/// "does this gate ref identify an awaited child recorded for this parent
/// run?" — verifies a `BlockedDependentRun` checkpoint's gate binding isn't
/// spoofed, independent of terminal/delivery state. Reuses [`Self::list_group`]'s
/// same list+filter-by-`gate_ref` logic under the parent's edge directory.
#[async_trait::async_trait]
impl<F> crate::loop_exit_applier::AwaitDependentRunEvidenceStore for FilesystemAwaitEdgeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    async fn has_awaited_child_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &ironclaw_turns::LoopGateRef,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        let gate_ref = ironclaw_turns::GateRef::new(gate_ref.as_str()).map_err(|reason| {
            ironclaw_turns::TurnError::InvalidRequest {
                reason: format!("awaited child gate evidence has invalid gate ref: {reason}"),
            }
        })?;
        let group = self
            .list_group(scope, run_id, &gate_ref)
            .await
            .map_err(|error| ironclaw_turns::TurnError::Unavailable {
                reason: error.to_string(),
            })?;
        // Only blocking-mode edges count as blocked-exit evidence — a
        // background-mode child's gate must not be trusted to unblock a
        // `BlockedDependentRun` checkpoint (background mode doesn't exist
        // yet in production; this check preserves the pre-existing
        // fail-closed behavior for when it lands).
        Ok(group
            .iter()
            .any(|(_, edge)| edge.mode == ironclaw_loop_host::SpawnSubagentMode::Blocking))
    }
}

fn backend_error(error: FilesystemError) -> AwaitEdgeStoreError {
    AwaitEdgeStoreError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        AgentId, CapabilityId, MountAlias, MountGrant, MountPermissions, MountView, TenantId,
        ThreadId, VirtualPath,
    };
    use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
    use ironclaw_turns::{GateRef, LoopResultRef, ReplyTargetBindingRef, SourceBindingRef};

    use super::*;

    fn scoped_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    fn turn_scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant").unwrap(),
            Some(AgentId::new("agent").unwrap()),
            None,
            ThreadId::new("parent-thread").unwrap(),
        )
    }

    fn test_edge(gate_ref: &str) -> AwaitEdge {
        AwaitEdge {
            child_scope: turn_scope(),
            child_thread_id: ThreadId::new("child-thread").unwrap(),
            parent_thread_id: ThreadId::new("parent-thread").unwrap(),
            parent_run_context: ironclaw_agent_loop::test_support::test_run_context(
                "await-edge-store-test",
            ),
            tree_root_run_id: TurnRunId::new(),
            gate_ref: GateRef::new(gate_ref).unwrap(),
            source_binding_ref: SourceBindingRef::new("subagent-source:test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:test").unwrap(),
            subagent_kind: SubagentKindId::new("general").unwrap(),
            spawn_capability_id: CapabilityId::new("builtin.spawn_subagent").unwrap(),
            result_ref: LoopResultRef::new("result:subagent.test").unwrap(),
            mode: SpawnSubagentMode::Blocking,
            state: AwaitEdgeState::Open,
            terminal_kind: None,
            terminal_byte_len: None,
            terminal_reason: None,
            reservation_release: ReservationReleaseState::Unclaimed,
            created_at: Utc::now(),
            settled_at: None,
        }
    }

    #[tokio::test]
    async fn open_then_settle_then_close_deletes_edge() {
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();

        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        assert!(store.peek(&scope, parent, child).await.unwrap().is_some());

        let settled = store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                Some(42),
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settled.state, AwaitEdgeState::Settled);
        assert_eq!(settled.terminal_byte_len, Some(42));

        store
            .close_with_release(
                &scope,
                parent,
                child,
                || async { Ok(()) },
                || async { Ok(()) },
                CloseCrashHooks::default(),
            )
            .await
            .unwrap();

        assert!(store.peek(&scope, parent, child).await.unwrap().is_none());
    }

    // Required test (§4.0 round-5 self-heal): `open()` must re-touch
    // (version-bump) the roster marker again after the edge write succeeds,
    // not just before it. A fresh scope's marker is created at version 1 by
    // the pre-edge touch; without the post-edge touch it would stay there.
    #[tokio::test]
    async fn open_bumps_roster_marker_version_again_after_edge_write() {
        let fs = scoped_fs();
        let store = FilesystemAwaitEdgeStore::new(Arc::clone(&fs));
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();

        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();

        let roster_key = RosterKey::from_resource_scope(&scope.to_resource_scope());
        let roster_path = roster::roster_path(&roster_key).unwrap();
        let version_after_open = fs
            .get(&ResourceScope::system(), &roster_path)
            .await
            .unwrap()
            .unwrap()
            .version;

        assert!(
            version_after_open.get() > 1,
            "a fresh scope's roster marker would be stuck at version 1 (just the \
             pre-edge create) if the post-edge self-heal touch were missing; got {version_after_open}"
        );
    }

    // Required tests (§4.5 round-7 roster liveness): the close path's
    // opportunistic roster prune. `prune_roster_if_parent_empty` must prune
    // the marker once a scope's last edge is gone, and must leave it in
    // place while a sibling edge under the same parent is still open.
    #[tokio::test]
    async fn close_with_release_prunes_roster_marker_once_scopes_last_edge_is_gone() {
        let fs = scoped_fs();
        let store = FilesystemAwaitEdgeStore::new(Arc::clone(&fs));
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();

        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        let roster_key = RosterKey::from_resource_scope(&scope.to_resource_scope());
        let roster_path = roster::roster_path(&roster_key).unwrap();
        assert!(
            fs.get(&ResourceScope::system(), &roster_path)
                .await
                .unwrap()
                .is_some(),
            "roster marker exists after open()"
        );

        store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .close_with_release(
                &scope,
                parent,
                child,
                || async { Ok(()) },
                || async { Ok(()) },
                CloseCrashHooks::default(),
            )
            .await
            .unwrap();

        assert!(
            fs.get(&ResourceScope::system(), &roster_path)
                .await
                .unwrap()
                .is_none(),
            "closing a scope's last edge must opportunistically prune its now-stale roster marker"
        );
    }

    #[tokio::test]
    async fn close_with_release_leaves_roster_marker_when_sibling_edge_still_open() {
        let fs = scoped_fs();
        let store = FilesystemAwaitEdgeStore::new(Arc::clone(&fs));
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();

        store
            .open(&scope, parent, child_a, test_edge("gate:a"))
            .await
            .unwrap();
        store
            .open(&scope, parent, child_b, test_edge("gate:b"))
            .await
            .unwrap();
        let roster_key = RosterKey::from_resource_scope(&scope.to_resource_scope());
        let roster_path = roster::roster_path(&roster_key).unwrap();

        store
            .settle(
                &scope,
                parent,
                child_a,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();
        store
            .close_with_release(
                &scope,
                parent,
                child_a,
                || async { Ok(()) },
                || async { Ok(()) },
                CloseCrashHooks::default(),
            )
            .await
            .unwrap();

        assert!(
            fs.get(&ResourceScope::system(), &roster_path)
                .await
                .unwrap()
                .is_some(),
            "sibling edge child_b is still open under this parent -- the roster marker must survive"
        );
        assert!(store.peek(&scope, parent, child_b).await.unwrap().is_some());
    }

    // Required test (§5.5 round-7 scenario (a)): a crash between the
    // `Released` CAS and the prune leaves the edge Released with the dedup
    // entry still present -- recovery must complete the prune and delete
    // without re-invoking `release_fn` (mirrors the scenario-(b) test below).
    #[tokio::test]
    async fn crash_before_prune_then_recovery_completes_prune_and_delete_without_double_release() {
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();

        let release_calls = std::sync::atomic::AtomicUsize::new(0);
        let prune_calls = std::sync::atomic::AtomicUsize::new(0);
        let panicked = std::thread::scope(|scope_thread| {
            scope_thread
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build single-threaded runtime for crash-injection thread");
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        rt.block_on(store.close_with_release(
                            &scope,
                            parent,
                            child,
                            || {
                                release_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                async { Ok(()) }
                            },
                            || {
                                prune_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                async { Ok(()) }
                            },
                            CloseCrashHooks {
                                before_prune: Some(&|| {
                                    panic!("simulated crash between Released CAS and prune")
                                }),
                                before_delete: None,
                            },
                        ))
                    }))
                })
                .join()
                .expect("crash-injection thread itself must not panic")
        });
        assert!(
            panicked.is_err(),
            "expected the injected crash hook to unwind"
        );
        assert_eq!(release_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            prune_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "prune_fn must not have run yet -- the crash lands strictly before it"
        );

        // The edge is still on disk at Released (crash landed before prune).
        let surviving = store.peek(&scope, parent, child).await.unwrap().unwrap();
        assert_eq!(
            surviving.reservation_release,
            ReservationReleaseState::Released
        );

        // Recovery re-runs close_with_release: observes Released, does NOT
        // call release_fn again, and completes the prune + delete.
        store
            .close_with_release(
                &scope,
                parent,
                child,
                || {
                    release_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Ok(()) }
                },
                || {
                    prune_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Ok(()) }
                },
                CloseCrashHooks::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            release_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "release_fn must not be re-invoked once the edge is already Released"
        );
        assert_eq!(
            prune_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "recovery's pass must run prune_fn exactly once"
        );
        assert!(store.peek(&scope, parent, child).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_token_is_last_cas_version_not_earlier_one() {
        // Pins §2 round-4: using the terminal-state CAS's version for the
        // delete would fail VersionMismatch every time, since the
        // Claimed->Released transition bumps the version again afterward.
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        let (_, version_at_open) = store
            .get_edge(&scope, parent, child)
            .await
            .unwrap()
            .unwrap();
        store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();
        let claimed_version = store
            .claim_release(&scope, parent, child)
            .await
            .unwrap()
            .unwrap();
        let released_version = store
            .mark_released(&scope, parent, child, claimed_version)
            .await
            .unwrap();
        assert_ne!(released_version, version_at_open);

        // Using the stale (open-time) version fails.
        assert!(matches!(
            store
                .delete_if_version(&scope, parent, child, version_at_open, None)
                .await,
            Ok(()) // benign VersionMismatch collapses to Ok per §2 — assert the edge SURVIVES instead
        ));
        assert!(store.peek(&scope, parent, child).await.unwrap().is_some());

        // The Released CAS's own version succeeds.
        store
            .delete_if_version(&scope, parent, child, released_version, None)
            .await
            .unwrap();
        assert!(store.peek(&scope, parent, child).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reservation_release_tri_state_transitions_and_retry_unlock() {
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();

        let _claimed_version = store
            .claim_release(&scope, parent, child)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .peek(&scope, parent, child)
                .await
                .unwrap()
                .unwrap()
                .reservation_release,
            ReservationReleaseState::Claimed
        );
        // A second claim attempt loses the race.
        assert!(
            store
                .claim_release(&scope, parent, child)
                .await
                .unwrap()
                .is_none()
        );

        // Failure path: retry-unlock back to Unclaimed.
        store.unclaim_release(&scope, parent, child).await.unwrap();
        assert_eq!(
            store
                .peek(&scope, parent, child)
                .await
                .unwrap()
                .unwrap()
                .reservation_release,
            ReservationReleaseState::Unclaimed
        );

        // Now succeeds end to end.
        let claimed_version = store
            .claim_release(&scope, parent, child)
            .await
            .unwrap()
            .unwrap();
        store
            .mark_released(&scope, parent, child, claimed_version)
            .await
            .unwrap();
        assert_eq!(
            store
                .peek(&scope, parent, child)
                .await
                .unwrap()
                .unwrap()
                .reservation_release,
            ReservationReleaseState::Released
        );
    }

    // Required test (§5.5 round-7, promoted from optional to required per
    // plan review): crash injected between the released_children prune and
    // delete_if_version — recovery (re-running close_with_release against an
    // edge already at Released) completes the delete without re-invoking
    // the release call.
    #[tokio::test]
    async fn mid_prune_crash_then_recovery_completes_delete_without_double_release() {
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();

        // Simulate the crash with a real unwind (not just an injected error),
        // so the test genuinely pins "execution stops between prune and
        // delete" rather than "the call returns Err". `#[tokio::test]`
        // already drives a runtime on this thread, so the panicking call
        // runs on a fresh OS thread with its own current-thread runtime —
        // `std::thread::scope` lets that thread borrow `store`/`scope` for
        // the duration of the call.
        let release_calls = std::sync::atomic::AtomicUsize::new(0);
        let prune_calls = std::sync::atomic::AtomicUsize::new(0);
        let panicked = std::thread::scope(|scope_thread| {
            scope_thread
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build single-threaded runtime for crash-injection thread");
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        rt.block_on(store.close_with_release(
                            &scope,
                            parent,
                            child,
                            || {
                                release_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                async { Ok(()) }
                            },
                            || {
                                prune_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                async { Ok(()) }
                            },
                            CloseCrashHooks {
                                before_prune: None,
                                before_delete: Some(&|| {
                                    panic!("simulated crash between prune and delete")
                                }),
                            },
                        ))
                    }))
                })
                .join()
                .expect("crash-injection thread itself must not panic")
        });
        assert!(
            panicked.is_err(),
            "expected the injected crash hook to unwind"
        );
        assert_eq!(release_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The crash hook fires inside `delete_if_version`, strictly after
        // `prune_fn` already ran — pins §5.5 round-7's fixed ordering
        // (release -> prune -> delete), not just "release happened once".
        assert_eq!(
            prune_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "prune_fn must run before the crash point, i.e. before delete_if_version"
        );

        // The edge is still on disk at Released (crash landed before the
        // delete committed).
        let surviving = store.peek(&scope, parent, child).await.unwrap().unwrap();
        assert_eq!(
            surviving.reservation_release,
            ReservationReleaseState::Released
        );

        // Recovery re-runs close_with_release: observes Released, does NOT
        // call release_fn again, and completes the delete.
        store
            .close_with_release(
                &scope,
                parent,
                child,
                || {
                    release_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Ok(()) }
                },
                || {
                    prune_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Ok(()) }
                },
                CloseCrashHooks::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            release_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "release_fn must not be re-invoked once the edge is already Released"
        );
        // Recovery's second pass prunes again (idempotent no-op on disk —
        // the entry was already removed) rather than skipping the step, so
        // an edge that crashes between prune and delete on *every* attempt
        // still eventually gets pruned.
        assert_eq!(
            prune_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "prune_fn runs again on recovery's second pass, even though it's a no-op on disk"
        );
        assert!(store.peek(&scope, parent, child).await.unwrap().is_none());
    }

    /// §5.4's unified `VersionMismatch` disposition: a concurrent racer that
    /// finishes the `Claimed -> Released` CAS before this call reaches its
    /// own `mark_released` must not fail the whole close — it should adopt
    /// the racer's `Released` version as the delete token instead. Modeled
    /// without real thread concurrency: `release_fn` itself performs the
    /// "other racer's" full `mark_released` as a side effect using the same
    /// `claimed_version` this call will independently re-derive, so by the
    /// time `close_with_release`'s own `mark_released` call runs, the
    /// version has already moved — deterministically reproducing the race
    /// `close_with_release` must tolerate.
    #[tokio::test]
    async fn concurrent_mark_released_race_is_benign_not_a_close_failure() {
        let store = Arc::new(FilesystemAwaitEdgeStore::new(scoped_fs()));
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child = TurnRunId::new();
        store
            .open(&scope, parent, child, test_edge("gate:solo"))
            .await
            .unwrap();
        store
            .settle(
                &scope,
                parent,
                child,
                EdgeTerminalKind::Completed,
                None,
                None,
            )
            .await
            .unwrap();
        let claimed_version = store
            .claim_release(&scope, parent, child)
            .await
            .unwrap()
            .expect("edge must still be Unclaimed pre-claim");

        let racer_store = Arc::clone(&store);
        let racer_scope = scope.clone();
        let result = store
            .close_with_release(
                &scope,
                parent,
                child,
                move || {
                    let racer_store = Arc::clone(&racer_store);
                    let racer_scope = racer_scope;
                    async move {
                        // The "other racer": wins the `Claimed -> Released`
                        // CAS using the same `claimed_version` this call
                        // will also use, before this call's own
                        // `mark_released` runs.
                        racer_store
                            .mark_released(&racer_scope, parent, child, claimed_version)
                            .await
                            .map(|_| ())
                    }
                },
                || async { Ok(()) },
                CloseCrashHooks::default(),
            )
            .await;
        assert!(
            result.is_ok(),
            "the losing racer's stale-version mark_released must be absorbed as benign, \
             not fail the close: {result:?}"
        );
        assert!(
            store.peek(&scope, parent, child).await.unwrap().is_none(),
            "the edge must still end up deleted despite the mark_released race"
        );
    }

    #[tokio::test]
    async fn list_group_filters_by_gate_ref() {
        let store = FilesystemAwaitEdgeStore::new(scoped_fs());
        let scope = turn_scope();
        let parent = TurnRunId::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let child_other = TurnRunId::new();
        store
            .open(&scope, parent, child_a, test_edge("gate:batch"))
            .await
            .unwrap();
        store
            .open(&scope, parent, child_b, test_edge("gate:batch"))
            .await
            .unwrap();
        store
            .open(&scope, parent, child_other, test_edge("gate:solo-other"))
            .await
            .unwrap();

        let group = store
            .list_group(&scope, parent, &GateRef::new("gate:batch").unwrap())
            .await
            .unwrap();
        let ids: std::collections::HashSet<_> = group.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, std::collections::HashSet::from([child_a, child_b]));
    }
}
